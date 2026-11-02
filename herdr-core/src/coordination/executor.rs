use super::{AgentRecord, Command, Mutation, SpawnRecord, resolve_actor, view};
use crate::agent_start::ShellStart;
use crate::delivery::ladder::{Delivered, InputTarget, Prepared};
use crate::delivery::typed::SendRoute;
use crate::delivery::{
    Actor,
    worker::{Authority, Client, Effect},
};
use crate::engine::{Engine, EngineError, Wait};
use crate::runtime::delivery::CoordinationContext;
use crate::session_sync::ProjectedAgent;
use hide_node_link::protocol::StartForm;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const STORE_TIMEOUT: Duration = Duration::from_secs(5);
/// Herdr runs `git worktree add` inside `worktree.create`, which on a large
/// repository under load takes longer than the 5 s every other call gets.
const WORKTREE_CREATE_TIMEOUT: Duration = Duration::from_secs(30);
/// The refusal for a `worktree.create` whose answer never came: Herdr may
/// still finish the worktree, and asking the same intent again continues it.
pub(crate) const WORKTREE_UNCONFIRMED: &str = "worktree_unconfirmed";

/// What a failed `worktree.create` is refused as. Herdr's own refusal keeps
/// its words; a reply that never arrived leaves the worktree unknown, which
/// the same intent resolves, so it gets its own code instead of the
/// transport's sentence.
fn worktree_create_refusal(error: EngineError) -> String {
    if error.reply_unknown() {
        WORKTREE_UNCONFIRMED.to_owned()
    } else {
        error.to_string()
    }
}
// Creating something is single-flight: finite side effects serialize without
// occupying the durable store or Runtime, and a concurrent spawn is refused
// instead of accumulating blocked workers. The device checks before it only
// read, so they are not: each is bounded by `TARGET_CHECK_TIMEOUT` and runs
// beside another spawn's.
pub(crate) static SPAWN: Mutex<()> = Mutex::new(());
/// Tests share `SPAWN` through the process, so those that spawn take turns
/// when a runner puts them on threads of one process.
#[cfg(test)]
pub(crate) static SPAWN_TURN: Mutex<()> = Mutex::new(());
fn state(client: &Client) -> Result<Arc<crate::delivery::ledger::Ledger>, String> {
    client
        .runtime
        .upgrade()
        .ok_or("delivery_unavailable")?
        .lock()
        .map_err(|_| "delivery_unavailable")?
        .delivery_state()
}
/// The ledger with the registrations Hide holds asleep, read under one
/// hold of the runtime's lock so the two agree.
fn registrations(
    client: &Client,
) -> Result<(Arc<crate::delivery::ledger::Ledger>, super::Sleeping), String> {
    let runtime = client.runtime.upgrade().ok_or("delivery_unavailable")?;
    let runtime = runtime.lock().map_err(|_| "delivery_unavailable")?;
    let ledger = runtime.delivery_state()?;
    let sleeping = runtime.sleeping_registrations(&ledger.agents);
    Ok((ledger, sleeping))
}
/// `hide agent show <key>`, read only: a registration by its id (or the
/// caller's own, `here`) with its state, or a spawn by its id or the caller's
/// intent, answered by its child's registration once it has one and by the
/// spawn and its pane's state until then. Nothing is written or asked of an
/// engine; the facts are copied under the runtime's lock and joined after.
fn show(client: &Client, actor: &Actor, key: &str) -> Result<Value, String> {
    let runtime = client.runtime.upgrade().ok_or("delivery_unavailable")?;
    let runtime = runtime.lock().map_err(|_| "delivery_unavailable")?;
    let shown = Shown::read(&runtime, actor, key)?;
    drop(runtime);
    Ok(shown.answer(&actor.device_id))
}

/// What `hide agent show` found, with the facts it copied under the
/// runtime's lock.
pub(crate) struct Shown {
    ledger: Arc<crate::delivery::ledger::Ledger>,
    sleeping: super::Sleeping,
    found: Found,
}

enum Found {
    Agent(Box<AgentRecord>, crate::runtime::agent_show::StateFacts),
    Spawn(
        Box<super::SpawnRecord>,
        crate::runtime::agent_show::StateFacts,
    ),
}

impl Shown {
    /// Finds `key` for `actor` under the runtime's lock, reading only.
    pub(crate) fn read(
        runtime: &crate::runtime::Runtime,
        actor: &Actor,
        key: &str,
    ) -> Result<Self, String> {
        use crate::runtime::agent_show::spawns_of;
        let ledger = runtime.delivery_state()?;
        let sleeping = runtime.sleeping_registrations(&ledger.agents);
        let registered = |record: &AgentRecord| {
            let facts =
                runtime.agent_state_facts(&record.actor, &spawns_of(&ledger, Some(&record.id)));
            Found::Agent(Box::new(record.clone()), facts)
        };
        let found = if key == super::HERE {
            registered(super::here(&ledger, actor)?)
        } else if let Some(record) = ledger.agents.iter().find(|record| record.id == key) {
            registered(record)
        } else {
            let caller = super::live_self(&ledger, actor)
                .next()
                .map(|record| record.id.clone());
            let spawn = ledger
                .spawns
                .iter()
                .find(|spawn| spawn.id == key)
                .or_else(|| {
                    let caller = caller.as_deref()?;
                    ledger
                        .spawns
                        .iter()
                        .find(|spawn| spawn.parent == caller && spawn.intent == key)
                })
                .ok_or("agent_unavailable")?;
            match spawn
                .child
                .as_deref()
                .and_then(|child| ledger.agents.iter().find(|record| record.id == child))
            {
                Some(record) => registered(record),
                None => Found::Spawn(Box::new(spawn.clone()), runtime.spawn_state_facts(spawn)),
            }
        };
        Ok(Self {
            ledger,
            sleeping,
            found,
        })
    }

    /// The answer, for a caller on `device`.
    pub(crate) fn answer(self, device: &str) -> Value {
        match self.found {
            Found::Agent(record, facts) => {
                let life = super::life(&record, &self.sleeping);
                json!(crate::delivery::answer::AgentShow {
                    agent: super::agent_view(&record, &self.ledger, &self.sleeping),
                    state: facts.state(Some(life), record.parent.as_deref()),
                })
            }
            Found::Spawn(spawn, facts) => {
                let parent =
                    (spawn.mode == super::SpawnMode::Delegation).then_some(spawn.parent.as_str());
                json!(crate::delivery::answer::SpawnShow {
                    spawn: spawn_ref(&spawn, device),
                    state: facts.state(None, parent),
                })
            }
        }
    }
}

/// A spawn that stopped partway with its pane made names that spawn and
/// pane in its refusal, so the caller can read the pane, answer its screen
/// and ask `hide agent show <spawn id>` without a registration
/// (`super::split_refusal` takes the detail off again).
fn incomplete_spawn(client: &Client, actor: &Actor, command: &Command, code: String) -> String {
    let Command::Spawn { parent, intent, .. } = command else {
        return code;
    };
    if !super::INCOMPLETE_SPAWN.contains(&code.as_str()) {
        return code;
    }
    let Ok(ledger) = state(client) else {
        return code;
    };
    let parent = match parent.as_deref() {
        None | Some(super::HERE) => super::live_self(&ledger, actor)
            .next()
            .map(|record| record.id.clone()),
        Some(parent) => Some(parent.to_owned()),
    };
    let found = parent.and_then(|parent| {
        ledger
            .spawns
            .iter()
            .find(|spawn| spawn.parent == parent && spawn.intent == *intent && spawn.pane.is_some())
    });
    match found {
        Some(spawn) => {
            let spawn = spawn_ref(spawn, &actor.device_id);
            super::refusal(
                &code,
                &json!({"id": spawn.id, "pane": spawn.pane}).to_string(),
            )
        }
        None => code,
    }
}

/// A spawn as an answer names it: its pane as `hide pane send` takes it from
/// a caller on `device`, bare on the caller's own machine.
pub(crate) fn spawn_ref(
    spawn: &super::SpawnRecord,
    device: &str,
) -> crate::delivery::answer::SpawnRef {
    crate::delivery::answer::SpawnRef {
        id: spawn.id.clone(),
        intent: spawn.intent.clone(),
        name: spawn.name.clone(),
        kind: spawn.kind.clone(),
        machine: spawn.machine.clone(),
        pane: spawn
            .pane
            .as_ref()
            .map(|pane| match spawn.machine.as_deref() {
                Some(machine) if machine != device => crate::address::scoped_pane_id(machine, pane),
                _ => pane.clone(),
            }),
        child: spawn.child.clone(),
    }
}

fn mutate(
    client: &Client,
    authority: &Authority,
    actor: &Actor,
    mutation: Mutation,
) -> Result<Value, String> {
    client.submit(
        Effect::Agents {
            authority: Authority {
                caller: authority.caller.clone(),
                context: authority.context.clone(),
            },
            actor: actor.clone(),
            mutation: Box::new(mutation),
        },
        STORE_TIMEOUT,
    )
}
fn context(client: &Client, device: &str) -> Result<CoordinationContext, String> {
    client
        .runtime
        .upgrade()
        .ok_or("delivery_unavailable")?
        .lock()
        .map_err(|_| "delivery_unavailable")?
        .coordination_context(device)
}
fn agents(engine: &dyn Engine) -> Result<Vec<ProjectedAgent>, String> {
    agents_with_timeout(engine, Duration::from_secs(2))
}
fn agents_with_timeout(
    engine: &dyn Engine,
    timeout: Duration,
) -> Result<Vec<ProjectedAgent>, String> {
    engine
        .agents(Wait::Each(timeout))
        .map_err(|error| match error {
            EngineError::Answer(_) | EngineError::Protocol { .. } => {
                "native_identity_unavailable".into()
            }
            error => error.to_string(),
        })
}
fn native_matches(agent: &ProjectedAgent, pane: &str, name: Option<&str>, kind: &str) -> bool {
    agent.pane_id == pane
        && name.is_none_or(|name| agent.name.as_deref() == Some(name))
        && agent.agent.as_deref().is_some_and(|observed| {
            hide_agent_adapter::canonical_kind(observed) == hide_agent_adapter::canonical_kind(kind)
        })
}
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn wait_native_identity(
    engine: &dyn Engine,
    pane: &str,
    name: Option<&str>,
    kind: &str,
) -> Result<ProjectedAgent, String> {
    // The public socket start acknowledges the typed command before the
    // process and its native session are detected. Wait for that observation,
    // never send another start, and keep every read within one finite deadline.
    // The agent runs by now, so a read Herdr did not answer says nothing of
    // it: the wait reads again, and at its deadline the spawn leaves the
    // registration to an identity wait rather than an agent nothing tracks.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("native_identity_unavailable".into());
        }
        let observed = match agents_with_timeout(engine, remaining.min(Duration::from_secs(2))) {
            Ok(observed) => observed,
            Err(detail) => {
                crate::diagnostic!(json!({
                    "component": "spawn", "kind": "identity.read_failed", "pane_id": pane,
                    "detail": detail,
                }));
                Vec::new()
            }
        };
        if let Some(agent) = observed.into_iter().find(|agent| {
            native_matches(agent, pane, name, kind)
                && agent.lineage_session.is_some()
                && agent.agent_session.is_some()
        }) {
            return Ok(agent);
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(100)),
        );
    }
}
/// Waits until Herdr reports the agent `name` in `pane` ready for input
/// (`BRIEF_READY_WAIT`), the moment the spawn's brief may be typed. An agent
/// that never takes input in that time is `brief_not_submitted`: nothing was
/// typed, and the same command waits again.
#[allow(clippy::disallowed_methods)] // a production wait on Herdr's readiness, bounded by the deadline
fn brief_ready(engine: &dyn Engine, spawn: &str, pane: &str, name: &str) -> Result<(), String> {
    let deadline = Instant::now() + super::BRIEF_READY_WAIT;
    loop {
        let agent = match engine.delivery_agent(Wait::Each(Duration::from_secs(5)), pane) {
            Err(EngineError::Request(message)) => return Err(message),
            agent => agent.map_err(|error| {
                brief_refused(spawn, pane, "brief.agent_unread", error.to_string())
            })?,
        };
        if agent.pane_id == pane && agent.name == name && agent.readiness.accepts_input() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(brief_refused(
                spawn,
                pane,
                "brief.agent_not_ready",
                format!(
                    "readiness {:?} after {:?}",
                    agent.readiness,
                    super::BRIEF_READY_WAIT
                ),
            ));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// How a machine starts `kind`'s program (`Call::AgentInstalled`): `None`
/// where it found no such program. The one ask of it, made by a device's
/// target check before anything is created there, and otherwise only for a
/// brief every other check let ride the start (`start_prompt::carried`).
fn start_form(
    node: &dyn crate::node_access::NodeLink,
    kind: &str,
) -> Result<Option<StartForm>, crate::node_access::LinkError> {
    crate::node_access::call_as::<Option<StartForm>>(
        node,
        hide_node_link::protocol::Call::AgentInstalled {
            name: hide_agent_adapter::canonical_kind(kind).to_owned(),
        },
        TARGET_CHECK_TIMEOUT,
    )
}

fn brief_refused(spawn: &str, pane: &str, kind: &str, detail: String) -> String {
    crate::diagnostic!(json!({
        "component": "spawn", "kind": kind, "spawn": spawn, "pane_id": pane, "detail": detail,
    }));
    "brief_not_submitted".to_owned()
}

/// The words a fresh start of `kind` adds to carry `brief` as its prompt
/// argument (`AgentApi::LaunchPrompt`), or `None` where the brief is typed
/// once the agent runs: an agent that does not hold a launch prompt through
/// its start screens, a start that cannot carry it (`start_prompt::carried`:
/// native arguments that give a prompt already, a brief the command line
/// refuses), a machine that did not say how the agent starts, or a start
/// through `cmd.exe`. `form` says how the pane's machine starts the agent's
/// program (`Call::AgentInstalled`) and is asked last, only for a brief every
/// other check let through. On Windows a program that is a batch file, npm's
/// `.cmd` shim of every CLI among them, runs through `cmd.exe`, which reads
/// the start line again: it refuses a line past 8,191 characters, expands
/// `%`, and a shim may read its arguments again, so such a start carries no
/// brief, which can be typed instead. Decided once, for the spawn's first
/// start, and saved with `BriefLaunched`.
fn launch_prompt(
    kind: &str,
    args: &[String],
    brief: &str,
    form: impl FnOnce() -> Result<StartForm, String>,
) -> Option<Vec<String>> {
    hide_agent_adapter::adapter(kind).filter(|row| row.launch_prompt())?;
    let refused = |detail: &str| {
        crate::diagnostic!(json!({
            "component": "spawn", "kind": "brief.launch_refused", "agent_kind": kind,
            "detail": detail,
        }));
    };
    let words = crate::start_prompt::carried(kind, args, brief)
        .map_err(|refusal| refused(refusal.detail()))
        .ok()?;
    match form() {
        Ok(StartForm::Program) => Some(words),
        Ok(StartForm::Batch) => {
            refused(
                "the agent starts through cmd.exe, which reads a prompt in its arguments again",
            );
            None
        }
        Err(detail) => {
            refused(&detail);
            None
        }
    }
}

/// The words this start of the spawn adds to carry its brief, `None` where
/// the brief is typed once the agent runs. A brief already launched carries
/// the words saved with it (`launch_words`) and asks nothing again, so a start
/// typed again for the same spawn cannot start the agent without its brief;
/// a record an earlier build saved without its words is given them as that
/// build gave them, every launched brief carried as given, with no machine
/// asked, and fails `brief_launch_unrecorded` where that start could carry
/// none. Only a fresh start whose brief was never sent decides
/// (`launch_prompt`).
fn brief_words(
    reserved: &SpawnRecord,
    start_args: &[String],
    form: impl FnOnce() -> Result<StartForm, String>,
) -> Result<Option<Vec<String>>, String> {
    let Some(brief) = reserved.brief.as_deref() else {
        return Ok(None);
    };
    let launched_before = reserved
        .brief_input
        .as_ref()
        .is_some_and(|input| input.route == SendRoute::Launch);
    if launched_before && reserved.launch_words.is_empty() {
        return crate::start_prompt::carried(&reserved.kind, start_args, brief)
            .map(Some)
            .map_err(|_| "brief_launch_unrecorded".to_owned());
    }
    if launched_before {
        return Ok(Some(reserved.launch_words.clone()));
    }
    if reserved.briefed {
        return Ok(None);
    }
    Ok(launch_prompt(&reserved.kind, start_args, brief, form))
}

/// Sends `brief` down its prepared route, the one submission path the
/// doorbell uses too. Its typing was reserved first (`Mutation::BriefTyping`),
/// and the route it answers is saved after (`Mutation::Briefed`); the
/// agent's own record confirms it (`delivery::typed`). A send whose answer
/// was lost answers `SendRoute::Lost`, so the brief counts as sent and no
/// retry of the spawn sends it again (engineering principle 11). A send
/// that sent nothing is `brief_not_submitted`, and the typing it reserved
/// stays spent.
pub(crate) fn send_brief(
    engine: &dyn Engine,
    (target, prepared): (&InputTarget, Prepared),
    spawn: &str,
    brief: &str,
) -> Result<SendRoute, String> {
    let route = match prepared.deliver(engine, target, brief) {
        Delivered::Sent(route) => route,
        Delivered::Lost(code) => {
            crate::diagnostic!(json!({
                "component": "spawn", "kind": "brief.send_lost", "spawn": spawn,
                "pane_id": target.raw_pane_id, "code": code,
            }));
            return Ok(SendRoute::Lost);
        }
        Delivered::NotYet(code) => {
            return Err(brief_refused(
                spawn,
                &target.raw_pane_id,
                "brief.prompt_refused",
                code.to_owned(),
            ));
        }
    };
    crate::diagnostic!(json!({
        "component": "spawn", "kind": "brief.submitted", "spawn": spawn,
        "pane_id": target.raw_pane_id, "bytes": brief.len(), "route": route,
    }));
    Ok(route)
}

/// The actor of `agent` on `device`; `on_node` when that device is the core's
/// own node, whose panes the core names without a device prefix.
fn actor_for(agent: &ProjectedAgent, device: &str, on_node: bool) -> Result<Actor, String> {
    let actor = Actor {
        pane_id: if on_node {
            agent.pane_id.clone()
        } else {
            crate::address::scoped_pane_id(device, &agent.pane_id)
        },
        // Herdr's own identifier for the agent: its name, else its pane. It
        // keys the pane, never labels it (`AgentRecord::wire_name`).
        name: agent.name.clone().unwrap_or_else(|| agent.pane_id.clone()),
        kind: agent.agent.clone().ok_or("native_identity_unavailable")?,
        device_id: device.into(),
        session: agent.lineage_session.clone(),
    };
    actor.require_native_identity()?;
    Ok(actor)
}
pub(super) struct HostIdentity<'a> {
    pub(super) machine: &'a str,
    pub(super) on_node: bool,
    pub(super) scope: &'a str,
    pub(super) native_machine: &'a str,
}
pub(super) fn record(
    agent: &ProjectedAgent,
    host: HostIdentity<'_>,
    instance: String,
    name: Option<String>,
    parent: Option<String>,
    project: Option<String>,
) -> Result<AgentRecord, String> {
    let session = agent
        .agent_session
        .as_ref()
        .ok_or("native_identity_unavailable")?
        .value
        .clone();
    super::kept_session(&session)?;
    Ok(AgentRecord {
        id: String::new(),
        // A pane id is never a name: Herdr's fallback identifier for an
        // agent it never named, or a name restating the pane, is none.
        name: name.filter(|name| *name != agent.pane_id),
        machine: host.machine.into(),
        host_scope: host.scope.into(),
        native_machine: host.native_machine.into(),
        session,
        instance,
        pane: agent.pane_id.clone(),
        parent,
        origin: None,
        project,
        actor: actor_for(agent, host.machine, host.on_node)?,
        ended: false,
        terminal_id: Some(agent.terminal_id.clone()),
        forked: false,
    })
}

pub(crate) fn run(
    client: Client,
    authority: Authority,
    actor: Actor,
    command: Command,
) -> Result<Value, String> {
    match command {
        Command::List => {
            let (ledger, sleeping) = registrations(&client)?;
            Ok(json!(crate::delivery::answer::AgentList {
                items: ledger
                    .agents
                    .iter()
                    .map(|record| super::agent_view(record, &ledger, &sleeping))
                    .collect(),
            }))
        }
        Command::Show { id } => show(&client, &actor, &id),
        Command::End {
            id,
            actor: asserted,
        } => mutate(
            &client,
            &authority,
            &actor,
            Mutation::End {
                id,
                actor: asserted,
            },
        ),
        Command::Register {
            check,
            machine,
            host_scope,
            session,
            instance,
            name,
            pane,
            parent,
            project,
        } => {
            if ![&host_scope, &instance, &name, &pane]
                .into_iter()
                .all(|s| super::key(s))
            {
                return Err("invalid_registration".into());
            }
            super::kept_session(&session)?;
            let machine = registration_machine(machine, &actor.device_id)?;
            let CoordinationContext {
                engine,
                host_scope: actual_scope,
                machine: native_machine,
                on_node,
                ..
            } = context(&client, &machine)?;
            if on_node && host_scope != actual_scope {
                return Err("host_scope_conflict".into());
            }
            let observed = agents(engine.as_ref())?;
            let native = observed
                .iter()
                .find(|agent| agent.pane_id == pane)
                .ok_or("target_unavailable")?;
            if native.lineage_session != crate::engine::tokens::session_digest(&session) {
                return Err("session_identity_conflict".into());
            }
            let ledger = state(&client)?;
            let parent = parent
                .map(|id| {
                    if id == super::HERE {
                        super::live_self(&ledger, &actor)
                            .next()
                            .map(|p| p.id.clone())
                            .ok_or("parent_unavailable".to_owned())
                    } else {
                        Ok(id)
                    }
                })
                .transpose()?;
            let record = record(
                native,
                HostIdentity {
                    machine: &machine,
                    scope: &host_scope,
                    native_machine: &native_machine,
                    on_node,
                },
                instance,
                Some(name),
                parent,
                project,
            )?;
            let value = mutate(
                &client,
                &authority,
                &actor,
                Mutation::Register { record, check },
            )?;
            if !check {
                publish_tokens(
                    &client,
                    engine.as_ref(),
                    value["id"].as_str().ok_or("agent_unavailable")?,
                )?;
            }
            Ok(value)
        }
        command @ Command::Spawn { .. } => spawn(&client, &authority, &actor, command.clone())
            .map_err(|code| incomplete_spawn(&client, &actor, &command, code)),
    }
}
/// A registration is always on the caller's own machine, which its pane
/// capability already names; a machine the caller restates must agree with it.
fn registration_machine(requested: Option<String>, caller: &str) -> Result<String, String> {
    match requested {
        None => Ok(caller.to_owned()),
        Some(machine) if !super::key(&machine) => Err("invalid_registration".into()),
        Some(machine) if machine != caller => Err("machine_identity_conflict".into()),
        Some(machine) => Ok(machine),
    }
}
pub(super) fn publish_tokens(client: &Client, engine: &dyn Engine, id: &str) -> Result<(), String> {
    let ledger = state(client)?;
    let child = ledger
        .agents
        .iter()
        .find(|record| record.id == id)
        .ok_or("agent_unavailable")?;
    if let Some(related) = super::lineage::related(child, &ledger) {
        super::lineage::write_record(engine, child, related)?;
    }
    Ok(())
}
/// Registers a code-owned identity, a Factory's or Hide's own, as the
/// parent or origin of the agents it starts (D-14); converges on the
/// existing record.
fn register_code_owned(
    client: &Client,
    authority: &Authority,
    actor: &Actor,
) -> Result<String, String> {
    if !actor.code_owned() {
        return Err("reserved_name".into());
    }
    let record = AgentRecord {
        id: String::new(),
        name: Some(actor.name.clone()),
        machine: actor.device_id.clone(),
        host_scope: actor.kind.clone(),
        native_machine: actor.kind.clone(),
        session: actor.name.clone(),
        instance: actor.pane_id.clone(),
        pane: actor.pane_id.clone(),
        parent: None,
        origin: None,
        project: None,
        actor: actor.clone(),
        ended: false,
        terminal_id: None,
        forked: false,
    };
    let registered = mutate(
        client,
        authority,
        actor,
        Mutation::Register {
            record,
            check: false,
        },
    )?;
    registered["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "parent_unavailable".into())
}

/// How long the device's node may take to answer one of the spawn's checks.
const TARGET_CHECK_TIMEOUT: Duration = Duration::from_secs(8);

/// Judges a device that is not the caller's before anything is made there:
/// the id names a device, the device takes work now, `repo` (when this
/// attempt still has to make the checkout) is a repository there, and the
/// agent CLI is installed there, answering how it starts when that was
/// asked. The node link is taken under the runtime lock and called after it
/// is released.
fn check_target(
    client: &Client,
    actor: &Actor,
    device: &str,
    kind: &str,
    intent: &str,
    repository: Option<&str>,
    agent: bool,
) -> Result<Option<StartForm>, String> {
    use crate::node_access::{LinkError, call_as};
    use hide_node_link::{cleanup::RepositoryDirs, protocol::Call};
    let link = client
        .runtime
        .upgrade()
        .ok_or("delivery_unavailable")?
        .lock()
        .map_err(|_| "delivery_unavailable")?
        .spawn_target(&actor.device_id, device)?;
    // A node that did not answer says nothing about the repository or the
    // CLI, so it is the device that is unavailable; only its own answer can
    // refuse them.
    let unavailable = |check: &str, error: LinkError| {
        // The caller is told what to do next; what the node said stays here.
        crate::diagnostic!(json!({
            "component": "spawn", "kind": "device_check_failed", "device": device,
            "intent": intent, "check": check, "cause": error.to_string()
        }));
        match error {
            LinkError::Refused(_) => None,
            _ => Some("machine_unavailable".to_owned()),
        }
    };
    if let Some(repository) = repository {
        match call_as::<Option<RepositoryDirs>>(
            link.as_ref(),
            Call::Repository {
                path: repository.to_owned(),
            },
            TARGET_CHECK_TIMEOUT,
        ) {
            Ok(Some(_)) => {}
            Ok(None) => return Err("repository_unavailable".into()),
            Err(error) => {
                return Err(
                    unavailable("repository", error).unwrap_or("repository_unavailable".into())
                );
            }
        }
    }
    if !agent {
        return Ok(None);
    }
    match start_form(link.as_ref(), kind) {
        Ok(Some(form)) => Ok(Some(form)),
        Ok(None) => Err("agent_not_installed".into()),
        Err(error) => {
            Err(unavailable("agent_installed", error).unwrap_or("agent_not_installed".into()))
        }
    }
}

fn spawn(
    client: &Client,
    authority: &Authority,
    actor: &Actor,
    mut command: Command,
) -> Result<Value, String> {
    // The caller's own device is the absence of a device: one intent, one
    // spelling.
    if let Command::Spawn { machine, .. } = &mut command
        && machine.as_deref() == Some(actor.device_id.as_str())
    {
        *machine = None;
    }
    let Command::Spawn {
        parent,
        machine,
        name,
        intent,
        kind,
        repo,
        branch,
        path,
        args,
        launch,
        task,
        brief,
        mission: _,
        plan_node,
        plan,
        beside,
    } = &command
    else {
        return Err("invalid_spawn".into());
    };
    // A plan step's spawn runs only as the core admitted it.
    if plan_node.is_some() != plan.is_some() {
        return Err("invalid_spawn".into());
    }
    if brief
        .as_ref()
        .is_some_and(|brief| brief.trim().is_empty() || brief.len() > crate::delivery::BODY_LIMIT)
    {
        return Err("invalid_brief".into());
    }
    if parent.as_ref().is_some_and(|parent| !super::key(parent))
        || machine.as_ref().is_some_and(|machine| !super::key(machine))
        || ![name, intent, kind, repo, branch]
            .into_iter()
            .all(|s| super::key(s))
        || args.len() > 128
        || args.iter().map(String::len).sum::<usize>() > 8192
        || args.iter().any(|arg| arg.contains('\0'))
        || task.as_ref().is_some_and(|task| {
            task.kind.home().is_none() || task.target.as_deref().is_some_and(|t| !super::key(t))
        })
    {
        return Err("invalid_spawn".into());
    }
    if !crate::fork::valid_agent_name(name) {
        return Err("invalid_agent_name".into());
    }
    // The choice goes before the native arguments, which may end with the
    // first prompt after `--`; the machine's permission choice goes before
    // both once the child's machine is known.
    let mut start_args =
        crate::runtime::agent_choice::choice_arguments(kind, launch).map_err(|detail| {
            crate::diagnostic!(json!({
                "component": "spawn", "kind": "spawn.invalid_launch_choice", "intent": intent,
                "agent_kind": kind, "detail": detail,
            }));
            "invalid_launch_choice"
        })?;
    start_args.extend(args.iter().cloned());
    // The device probe takes the kind as a program name, so only a kind Hide
    // can start reaches it.
    if machine.is_some() && hide_agent_adapter::start_kind(kind).is_none() {
        return Err("invalid_spawn".into());
    }
    let mut ledger = state(client)?;
    // A spawn beside its caller opens in the caller's own tab, so it is
    // judged before anything is registered, reserved or created; a retry of
    // an intent whose pane exists already is judged by what it did, and a
    // completed one stays offline (engineering #11).
    let beside_of = if *beside {
        let known_parent = if parent.as_deref().is_none_or(|parent| parent == super::HERE) {
            super::live_self(&ledger, actor)
                .next()
                .map(|p| p.id.clone())
        } else {
            parent.clone()
        };
        let placed = known_parent.is_some_and(|id| {
            ledger.spawns.iter().any(|record| {
                record.parent == id
                    && record.intent == *intent
                    && record.beside
                    && (record.pane.is_some() || record.completed)
            })
        });
        Some(beside_target(
            client,
            authority,
            actor,
            machine.as_deref(),
            (!placed).then_some((repo.as_str(), branch.as_str(), path.as_deref())),
            intent,
        )?)
    } else {
        None
    };
    // How the device's machine starts the agent, when its check asked; a
    // brief that may ride the start reads it rather than asking again.
    let mut checked_form = None;
    if let Some(device) = machine {
        // Nothing is created for a device that cannot take the spawn, so the
        // judgment comes before the parent is registered or the intent
        // reserved, and a retry of an intent that already progressed or
        // completed is judged by what it already did.
        // Another agent's id is judged before any device is probed, so a
        // device's answer never reaches a caller with no authority over it.
        if let Some(explicit) = parent.as_deref().filter(|parent| *parent != super::HERE)
            && !resolve_actor(&ledger, explicit).is_some_and(|parent| parent.same_identity(actor))
        {
            return Err("parent_authority_required".into());
        }
        let known_parent = if parent.as_deref().is_none_or(|parent| parent == super::HERE) {
            super::live_self(&ledger, actor)
                .next()
                .map(|p| p.id.clone())
        } else {
            parent.clone()
        };
        let earlier = known_parent.and_then(|id| {
            ledger
                .spawns
                .iter()
                .find(|record| record.parent == id && record.intent == *intent)
        });
        if earlier.is_some_and(|record| record.machine != *machine) {
            return Err("intent_conflict".into());
        }
        let started = earlier.is_some_and(|record| record.pane.is_some());
        if !earlier.is_some_and(|record| record.completed) {
            checked_form = check_target(
                client,
                actor,
                device,
                kind,
                intent,
                // A retry whose pane exists needs the device only to be
                // reachable: the repository and the agent were already used.
                (!started).then_some(repo.as_str()),
                !started,
            )?;
        }
    }
    // The device checks above only read, so a slow device never holds the
    // process-wide lock that local and Factory spawns need; everything that
    // creates something runs under it.
    let single = SPAWN.try_lock().map_err(|_| "spawn_busy")?;
    let parent_id = if parent.as_deref().is_none_or(|parent| parent == super::HERE) {
        if let Some(parent) = super::live_self(&ledger, actor).next() {
            parent.id.clone()
        } else if actor.hide_owned() {
            // Hide's own work (Desk, Hide AI) has no pane for Herdr to
            // report: its record is the origin it registers here, once.
            register_code_owned(client, authority, actor)?
        } else {
            let CoordinationContext {
                engine,
                host_scope,
                machine: native_machine,
                on_node,
                ..
            } = context(client, &actor.device_id)?;
            let observed = agents(engine.as_ref())?;
            let native = observed
                .iter()
                .find(|agent| {
                    actor_for(agent, &actor.device_id, on_node)
                        .is_ok_and(|current| current.same_identity(actor))
                })
                .ok_or("parent_unavailable")?;
            let record = record(
                native,
                HostIdentity {
                    machine: &actor.device_id,
                    scope: &host_scope,
                    native_machine: &native_machine,
                    on_node,
                },
                native.pane_id.clone(),
                // Herdr's name for the parent while it has one; a parent
                // Herdr never named stays unnamed instead of taking its pane
                // id for a name.
                native.name.clone(),
                None,
                // A device's path names no project of the caller's machine.
                machine.is_none().then(|| repo.clone()),
            )?;
            mutate(
                client,
                authority,
                actor,
                Mutation::Register {
                    record,
                    check: false,
                },
            )?["id"]
                .as_str()
                .ok_or("parent_unavailable")?
                .to_owned()
        }
    } else {
        parent.clone().ok_or("parent_unavailable")?
    };
    ledger = state(client)?;
    if !resolve_actor(&ledger, &parent_id).is_some_and(|parent| parent.same_identity(actor)) {
        return Err("parent_authority_required".into());
    }
    let reserved = mutate(
        client,
        authority,
        actor,
        Mutation::Reserve {
            parent: parent_id.clone(),
            command: command.clone(),
        },
    )?;
    let mut reserved: SpawnRecord =
        serde_json::from_value(reserved).map_err(|_| "spawn_unavailable")?;
    if reserved.completed {
        let (ledger, sleeping) = registrations(client)?;
        return Ok(view(
            ledger
                .agents
                .iter()
                .find(|record| Some(&record.id) == reserved.child.as_ref())
                .ok_or("child_unavailable")?,
            &ledger,
            &sleeping,
        ));
    }
    // Everything the child needs happens on its own device; only the parent's
    // authority and watch stay with the caller.
    let child_device = reserved
        .machine
        .clone()
        .unwrap_or_else(|| actor.device_id.clone());
    let CoordinationContext {
        engine,
        host_scope,
        machine: native_machine,
        codex: codex_daemon,
        permissions,
        on_node,
        node,
    } = context(client, &child_device).map_err(|reason| {
        if reserved.machine.is_some() {
            "machine_unavailable".to_owned()
        } else {
            reason
        }
    })?;
    // A device that stops answering partway leaves the spawn reserved and
    // resumable: the caller is told the device is unavailable, and the same
    // command continues once it answers again.
    let (spawn_id, on_device) = (reserved.id.clone(), reserved.machine.is_some());
    let placed = (|| -> Result<Value, String> {
        if reserved.pane.is_none() {
            // An engine that names a pane before it runs anything there (the
            // core's own) has this record name it first, so the pane is
            // never seen running unregistered, and a pane whose holder does
            // not start is taken off the record in the same call (PRD
            // engine-bundle-3-core-owns D-11). Herdr names a pane only once
            // it runs, so its pane is recorded below, after the call.
            let engine = engine.gated(crate::engine::Gate::placing(SpawnPlacement {
                client,
                authority,
                actor,
                spawn: reserved.id.clone(),
            }));
            // How the pane's shell says it has finished starting, recorded
            // with the pane so every later start in it waits the same way.
            let shell;
            // Reconcile a worktree whose creation reply was interrupted. The
            // existing local/device engine and checkout owner are shared with
            // the product's worktree/start path; no spawn-specific SSH exists.
            let existing = engine
                .worktree_path(Wait::Each(Duration::from_secs(5)), repo, branch)
                .map_err(String::from)?;
            // `made`: this spawn made the worktree, now or in an attempt whose
            // reply was lost; only such a worktree is trusted for an agent
            // that asks (agy), as for a task's worktree. A worktree that was
            // already the operator's keeps whatever trust they gave it.
            let (checkout, pane, seed_tab, made) = if let Some(existing) = existing {
                if path.as_ref().is_some_and(|path| path != &existing) {
                    return Err("checkout_path_conflict".into());
                }
                let owner = crate::checkout_owner::OwnerOpen::Worktree {
                    path: existing.clone(),
                    repository_root: repo.clone(),
                    label: if existing == *repo {
                        repo.trim_end_matches('/')
                            .rsplit('/')
                            .next()
                            .unwrap_or(repo)
                            .to_owned()
                    } else {
                        branch.clone()
                    },
                };
                // The persistent unique tab label makes an interrupted tab-create
                // discoverable before a retry could create another child pane.
                let snapshot = engine.session(Wait::Each(Duration::from_secs(5))).map_err(
                    |error| match error {
                        EngineError::Answer(_) | EngineError::Protocol { .. } => {
                            "snapshot_unavailable".to_owned()
                        }
                        error => error.to_string(),
                    },
                )?;
                let label = format!("hide:{}", reserved.id);
                let made = made_by_spawn(
                    &existing,
                    repo,
                    snapshot
                        .workspaces
                        .iter()
                        .map(|workspace| workspace.label.as_str()),
                    &label,
                );
                // A split beside the caller runs Herdr's own shell, as a fork's
                // does, so only a tab is launched with Hide's.
                let launch = beside_of
                    .is_none()
                    .then(|| crate::agent_start::shell_launch(node.as_deref(), &reserved.id))
                    .flatten();
                // A tab under the spawn's label is one this spawn opened, with
                // the launch when there is one; a workspace under it holds
                // Herdr's own first tab, which was never replaced.
                let labelled_tab = snapshot
                    .tabs
                    .iter()
                    .find(|tab| tab.label == label)
                    .and_then(|tab| snapshot.panes.iter().find(|pane| pane.tab_id == tab.tab_id))
                    .map(|pane| (pane.pane_id.clone(), true));
                let existing_pane = labelled_tab.or_else(|| {
                    snapshot
                        .workspaces
                        .iter()
                        .find(|workspace| workspace.label == label)
                        .and_then(|workspace| {
                            snapshot
                                .panes
                                .iter()
                                .find(|pane| pane.workspace_id == workspace.workspace_id)
                        })
                        .map(|pane| (pane.pane_id.clone(), false))
                });
                let mut seed_tab = None;
                let pane = match (beside_of.as_deref(), existing_pane) {
                    // Herdr's own shell runs in the split, as in a fork's.
                    (Some(caller), _) => {
                        shell = ShellStart::Plain;
                        let (pane, split) = beside_pane(
                            engine.as_ref(),
                            &snapshot,
                            caller,
                            &existing,
                            &reserved.id,
                        )?;
                        // Drawing ahead is only the screen's: Herdr's layout
                        // replaces it, so a runtime that cannot draw it now
                        // never fails the spawn.
                        if split
                            && on_node
                            && let Some(runtime) = client.runtime.upgrade()
                        {
                            match runtime.lock() {
                                Ok(mut runtime) => runtime.draw_answered_split(caller, &pane),
                                Err(_) => crate::diagnostic!(json!({
                                    "component": "spawn", "kind": "beside.draw_skipped",
                                    "spawn": reserved.id, "pane_id": pane,
                                })),
                            }
                        }
                        pane
                    }
                    (None, Some((pane, launched))) => {
                        shell = ShellStart::of_tab(launched, launch.as_ref());
                        pane
                    }
                    (None, None) => {
                        let tab = crate::checkout_owner::open::open_owner_tab(
                            engine.as_ref(),
                            &owner,
                            &existing,
                            &label,
                            false,
                            launch
                                .as_ref()
                                .map(|launch| launch.env.clone())
                                .unwrap_or_default(),
                            launch.as_ref().map(|launch| launch.command.as_slice()),
                        )
                        .map_err(|error| format!("{error:?}"))?;
                        shell = ShellStart::of_tab(tab.launched, launch.as_ref());
                        seed_tab = tab.initial_label.map(|label| super::naming::SeedTab {
                            id: tab.tab_id,
                            label,
                        });
                        tab.pane_id
                    }
                };
                (existing, pane, seed_tab, made)
            } else if beside_of.is_some() {
                // The caller's worktree was listed a moment ago and is gone
                // now; a spawn beside never makes one.
                return Err("beside_other_checkout".into());
            } else {
                // A device whose helper is not connected leaves the start to
                // Herdr, the folder's own HEAD.
                let start = node
                    .as_deref()
                    .and_then(|node| crate::worktree_control::new_branch_start(node, repo, None));
                let created = engine
                    .create_worktree(
                        Wait::Each(WORKTREE_CREATE_TIMEOUT),
                        repo,
                        branch,
                        start.as_deref(),
                        Some(&format!("hide:{}", reserved.id)),
                        path.as_deref(),
                        false,
                    )
                    .map_err(|error| match error {
                        EngineError::Request(message) | EngineError::Answer(message) => message,
                        error => worktree_create_refusal(error),
                    })?;
                let launch = crate::agent_start::shell_launch(node.as_deref(), &reserved.id);
                let tab = crate::checkout_owner::open::replace_tab(
                    engine.as_ref(),
                    crate::checkout_owner::open::OwnedTab {
                        initial_label: Some(created.tab_label),
                        workspace_id: created.workspace_id,
                        tab_id: created.tab_id,
                        pane_id: created.pane_id,
                        launched: false,
                    },
                    &created.path,
                    &format!("hide:{}", reserved.id),
                    false,
                    launch
                        .as_ref()
                        .map(|launch| launch.env.clone())
                        .unwrap_or_default(),
                    launch.as_ref().map(|launch| launch.command.as_slice()),
                );
                shell = ShellStart::of_tab(tab.launched, launch.as_ref());
                let seed_tab = tab.initial_label.map(|label| super::naming::SeedTab {
                    id: tab.tab_id,
                    label,
                });
                (created.path, tab.pane_id, seed_tab, true)
            };
            if made {
                crate::agent_start::trust_worktree(node.as_deref(), kind, &checkout)?;
            }
            // The terminal is recorded with the pane, so a later attempt can
            // tell this pane from one Herdr gives the same id afterwards.
            let terminal = pane_terminal(engine.as_ref(), &pane)?;
            reserved = serde_json::from_value(mutate(
                client,
                authority,
                actor,
                Mutation::Advance {
                    seed_tab,
                    id: reserved.id.clone(),
                    path: Some(checkout),
                    pane: Some(pane),
                    terminal: Some(terminal),
                    shell: shell.program().map(str::to_owned),
                    child: None,
                },
            )?)
            .map_err(|_| "spawn_unavailable")?;
            // The record names the split now, so its label has done its work
            // and leaves Herdr's own title of the pane.
            if let (Some(_), Some(pane)) = (beside_of.as_ref(), reserved.pane.as_deref()) {
                let cleared = engine
                    .rename_pane(Wait::Each(Duration::from_secs(5)), pane, None)
                    .map_err(String::from);
                if let Err(detail) = cleared {
                    crate::diagnostic!(json!({
                        "component": "spawn", "kind": "beside.label_kept",
                        "spawn": reserved.id, "pane_id": pane, "detail": detail,
                    }));
                }
            }
        }
        let pane = reserved.pane.clone().ok_or("spawn_unavailable")?;
        let pane = pane.as_str();
        let shell = ShellStart::launched(reserved.shell.clone());
        let observed = agents(engine.as_ref())?;
        let live = observed
            .iter()
            .find(|agent| native_matches(agent, pane, Some(name), kind));
        if live.is_none() {
            // A changed session in an already registered child is refused. A
            // retry never launches over a replacement occupant.
            if reserved.child.is_some() {
                return Err("child_identity_changed".into());
            }
            if reserved.typed > 0 {
                // A start of this spawn was typed here already. While Herdr
                // holds its name with no agent (`launch_pending`), it gives
                // that name to whatever agent shows in this terminal and
                // refuses another start here, so nothing is typed while the
                // agent may still be starting: asking again later answers it.
                let held = observed.iter().any(|agent| {
                    agent.pane_id == pane
                        && agent.name.as_deref() == Some(name)
                        && agent.agent.is_none()
                });
                if held && !name_released(engine.as_ref(), name) {
                    return Err("native_identity_unavailable".into());
                }
                // Herdr gave that start up. Only a shell back at rest, with
                // nothing of its own in the foreground, takes the start again,
                // and only once: a start typed into a question at startup is
                // typed again at the prompt that followed, and a second one
                // that ends the same way is the answer.
                if reserved.typed >= 2 || shell_alone(engine.as_ref(), pane) != Ok(true) {
                    return Err("agent_not_started".into());
                }
                crate::diagnostic!(json!({
                    "component": "spawn", "kind": "start.resent", "spawn": reserved.id,
                    "pane_id": pane,
                }));
            }
            close_unstarted_attempts(engine.as_ref(), &*state(client)?, &reserved, &observed);
            let mut start_args = with_permission(kind, &permissions, start_args);
            // A fresh start carries the brief as its prompt argument where
            // the agent holds that through its own start screens
            // (`AgentApi::LaunchPrompt`). The words are decided for the
            // first start and saved before it, so a start typed again carries
            // the same words and nothing ever types the brief.
            let launched_before = reserved
                .brief_input
                .as_ref()
                .is_some_and(|input| input.route == SendRoute::Launch);
            let words = brief_words(&reserved, &start_args, || {
                if let Some(form) = checked_form {
                    return Ok(form);
                }
                let node = node.as_deref().ok_or("no machine runs the pane")?;
                start_form(node, kind)
                    .map_err(|error| {
                        format!("the pane's machine did not say how the agent starts ({error})")
                    })?
                    .ok_or_else(|| "the agent is not on the PATH of the pane's machine".to_owned())
            })?;
            if let Some(words) = words {
                if !launched_before {
                    reserved = serde_json::from_value(mutate(
                        client,
                        authority,
                        actor,
                        Mutation::BriefLaunched {
                            id: reserved.id.clone(),
                            words: words.clone(),
                        },
                    )?)
                    .map_err(|_| "spawn_unavailable")?;
                }
                start_args.extend(words);
                crate::diagnostic!(json!({
                    "component": "spawn", "kind": "brief.launched", "spawn": reserved.id,
                    "pane_id": pane,
                }));
            }
            crate::live::start_agent(
                engine.as_ref(),
                node.as_deref(),
                &reserved.id,
                pane,
                shell,
                name,
                kind,
                start_args,
                codex_daemon,
            )?;
            reserved = serde_json::from_value(mutate(
                client,
                authority,
                actor,
                Mutation::Typed {
                    id: reserved.id.clone(),
                },
            )?)
            .map_err(|_| "spawn_unavailable")?;
        }
        // The brief is the child's first prompt, submitted once the agent
        // takes input and before its session is waited for: an agent that
        // reports its session only at its first turn (Codex, OpenCode 2)
        // reports it on this very turn. A spawn asked again after the brief
        // went in never submits it twice.
        if let Some(brief) = reserved.brief.clone().filter(|_| !reserved.briefed) {
            if hide_agent_adapter::adapter(kind).is_some_and(|row| row.key.brief_after_session()) {
                // Its session start drops an Enter typed during it
                // (`AgentId::brief_after_session`). Unseen within the wait,
                // the brief goes in anyway and its record still judges it.
                if let Err(reason) = wait_native_identity(engine.as_ref(), pane, Some(name), kind) {
                    crate::diagnostic!(json!({
                        "component": "spawn", "kind": "brief.session_unseen",
                        "spawn": reserved.id, "pane_id": pane, "reason": reason,
                    }));
                }
            }
            brief_ready(engine.as_ref(), &reserved.id, pane, name)?;
            let pane_key = if on_node {
                pane.to_owned()
            } else {
                crate::address::scoped_pane_id(&child_device, pane)
            };
            // The session Herdr names for the agent now, read off the lock:
            // until it names one, the brief is typed.
            let lineage = engine
                .delivery_agent(Wait::Each(Duration::from_secs(5)), pane)
                .ok()
                .and_then(|agent| agent.session);
            let target = {
                let runtime = client.runtime.upgrade().ok_or("delivery_unavailable")?;
                let guard = runtime.lock().map_err(|_| "delivery_unavailable")?;
                guard.input_target_for(&pane_key, pane, kind, &child_device, lineage.as_deref())
            };
            let prepared = crate::delivery::ladder::prepare(&target);
            // Saved before it is sent, so a send a crash interrupts is
            // counted and never repeated blindly (engineering #11).
            let reservation = mutate(
                client,
                authority,
                actor,
                Mutation::BriefTyping {
                    id: reserved.id.clone(),
                },
            );
            let sent = match reservation {
                // An earlier attempt's send may have arrived and its outcome
                // was never saved: this spawn answers as that one would
                // have, sent and unconfirmed, and the agent's own record
                // decides (`hide agent show` says which).
                Err(code) if code == "brief_send_lost" => {
                    crate::diagnostic!(json!({
                        "component": "spawn", "kind": "brief.send_unconfirmed",
                        "spawn": reserved.id, "pane_id": pane,
                    }));
                    SendRoute::Lost
                }
                Err(code) if code == "brief_attempts_spent" => {
                    return Err(brief_refused(
                        &reserved.id,
                        pane,
                        "brief.attempts_spent",
                        code,
                    ));
                }
                Err(code) => return Err(code),
                Ok(value) => {
                    reserved = serde_json::from_value(value).map_err(|_| "spawn_unavailable")?;
                    match send_brief(engine.as_ref(), (&target, prepared), &reserved.id, &brief) {
                        Ok(sent) => sent,
                        Err(code) => {
                            // Nothing was sent: the brief may be typed again.
                            mutate(
                                client,
                                authority,
                                actor,
                                Mutation::BriefUnsent {
                                    id: reserved.id.clone(),
                                },
                            )?;
                            return Err(code);
                        }
                    }
                }
            };
            reserved = serde_json::from_value(mutate(
                client,
                authority,
                actor,
                Mutation::Briefed {
                    id: reserved.id.clone(),
                    route: sent,
                },
            )?)
            .map_err(|_| "spawn_unavailable")?;
        }
        // The child's pane keeps its launch choice for the restarts Hide
        // makes of it, under the id the core knows the pane by and the name
        // it was started under. It is kept before the agent shows its
        // session, which an agent that reports it only at its first turn
        // does later, so the record binds to that session when it arrives.
        if let Some(runtime) = client.runtime.upgrade() {
            let pane_id = if on_node {
                pane.to_owned()
            } else {
                crate::address::scoped_pane_id(&child_device, pane)
            };
            runtime
                .lock()
                .map_err(|_| "delivery_unavailable")?
                .record_pane_launch(&pane_id, kind, name, &reserved.launch, None);
        }
        let native = match wait_native_identity(engine.as_ref(), pane, Some(name), kind) {
            Err(reason) if reason == "native_identity_unavailable" => {
                // The agent rests without a session (Codex and OpenCode 2
                // report theirs at their first turn): the core binds it when
                // Herdr shows one, and the caller hears it has not yet.
                mutate(
                    client,
                    authority,
                    actor,
                    Mutation::AwaitIdentity {
                        wait: super::identity_wait::IdentityWait {
                            id: String::new(),
                            machine: child_device.clone(),
                            pane: pane.into(),
                            name: name.clone(),
                            kind: kind.clone(),
                            owed: super::identity_wait::Owed::Spawn {
                                spawn: reserved.id.clone(),
                            },
                        },
                    },
                )?;
                return Err(reason);
            }
            native => native?,
        };
        let session = native
            .agent_session
            .as_ref()
            .map(|session| session.value.as_str());
        if let Err(reason) = session.map_or(Ok(()), super::kept_session) {
            // The agent runs but can never be registered, and a retry of
            // this intent meets the same session: the pane this spawn made is
            // closed rather than left running with nothing tracking it.
            close_refused_pane(engine.as_ref(), &reserved, pane, reason);
            return Err(reason.into());
        }
        let mut child = record(
            &native,
            HostIdentity {
                machine: &child_device,
                scope: &host_scope,
                native_machine: &native_machine,
                on_node,
            },
            pane.into(),
            Some(name.clone()),
            reserved.mode.responsibility(&parent_id),
            reserved.path.clone(),
        )?;
        child.origin = reserved.mode.origin(&parent_id);
        reserved = serde_json::from_value(mutate(
            client,
            authority,
            actor,
            Mutation::BindChild {
                id: reserved.id.clone(),
                record: child,
            },
        )?)
        .map_err(|_| "spawn_unavailable")?;
        let child_id = reserved.child.clone().ok_or("child_unavailable")?;
        publish_tokens(client, engine.as_ref(), &child_id)?;
        mutate(
            client,
            authority,
            actor,
            Mutation::Complete {
                id: reserved.id.clone(),
            },
        )?;
        // The receipt-driven lineage worker alone owns naming. Keeping it
        // out of this path also keeps completed replay entirely offline.
        let (ledger, sleeping) = registrations(client)?;
        Ok(view(
            ledger
                .agents
                .iter()
                .find(|record| record.id == child_id)
                .ok_or("child_unavailable")?,
            &ledger,
            &sleeping,
        ))
    })();
    // Nothing after this creates anything, so a device that has gone quiet
    // never holds the lock for the probe below.
    drop(single);
    placed.map_err(|reason| {
        // The caller may have stopped waiting for this answer, so the log is
        // where a spawn that stopped partway is seen; the same intent
        // continues it.
        crate::diagnostic!(json!({
            "component": "spawn", "kind": "spawn.unfinished", "spawn": spawn_id,
            "reason": reason,
        }));
        if on_device {
            device_unreachable(engine.as_ref(), &child_device, &spawn_id, reason)
        } else {
            reason
        }
    })
}

/// A spawn's record told each pane the engine makes for it before the pane
/// runs anything ([`crate::engine::Placement`]): the record's mutation takes
/// the delivery store's turn and the runtime's lock, never the engine's
/// topology lock, which the engine let go before it asked.
struct SpawnPlacement<'c> {
    client: &'c Client,
    authority: &'c Authority,
    actor: &'c Actor,
    spawn: String,
}

impl crate::engine::Placement for SpawnPlacement<'_> {
    fn place(&self, pane: &str) -> Result<(), EngineError> {
        mutate(
            self.client,
            self.authority,
            self.actor,
            Mutation::Place {
                id: self.spawn.clone(),
                pane: pane.to_owned(),
            },
        )
        .map(drop)
        .map_err(|code| EngineError::Refused {
            message: format!("spawn {} could not name pane {pane}: {code}", self.spawn),
            code,
        })
    }

    fn withdraw(&self, pane: &str) {
        let withdrawn = mutate(
            self.client,
            self.authority,
            self.actor,
            Mutation::Withdraw {
                id: self.spawn.clone(),
                pane: pane.to_owned(),
            },
        );
        if let Err(code) = withdrawn {
            // The record names a pane that never ran until the same intent
            // asks again, which finds no such pane.
            crate::diagnostic!(json!({
                "component": "spawn", "kind": "placement.withdraw_failed",
                "spawn": self.spawn, "pane_id": pane, "code": code,
            }));
        }
    }
}

/// The caller's own pane a spawn beside it splits, on the caller's own
/// device: refused, with nothing created, when the child would run on another
/// device or in another checkout than the caller's (a new worktree, another
/// branch or another folder), or when the caller is no agent pane Hide knows.
/// `target` is the repository, branch and folder asked for, compared with
/// Herdr's worktrees; none for a retry whose pane exists already.
fn beside_target(
    client: &Client,
    authority: &Authority,
    actor: &Actor,
    machine: Option<&str>,
    target: Option<(&str, &str, Option<&str>)>,
    intent: &str,
) -> Result<String, String> {
    let refused = |reason: &str| {
        crate::diagnostic!(json!({
            "component": "spawn", "kind": "beside.refused", "intent": intent,
            "pane_id": actor.pane_id, "reason": reason,
        }));
        reason.to_owned()
    };
    if machine.is_some() {
        return Err(refused("beside_other_machine"));
    }
    let checkout = authority.context.checkout_path.trim_end_matches('/');
    if actor.code_owned() || checkout.is_empty() {
        return Err(refused("beside_needs_agent_pane"));
    }
    let pane = crate::address::pane_of(&actor.pane_id, &actor.device_id)
        .unwrap_or(&actor.pane_id)
        .to_owned();
    let Some((repo, branch, path)) = target else {
        return Ok(pane);
    };
    let CoordinationContext { engine, .. } = context(client, &actor.device_id)?;
    let existing = engine
        .worktree_path(Wait::Each(Duration::from_secs(5)), repo, branch)
        .map_err(String::from)?;
    let same = |other: &str| other.trim_end_matches('/') == checkout;
    if !existing.as_deref().is_some_and(same) || path.is_some_and(|path| !same(path)) {
        return Err(refused("beside_other_checkout"));
    }
    Ok(pane)
}

/// The pane a spawn beside `caller` runs in, and whether it was split now:
/// the pane an earlier attempt split and labelled, else a new split right of
/// the caller, labelled before anything else so that, once labelled, a retry
/// finds it rather than splitting again. A split that cannot be labelled is
/// closed; one whose own reply was lost is never labelled, and a retry makes
/// its own.
fn beside_pane(
    engine: &dyn Engine,
    snapshot: &crate::session_sync::ProjectionState,
    caller: &str,
    cwd: &str,
    spawn: &str,
) -> Result<(String, bool), String> {
    let label = crate::start_claim::spawn_label(spawn);
    if let Some(pane) = snapshot
        .panes
        .iter()
        .find(|pane| pane.label.as_deref() == Some(label.as_str()))
    {
        return Ok((pane.pane_id.clone(), false));
    }
    let refused = |detail: String| {
        crate::diagnostic!(json!({
            "component": "spawn", "kind": "beside.split_failed", "spawn": spawn,
            "pane_id": caller, "detail": detail,
        }));
        "beside_pane_unavailable".to_owned()
    };
    let pane = match engine.split_beside(
        Wait::Each(Duration::from_secs(5)),
        caller,
        Some(cwd),
        Default::default(),
    ) {
        Err(EngineError::Request(message)) => return Err(message),
        pane => pane.map_err(|error| refused(error.to_string()))?,
    };
    match engine.rename_pane(Wait::Each(Duration::from_secs(5)), &pane, Some(&label)) {
        Ok(()) => {}
        Err(EngineError::Request(message)) => return Err(message),
        Err(error) => {
            let closed = close_pane(engine, &pane);
            return Err(refused(format!(
                "{error}; split pane {pane} closed: {}",
                closed.is_ok()
            )));
        }
    }
    crate::diagnostic!(json!({
        "component": "spawn", "kind": "beside.split", "spawn": spawn,
        "pane_id": caller, "created": pane,
    }));
    Ok((pane, true))
}

/// Whether Herdr has let go of `name`. Herdr holds the name of a start whose
/// agent never showed until that start's own deadline, and settles a passed
/// deadline only when the name is read, which this `agent.get` is.
fn name_released(engine: &dyn Engine, name: &str) -> bool {
    engine
        .delivery_agent(Wait::Each(Duration::from_secs(2)), name)
        .is_err_and(|error| error.code() == Some("agent_not_found"))
}

/// Whether `pane`'s shell alone holds its terminal: no program it started
/// runs in the foreground.
fn shell_alone(engine: &dyn Engine, pane: &str) -> Result<bool, String> {
    engine
        .pane_processes(Wait::Each(Duration::from_secs(2)), pane)
        .map_err(String::from)
        .map(|group| group.shell_holds_terminal())
}

/// The terminal Herdr runs in `pane` now.
fn pane_terminal(engine: &dyn Engine, pane: &str) -> Result<String, String> {
    engine
        .pane_terminal(Wait::Each(Duration::from_secs(5)), pane)
        .map_err(String::from)
}

/// Closes the pane of every earlier attempt of `reserved`: a spawn of the
/// same parent, name and device that made a pane and never bound a child,
/// because its agent did not start (a refused start, or one Herdr typed that
/// never showed its agent). Each new attempt is its own intent with a pane of
/// its own, so the earlier pane would stay behind as an extra tab, holding the
/// agent's name in Herdr when its start was typed. Herdr hands a pane id out
/// again (a new server, a live handoff), so a pane is the attempt's own only
/// while it runs the terminal the attempt recorded; any other pane under that
/// id, and a record from before terminals were recorded, is logged and left
/// open. A pane where an agent shows or the shell does not hold the terminal
/// is left alone, and a pane that cannot be read or closed is logged and left
/// to Herdr's own answer.
fn close_unstarted_attempts(
    engine: &dyn Engine,
    ledger: &crate::delivery::ledger::Ledger,
    reserved: &SpawnRecord,
    observed: &[ProjectedAgent],
) {
    let earlier = ledger.spawns.iter().filter(|record| {
        record.id != reserved.id
            && record.parent == reserved.parent
            && record.name == reserved.name
            && record.machine == reserved.machine
            && record.child.is_none()
            && !record.completed
            && record.pane != reserved.pane
    });
    for record in earlier {
        let Some(pane) = record.pane.as_deref() else {
            continue;
        };
        if observed
            .iter()
            .any(|agent| agent.pane_id == pane && agent.agent.is_some())
        {
            continue;
        }
        let closed =
            match same_terminal(engine, record, pane).and_then(|()| shell_alone(engine, pane)) {
                Ok(false) => continue,
                Ok(true) => close_pane(engine, pane),
                Err(reason) => Err(reason),
            };
        crate::diagnostic!(json!({
            "component": "spawn", "kind": "unstarted_attempt.close", "spawn": reserved.id,
            "attempt": record.id, "pane_id": pane, "closed": closed.is_ok(),
            "reason": closed.err(),
        }));
    }
}

/// Closes the pane `reserved` made, whose agent started but whose session a
/// registration cannot keep (`reason`), when it is still the terminal the
/// spawn recorded.
pub(super) fn close_refused_pane(
    engine: &dyn Engine,
    reserved: &SpawnRecord,
    pane: &str,
    reason: &str,
) {
    let closed = same_terminal(engine, reserved, pane).and_then(|()| close_pane(engine, pane));
    crate::diagnostic!(json!({
        "component": "spawn", "kind": "refused_pane.close", "spawn": reserved.id,
        "pane_id": pane, "refusal": reason, "closed": closed.is_ok(),
        "reason": closed.err(),
    }));
}

/// Whether `pane` is still the terminal `record` recorded with it, so a pane
/// Herdr gave the same id afterwards is never taken for the spawn's.
fn same_terminal(engine: &dyn Engine, record: &SpawnRecord, pane: &str) -> Result<(), String> {
    let recorded = record
        .terminal
        .as_deref()
        .ok_or("pane_identity_unrecorded")?;
    if pane_terminal(engine, pane)? == recorded {
        Ok(())
    } else {
        Err("pane_replaced".to_owned())
    }
}

fn close_pane(engine: &dyn Engine, pane: &str) -> Result<(), String> {
    engine
        .close_pane(Wait::Each(Duration::from_secs(5)), pane)
        .map_err(String::from)
}

/// A failure of a spawn's work on a device, as the caller should read it: the
/// device is unavailable when its Herdr no longer answers, whatever step was
/// running; any other failure keeps its own code. The dropped cause is logged
/// with the device and the spawn it belongs to.
fn device_unreachable(engine: &dyn Engine, device: &str, spawn: &str, reason: String) -> String {
    match engine.agents(Wait::Each(Duration::from_secs(2))) {
        Err(
            error @ (EngineError::NotRunning(_)
            | EngineError::Transport(_)
            | EngineError::Unanswered(_)),
        ) => {
            crate::diagnostic!(json!({
                "component": "spawn", "kind": "device_unreachable", "device": device,
                "spawn": spawn, "reason": reason, "cause": error.to_string()
            }));
            "machine_unavailable".to_owned()
        }
        _ => reason,
    }
}

/// Records the fork Hide started in pane `child` from the agent in pane
/// `source`, under the Herdr name `name` the fork's start gave it. The
/// source is registered now, while it is the execution that was forked; the
/// fork is registered once Herdr shows its own session, which Codex reports
/// only at the fork's first turn, so the registration waits for it in the
/// ledger (`identity_wait`) rather than for a fixed time here.
pub(crate) fn link_fork(
    context: &crate::live::LiveContext,
    source: &str,
    child: &str,
    name: &str,
) -> Result<(), String> {
    let runtime = context.runtime.upgrade().ok_or("delivery_unavailable")?;
    let (client, authority, actor) = runtime
        .lock()
        .map_err(|_| "delivery_unavailable")?
        .coordination_fork_context(source)?;
    let CoordinationContext {
        engine,
        host_scope,
        machine: native_machine,
        on_node,
        ..
    } = self::context(&client, &actor.device_id)?;
    let observed = agents(engine.as_ref())?;
    let native_source = observed
        .iter()
        .find(|agent| agent.pane_id == source)
        .ok_or("parent_unavailable")?;
    let ledger = state(&client)?;
    let source_id = if let Some(existing) = ledger
        .agents
        .iter()
        .find(|record| !record.ended && record.actor.same_identity(&actor))
    {
        existing.id.clone()
    } else {
        let source_record = record(
            native_source,
            HostIdentity {
                machine: &actor.device_id,
                scope: &host_scope,
                native_machine: &native_machine,
                on_node,
            },
            source.into(),
            native_source.name.clone(),
            None,
            None,
        )?;
        let source_value = mutate(
            &client,
            &authority,
            &actor,
            Mutation::Register {
                record: source_record,
                check: false,
            },
        )?;
        source_value["id"]
            .as_str()
            .ok_or("parent_unavailable")?
            .to_owned()
    };
    mutate(
        &client,
        &authority,
        &actor,
        Mutation::AwaitIdentity {
            wait: super::identity_wait::IdentityWait {
                id: String::new(),
                machine: actor.device_id.clone(),
                pane: child.into(),
                name: name.into(),
                kind: native_source.agent.clone().ok_or("parent_unavailable")?,
                owed: super::identity_wait::Owed::Fork { origin: source_id },
            },
        },
    )
    .map(drop)
}

/// Records an agent `source` started through Herdr directly, observed after
/// the fact (PRD direct-prompt-observation-widen D-23, D-25): the operator's
/// root with `source` as its origin, the shape a handoff's child has, with
/// no parent, watch or pane tokens. `started` picks the agent out of Herdr's
/// current list; exactly one must answer, with its session already shown.
pub(crate) fn link_started(
    runtime: &std::sync::Weak<std::sync::Mutex<crate::runtime::Runtime>>,
    source: &crate::delivery::Actor,
    raw_source: &str,
    started: impl Fn(&ProjectedAgent) -> bool,
    within: Duration,
) -> Result<String, String> {
    let runtime = runtime.upgrade().ok_or("delivery_unavailable")?;
    let (client, authority, actor) = runtime
        .lock()
        .map_err(|_| "delivery_unavailable")?
        .coordination_fork_context(&source.pane_id)?;
    // The pane may hold another agent since the sender was proven.
    if !actor.same_identity(source) {
        return Err("observation_native_changed".into());
    }
    let CoordinationContext {
        engine,
        host_scope,
        machine: native_machine,
        on_node,
        ..
    } = self::context(&client, &actor.device_id)?;
    let observed = agents_with_timeout(engine.as_ref(), within)?;
    let mut matching = observed.iter().filter(|agent| started(agent));
    let child = matching.next().ok_or("child_unavailable")?;
    if matching.next().is_some() {
        return Err("child_ambiguous".into());
    }
    let host = || HostIdentity {
        machine: &actor.device_id,
        scope: &host_scope,
        native_machine: &native_machine,
        on_node,
    };
    let ledger = state(&client)?;
    let source_id = if let Some(existing) = ledger
        .agents
        .iter()
        .find(|record| !record.ended && record.actor.same_identity(&actor))
    {
        existing.id.clone()
    } else {
        let native_source = observed
            .iter()
            .find(|agent| agent.pane_id == raw_source)
            .ok_or("parent_unavailable")?;
        let source_record = record(
            native_source,
            host(),
            raw_source.into(),
            native_source.name.clone(),
            None,
            None,
        )?;
        mutate(
            &client,
            &authority,
            &actor,
            Mutation::Register {
                record: source_record,
                check: false,
            },
        )?["id"]
            .as_str()
            .ok_or("parent_unavailable")?
            .to_owned()
    };
    let mut child_record = record(
        child,
        host(),
        child.pane_id.clone(),
        child.name.clone(),
        None,
        child.cwd.clone(),
    )?;
    child_record.origin = Some(source_id);
    mutate(
        &client,
        &authority,
        &actor,
        Mutation::RegisterStarted {
            record: child_record,
        },
    )?["id"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "child_unavailable".into())
}

/// Whether the existing worktree `existing` of `repo` is one this spawn
/// made in an attempt whose reply was lost: `worktree.create` names the
/// workspace it makes `label` (`hide:<spawn id>`). The repository itself,
/// or a worktree with no such workspace, is the operator's.
fn made_by_spawn<'a>(
    existing: &str,
    repo: &str,
    mut workspaces: impl Iterator<Item = &'a str>,
    label: &str,
) -> bool {
    existing != repo && workspaces.any(|workspace| workspace == label)
}

/// A spawn's arguments behind the permission choice of the machine it
/// starts on (`agent_permission`). A full-access option the spawn's own
/// native arguments already name is not given twice, with its value if it
/// has one: a CLI may refuse an option given twice (Grok 1.0.70 refuses
/// `--always-approve` twice), and the operator who wrote it meant theirs.
/// Codex's `-c` overrides are the exception: Codex accepts them repeated and
/// the later one wins (0.166.0, 2026-11-01), so Hide's go first and a
/// spawn's own `-c`, an effort or its own approval policy, never stands in
/// for them. A spawn's own Claude Code `--settings` replaces Hide's warning
/// setting rather than joining it, because Claude Code 2.1.316 keeps only the
/// last `--settings` it is given (measured 2026-11-01), so such a spawn stops
/// at the bypass warning. Typed options go as typed.
fn with_permission(
    kind: &str,
    permissions: &crate::agent_permission::AgentPermissions,
    args: Vec<String>,
) -> Vec<String> {
    use crate::agent_permission::AgentPermission;
    let permission = crate::agent_permission::of(permissions, kind);
    let mut start: Vec<String> = match &permission {
        AgentPermission::Full => {
            let named = |name: &str| {
                args.iter()
                    .take_while(|arg| arg.as_str() != "--")
                    .any(|arg| arg == name)
            };
            crate::agent_permission::full_access(kind)
                .iter()
                .filter(|option| {
                    option
                        .first()
                        .is_none_or(|name| overridden_later(kind, name) || !named(name))
                })
                .flat_map(|option| option.iter().map(|word| (*word).to_owned()))
                .collect()
        }
        _ => crate::agent_permission::arguments(kind, &permission),
    };
    start.extend(args);
    start
}

/// Whether `kind`'s CLI takes a full-access option named `name` again with
/// the later one winning: Codex's `-c` configuration override.
fn overridden_later(kind: &str, name: &str) -> bool {
    name == "-c"
        && hide_agent_adapter::adapter(kind).and_then(|row| row.start)
            == Some(hide_agent_adapter::LaunchDialect::Codex)
}

#[cfg(test)]
mod tests {
    use hide_node_link::protocol::StartForm;

    /// A brief rides the start only where the pane's machine said the start
    /// carries it whole: through `cmd.exe` (a Windows batch file, npm's shim)
    /// a brief past its line, or with a character it reads as its own, is
    /// typed, and so is one whose start form the node could not tell.
    #[test]
    fn a_brief_rides_the_start_only_where_the_start_carries_it_and_the_machine_is_asked_last() {
        let program = || Ok(StartForm::Program);
        // A whole brief rides the start, starting with `-` or not.
        let brief = format!("-{}", "x".repeat(16 * 1024 - 1));
        assert_eq!(
            super::launch_prompt("omp", &[], &brief, program).unwrap(),
            ["--".to_owned(), brief.clone()]
        );
        assert_eq!(
            super::launch_prompt("omp", &[], "@a", || Ok(StartForm::Batch)),
            None
        );
        assert_eq!(
            super::launch_prompt("omp", &[], "go", || Err("link_lost".into())),
            None
        );
        // Nothing asks the pane's machine for an agent that holds no launch
        // prompt, a spawn whose native arguments give one, or a brief the
        // command line refuses.
        let unasked = || -> Result<StartForm, String> { panic!("the machine was asked") };
        assert_eq!(super::launch_prompt("pi", &[], "go", unasked), None);
        assert_eq!(super::launch_prompt("opencode", &[], "go", unasked), None);
        assert_eq!(super::launch_prompt("cursor", &[], "go", unasked), None);
        assert_eq!(
            super::launch_prompt("codex", &["--".into(), "mine".into()], "go", unasked),
            None
        );
        assert_eq!(super::launch_prompt("codex", &[], "a\u{7}b", unasked), None);
    }

    /// A start typed again for a spawn whose brief rode its first start
    /// carries the saved words and asks the machine nothing, whatever the
    /// adapter or the machine would say now; a record an earlier build saved
    /// without its words is given them as that build gave them, and fails
    /// where that start can carry none.
    #[test]
    fn a_launched_brief_carries_its_saved_words_on_every_later_start() {
        let record = |kind: &str, route: Option<&str>, words: &[&str]| -> super::SpawnRecord {
            serde_json::from_value(serde_json::json!({
                "id": "spawn-1", "parent": "agent-1", "intent": "i", "name": "child",
                "kind": kind, "repo": "/r", "branch": "b", "requested_path": null,
                "args": [], "path": null, "pane": "w1:p1", "child": null,
                "brief": "-go", "briefed": route.is_some(),
                "brief_input": route.map(|route| serde_json::json!({
                    "attempts": 1, "typed_at_unix_ms": 1, "route": route,
                })),
                "launch_words": words,
            }))
            .unwrap()
        };
        let unasked = || -> Result<StartForm, String> { panic!("the machine was asked") };
        // Saved words win over what the adapter would give now: OpenCode's
        // brief is typed today, and its earlier launch is not undone.
        assert_eq!(
            super::brief_words(
                &record("opencode", Some("launch"), &["--prompt=-go"]),
                &[],
                unasked
            ),
            Ok(Some(vec!["--prompt=-go".to_owned()]))
        );
        // An earlier build's launched record has no words: its start gave
        // the brief as carried, and nothing is asked.
        assert_eq!(
            super::brief_words(&record("codex", Some("launch"), &[]), &[], unasked),
            Ok(Some(vec!["--".to_owned(), "-go".to_owned()]))
        );
        assert_eq!(
            super::brief_words(
                &record("codex", Some("launch"), &[]),
                &["--".into(), "mine".into()],
                unasked
            ),
            Err("brief_launch_unrecorded".to_owned())
        );
        // A brief that was typed is never launched by a later start.
        assert_eq!(
            super::brief_words(&record("codex", Some("typed"), &[]), &[], unasked),
            Ok(None)
        );
        // Only a fresh start asks how the machine starts the agent.
        assert_eq!(
            super::brief_words(&record("codex", None, &[]), &[], || Ok(StartForm::Program)),
            Ok(Some(vec!["--".to_owned(), "-go".to_owned()]))
        );
    }

    #[test]
    fn a_full_access_option_the_spawn_names_itself_is_left_to_the_spawn_with_its_value() {
        let full = crate::agent_permission::CLAUDE_FULL_ACCESS;
        let start = |native: &[&str]| {
            super::with_permission(
                "claude",
                &Default::default(),
                native.iter().map(|arg| (*arg).to_owned()).collect(),
            )
        };
        assert_eq!(start(&["--", "go"]), [&full[..], &["--", "go"]].concat());
        // The spawn's own settings replace Hide's whole option, never only
        // its name, which would leave Hide's value standing alone.
        assert_eq!(
            start(&["--settings", "/x.json", "--", "go"]),
            [full[0], "--settings", "/x.json", "--", "go"]
        );
        assert_eq!(start(&[full[0]]), [full[1], full[2], full[0]]);
        // A word after `--` is the prompt, not an option.
        assert_eq!(
            start(&["--", "--settings"]),
            [&full[..], &["--", "--settings"]].concat()
        );
    }

    #[test]
    fn a_codex_spawn_with_its_own_override_still_starts_with_full_access_first() {
        let full = crate::agent_permission::CODEX_FULL_ACCESS;
        let start = |native: &[&str]| {
            super::with_permission(
                "codex",
                &Default::default(),
                native.iter().map(|arg| (*arg).to_owned()).collect(),
            )
        };
        // An effort is a `-c` too; it does not stand in for full access.
        let effort = ["-c", "model_reasoning_effort=low", "--", "go"];
        assert_eq!(start(&effort), [&full[..], &effort[..]].concat());
        // The spawn's own policy comes after Hide's, where Codex lets it win.
        let own = ["-c", "approval_policy=on-request"];
        assert_eq!(start(&own), [&full[..], &own[..]].concat());
    }

    #[test]
    fn only_a_worktree_the_spawn_made_counts_as_made_by_it() {
        let label = "hide:spawn-1";
        assert!(made_by_spawn(
            "/r/wt",
            "/r",
            ["main", label].into_iter(),
            label
        ));
        assert!(!made_by_spawn(
            "/r/wt",
            "/r",
            ["main", "hide:spawn-2"].into_iter(),
            label
        ));
        assert!(!made_by_spawn("/r", "/r", [label].into_iter(), label));
    }

    use super::*;
    use crate::fake_herdr::FakeHerdr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A `worktree.create` whose answer never came may still finish in
    /// Herdr, so it is one stable code the same intent resolves; a refusal
    /// and a connection that never opened keep their own words.
    #[test]
    fn a_worktree_create_with_no_answer_is_refused_as_unconfirmed() {
        for lost in [
            "response could not be read: timed out",
            "response was empty",
        ] {
            assert_eq!(
                worktree_create_refusal(EngineError::Unanswered(lost.into())),
                WORKTREE_UNCONFIRMED,
                "{lost}"
            );
        }
        assert_eq!(
            worktree_create_refusal(EngineError::Transport(
                "connect failed for /s: refused".into()
            )),
            "connect failed for /s: refused"
        );
        assert_eq!(
            worktree_create_refusal(EngineError::Refused {
                code: "branch_in_use".into(),
                message: "taken".into()
            }),
            "branch_in_use: taken"
        );
    }

    #[test]
    fn a_registration_lands_on_the_callers_own_machine() {
        assert_eq!(
            registration_machine(None, crate::node::TEST_NODE).as_deref(),
            Ok(crate::node::TEST_NODE)
        );
        assert_eq!(registration_machine(None, "mini").as_deref(), Ok("mini"));
        assert_eq!(
            registration_machine(Some("mini".into()), "mini").as_deref(),
            Ok("mini")
        );
        assert_eq!(
            registration_machine(Some(crate::node::TEST_NODE.into()), "mini"),
            Err("machine_identity_conflict".into())
        );
        assert_eq!(
            registration_machine(Some(String::new()), crate::node::TEST_NODE),
            Err("invalid_registration".into())
        );
    }

    /// An agent Herdr never named is registered with no name: its pane id,
    /// which Herdr offers as its identifier, is not a name, and neither is a
    /// name that only restates the pane.
    #[test]
    fn registering_an_agent_herdr_never_named_stores_no_name() {
        let herdr = FakeHerdr::start("coordination-unnamed", move |method, _| {
            assert_eq!(method, "agent.list");
            json!({"type":"agent_list","agents":[{
                "pane_id":"w9J:p9R","workspace_id":"w9J","tab_id":"w9J:t1",
                "terminal_id":"terminal","revision":1,"focused":false,
                "agent_status":"idle","agent":"claude",
                "agent_session":{"source":"herdr:claude","agent":"claude",
                    "kind":"id","value":"parent-session"}}]})
        });
        let native =
            wait_native_identity(herdr.engine().as_ref(), "w9J:p9R", None, "claude").unwrap();
        assert_eq!(native.name, None);
        let host = || HostIdentity {
            machine: crate::node::TEST_NODE,
            scope: "scope",
            native_machine: "machine",
            on_node: true,
        };
        let make = |name: Option<String>| {
            record(&native, host(), "terminal".into(), name, None, None).unwrap()
        };
        let unnamed = make(native.name.clone());
        assert_eq!(unnamed.name, None);
        assert_eq!(make(Some("w9J:p9R".into())).name, None);
        assert_eq!(make(Some("lead".into())).name.as_deref(), Some("lead"));
    }

    /// Pi names its session by its file path, which Herdr keeps up to 4096
    /// bytes: a registration keeps the same, and a longer one is refused by
    /// its own reason when the record is made, never as an unavailable ledger.
    #[test]
    fn a_session_path_registers_up_to_herdrs_bound_and_past_it_is_refused_by_name() {
        for (bytes, expected) in [
            (crate::delivery::SESSION_LIMIT, None),
            (
                crate::delivery::SESSION_LIMIT + 1,
                Some(super::super::SESSION_TOO_LONG),
            ),
        ] {
            let file = "/session.jsonl";
            let path = format!("/{}{file}", "d".repeat(bytes - file.len() - 1));
            let reported = path.clone();
            let herdr = FakeHerdr::start("coordination-session-path", move |method, _| {
                assert_eq!(method, "agent.list");
                json!({"type":"agent_list","agents":[{
                    "pane_id":"w9J:p9R","workspace_id":"w9J","tab_id":"w9J:t1",
                    "terminal_id":"terminal","revision":1,"focused":false,
                    "agent_status":"idle","agent":"pi","name":"child",
                    "agent_session":{"source":"herdr:pi","agent":"pi",
                        "kind":"path","value":reported}}]})
            });
            let native =
                wait_native_identity(herdr.engine().as_ref(), "w9J:p9R", Some("child"), "pi")
                    .unwrap();
            let made = record(
                &native,
                HostIdentity {
                    machine: crate::node::TEST_NODE,
                    scope: "scope",
                    native_machine: "machine",
                    on_node: true,
                },
                "terminal".into(),
                native.name.clone(),
                None,
                None,
            );
            match expected {
                None => assert_eq!(made.unwrap().session, path),
                Some(code) => assert_eq!(made.unwrap_err(), code),
            }
        }
    }

    #[test]
    fn native_identity_accepts_known_agent_spellings() {
        // B5/D-08: spelling does not change the native agent's identity.
        for (requested, reported, session_agent) in [
            ("CODEX", "codex", "codex"),
            ("codex", " Codex", "codex"),
            ("claude_code", "claude", "claude"),
        ] {
            let herdr = FakeHerdr::start("coordination-native-alias", move |method, _| {
                assert_eq!(method, "agent.list");
                json!({"type":"agent_list","agents":[{
                    "pane_id":"w2:p1","workspace_id":"w2","tab_id":"w2:t1",
                    "terminal_id":"fixture-child-terminal","revision":1,
                    "focused":false,"agent_status":"idle","agent":reported,"name":"child",
                    "agent_session":{"source":format!("herdr:{session_agent}"),
                        "agent":session_agent,"kind":"id","value":"fixture-child-session"}
                }]})
            });
            let child =
                wait_native_identity(herdr.engine().as_ref(), "w2:p1", Some("child"), requested)
                    .unwrap();
            assert_eq!(child.agent.as_deref(), Some(reported));
            assert_eq!(
                child.lineage_session,
                crate::engine::tokens::session_digest("fixture-child-session")
            );
        }
    }

    #[test]
    fn an_accepted_start_waits_for_native_identity_without_starting_again() {
        let observations = Arc::new(AtomicUsize::new(0));
        let reads = observations.clone();
        let herdr = FakeHerdr::start("coordination-native-ready", move |method, _| {
            assert_eq!(method, "agent.list");
            let mut agent = json!({"pane_id":"w2:p1","workspace_id":"w2",
                "tab_id":"w2:t1","terminal_id":"fixture-child-terminal",
                "revision":1,"focused":false,"agent_status":"idle",
                "agent":"claude","name":"child"});
            if reads.fetch_add(1, Ordering::SeqCst) > 0 {
                agent["agent_session"] = json!({"source":"herdr:claude",
                    "agent":"claude","kind":"id","value":"fixture-child-session"});
            }
            json!({"type":"agent_list","agents":[agent]})
        });
        let child = wait_native_identity(herdr.engine().as_ref(), "w2:p1", Some("child"), "claude")
            .unwrap();
        assert_eq!(
            child.lineage_session,
            crate::engine::tokens::session_digest("fixture-child-session")
        );
        assert_eq!(observations.load(Ordering::SeqCst), 2);
        assert_eq!(herdr.methods(), ["agent.list", "agent.list"]);
    }
    /// A Factory parent and the spawn records of its attempts, each made the
    /// way the executor makes them: reserved, then advanced with its pane and
    /// the terminal recorded with it (none for a record an older build wrote).
    struct Attempts {
        actor: Actor,
        ledger: crate::delivery::ledger::Ledger,
        parent: String,
    }

    impl Attempts {
        fn new() -> Self {
            let actor = Actor {
                pane_id: "factory".into(),
                name: "factory".into(),
                kind: "codex".into(),
                device_id: crate::node::TEST_NODE.into(),
                session: crate::engine::tokens::session_digest("native-factory"),
            };
            let mut ledger = crate::delivery::ledger::Ledger::default();
            let parent = super::super::apply(
                &mut ledger,
                &actor,
                &Mutation::Register {
                    record: AgentRecord {
                        id: String::new(),
                        name: Some("factory".into()),
                        machine: crate::node::TEST_NODE.into(),
                        host_scope: "fixture".into(),
                        native_machine: "fixture-machine".into(),
                        session: "native-factory".into(),
                        instance: "factory-terminal".into(),
                        pane: "factory".into(),
                        parent: None,
                        origin: None,
                        project: None,
                        ended: false,
                        terminal_id: None,
                        forked: false,
                        actor: actor.clone(),
                    },
                    check: false,
                },
                1,
            )
            .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
            Self {
                actor,
                ledger,
                parent,
            }
        }

        fn attempt(
            &mut self,
            name: &str,
            intent: &str,
            pane: &str,
            terminal: Option<&str>,
        ) -> SpawnRecord {
            let reserved = super::super::apply(
                &mut self.ledger,
                &self.actor,
                &Mutation::Reserve {
                    parent: self.parent.clone(),
                    command: Command::Spawn {
                        mission: None,
                        plan_node: None,
                        plan: None,
                        parent: Some(self.parent.clone()),
                        machine: None,
                        name: name.into(),
                        intent: intent.into(),
                        kind: "claude".into(),
                        repo: "/fixture".into(),
                        branch: "factory/801".into(),
                        path: None,
                        args: Vec::new(),
                        launch: Default::default(),
                        task: None,
                        brief: None,
                        beside: false,
                    },
                },
                1,
            )
            .unwrap();
            serde_json::from_value(
                super::super::apply(
                    &mut self.ledger,
                    &self.actor,
                    &Mutation::Advance {
                        seed_tab: None,
                        id: reserved["id"].as_str().unwrap().into(),
                        path: Some("/fixture.worktrees/801".into()),
                        pane: Some(pane.into()),
                        terminal: terminal.map(Into::into),
                        shell: None,
                        child: None,
                    },
                    1,
                )
                .unwrap(),
            )
            .unwrap()
        }
    }

    /// A Herdr whose panes run the given terminals, each with its shell
    /// alone at the prompt unless named busy, and that closes what it is
    /// asked to.
    fn attempt_panes(
        name: &str,
        terminals: &'static [(&'static str, &'static str)],
        busy: &'static [&'static str],
    ) -> FakeHerdr {
        FakeHerdr::start(name, move |method, params| {
            let pane = params["pane_id"].as_str().unwrap().to_owned();
            match method {
                "pane.get" => {
                    let (_, terminal) = terminals
                        .iter()
                        .find(|(id, _)| *id == pane)
                        .expect("a pane the fixture runs");
                    json!({"type": "pane_info", "pane": {"pane_id": pane,
                        "terminal_id": terminal, "workspace_id": "w2", "tab_id": "w2:t1",
                        "focused": false, "agent_status": "idle", "revision": 1}})
                }
                "pane.process_info" => {
                    let foreground = if busy.contains(&pane.as_str()) {
                        4200
                    } else {
                        4100
                    };
                    json!({"type": "pane_process_info", "process_info": {
                        "pane_id": pane, "shell_pid": 4100,
                        "foreground_process_group_id": foreground,
                        "foreground_processes": [{"pid": foreground, "name": "zsh"}]}})
                }
                "pane.close" => json!({"type": "ok"}),
                other => panic!("unexpected {other}"),
            }
        })
    }

    fn closed_panes(herdr: &FakeHerdr) -> Vec<String> {
        herdr
            .calls()
            .into_iter()
            .filter(|(method, _)| method == "pane.close")
            .map(|(_, params)| params["pane_id"].as_str().unwrap().to_owned())
            .collect()
    }

    /// A Factory retry is a new attempt with its own intent and pane; the
    /// earlier attempts of the same parent and name whose agent never
    /// started are closed, so no extra tab and no held name stays behind.
    /// A pane where an agent shows or the shell is busy, another name's
    /// spawn and the attempt's own pane are left alone.
    #[test]
    fn a_new_attempt_closes_the_idle_panes_of_earlier_attempts_that_never_started() {
        let mut attempts = Attempts::new();
        attempts.attempt("worker", "factory-801", "w2:p1", Some("t1"));
        attempts.attempt("worker", "factory-801-a1", "w2:p2", Some("t2"));
        attempts.attempt("worker", "factory-801-a2", "w2:p3", Some("t3"));
        attempts.attempt("other", "factory-857", "w3:p1", Some("t31"));
        let current = attempts.attempt("worker", "factory-801-a3", "w2:p4", Some("t4"));
        // The shell runs a command in w2:p3: that attempt is busy.
        let herdr = attempt_panes(
            "coordination-earlier-attempts",
            &[
                ("w2:p1", "t1"),
                ("w2:p2", "t2"),
                ("w2:p3", "t3"),
                ("w3:p1", "t31"),
            ],
            &["w2:p3"],
        );
        // An agent shows in w2:p2, and Herdr holds the name for the start in
        // w2:p1 that never showed one.
        let observed: Vec<ProjectedAgent> =
            crate::engine::herdr::wire::agents_response(json!({"type": "agent_list", "agents": [
                {"pane_id": "w2:p1", "workspace_id": "w2", "tab_id": "w2:t1",
                    "terminal_id": "t1", "revision": 0, "focused": false,
                    "agent_status": "unknown", "name": "worker", "launch_pending": true},
                {"pane_id": "w2:p2", "workspace_id": "w2", "tab_id": "w2:t2",
                    "terminal_id": "t2", "revision": 0, "focused": false,
                    "agent_status": "idle", "agent": "claude"},
            ]}))
            .unwrap();
        close_unstarted_attempts(
            herdr.engine().as_ref(),
            &attempts.ledger,
            &current,
            &observed,
        );
        assert_eq!(closed_panes(&herdr), ["w2:p1"]);
    }

    /// Herdr hands a pane id out again after its server is replaced, so an
    /// earlier attempt's pane id can name an idle shell the attempt never
    /// made (issue 874). Only a pane still running the terminal the attempt
    /// recorded is its own and closed; a pane id now running another
    /// terminal, and a record an older build wrote without one, are left
    /// open and logged.
    #[test]
    fn an_earlier_attempt_pane_is_closed_only_while_it_runs_the_terminal_the_attempt_recorded() {
        let mut attempts = Attempts::new();
        let kept = attempts.attempt("worker", "factory-801", "w2:p1", Some("t1"));
        let replaced = attempts.attempt("worker", "factory-801-a1", "w2:p2", Some("t2-before"));
        let unrecorded = attempts.attempt("worker", "factory-801-a2", "w2:p3", None);
        let current = attempts.attempt("worker", "factory-801-a3", "w2:p4", Some("t4"));
        let herdr = attempt_panes(
            "coordination-earlier-attempt-identity",
            &[("w2:p1", "t1"), ("w2:p2", "t2-after"), ("w2:p3", "t3")],
            &[],
        );
        let ((), records) = crate::diagnostics::capture(|| {
            close_unstarted_attempts(herdr.engine().as_ref(), &attempts.ledger, &current, &[]);
        });
        assert_eq!(closed_panes(&herdr), ["w2:p1"]);
        let outcome = |attempt: &SpawnRecord| {
            records
                .iter()
                .find(|record| {
                    record["kind"] == "unstarted_attempt.close" && record["attempt"] == attempt.id
                })
                .map(|record| (record["closed"].clone(), record["reason"].clone()))
        };
        assert_eq!(outcome(&kept), Some((json!(true), Value::Null)));
        assert_eq!(
            outcome(&replaced),
            Some((json!(false), json!("pane_replaced")))
        );
        assert_eq!(
            outcome(&unrecorded),
            Some((json!(false), json!("pane_identity_unrecorded")))
        );
    }

    #[test]
    fn completed_spawn_replay_after_restart_preserves_report_stop_end_and_explicit_watch() {
        use crate::delivery::{ledger, mailbox, watch, worker::Worker};
        use crate::handle::ChangeNotifier;
        use crate::runtime::delivery::tests::{authority, fixture};
        use crate::sidebar::SessionSnapshotPayload;
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("agents/runs/hcoord-retire/replay-tests");
        hide_platform::fs::private::create_dir_all(&base).unwrap();
        // The same public command is exercised through the resident store
        // after reload. No native connector exists: replay must return its
        // durable result before attempting another pane or worktree action.
        let _turn = SPAWN_TURN.lock().unwrap_or_else(|e| e.into_inner());
        for ending in ["report", "stop", "end", "explicit"] {
            let root = tempfile::Builder::new()
                .prefix("replay-")
                .tempdir_in(&base)
                .unwrap();
            let (runtime, _, _, path) = fixture(root.path());
            let parent = AgentRecord {
                id: String::new(),
                name: Some("sender".into()),
                machine: crate::node::TEST_NODE.into(),
                host_scope: "fixture".into(),
                native_machine: "fixture-machine".into(),
                session: "native-parent".into(),
                instance: "parent-terminal".into(),
                pane: "sender".into(),
                parent: None,
                origin: None,
                project: None,
                ended: false,
                terminal_id: None,
                forked: false,
                actor: Actor {
                    pane_id: "sender".into(),
                    name: "sender".into(),
                    kind: "codex".into(),
                    device_id: crate::node::TEST_NODE.into(),
                    session: crate::engine::tokens::session_digest("native-parent"),
                },
            };
            let actor = parent.actor.clone();
            let child_actor = Actor {
                pane_id: "recipient".into(),
                name: "recipient".into(),
                kind: "codex".into(),
                device_id: crate::node::TEST_NODE.into(),
                session: crate::engine::tokens::session_digest("native-child"),
            };
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working",
                    "state_change_seq":1,"lineage_session":actor.session},
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working",
                    "state_change_seq":1,"lineage_session":child_actor.session},
            ]}))
            .unwrap();
            runtime.lock().unwrap().observe_delivery_laid(
                crate::node::TEST_NODE,
                &payload,
                None,
                None,
            );
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64;
            let mut state = ledger::Ledger::default();
            let parent_id = super::super::apply(
                &mut state,
                &actor,
                &Mutation::Register {
                    record: parent,
                    check: false,
                },
                now,
            )
            .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
            let command = Command::Spawn {
                mission: None,
                plan_node: None,
                plan: None,
                parent: Some("here".into()),
                machine: None,
                name: "recipient".into(),
                intent: "one-child".into(),
                kind: "codex".into(),
                repo: "/fixture".into(),
                branch: "topic".into(),
                path: None,
                args: Vec::new(),
                launch: Default::default(),
                task: None,
                brief: None,
                beside: false,
            };
            let spawn = super::super::apply(
                &mut state,
                &actor,
                &Mutation::Reserve {
                    parent: parent_id.clone(),
                    command: command.clone(),
                },
                now,
            )
            .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
            super::super::apply(
                &mut state,
                &actor,
                &Mutation::Advance {
                    seed_tab: None,
                    id: spawn.clone(),
                    path: Some("/fixture/topic".into()),
                    pane: Some("recipient".into()),
                    terminal: None,
                    shell: None,
                    child: None,
                },
                now,
            )
            .unwrap();
            let child = AgentRecord {
                id: String::new(),
                name: Some("recipient".into()),
                machine: crate::node::TEST_NODE.into(),
                host_scope: "fixture".into(),
                native_machine: "fixture-machine".into(),
                session: "native-child".into(),
                instance: "child-terminal".into(),
                pane: "recipient".into(),
                parent: Some(parent_id.clone()),
                origin: None,
                project: Some("/fixture/topic".into()),
                actor: child_actor.clone(),
                ended: false,
                terminal_id: None,
                forked: false,
            };
            let child_id = super::super::apply(
                &mut state,
                &actor,
                &Mutation::BindChild {
                    id: spawn.clone(),
                    record: child,
                },
                now,
            )
            .unwrap()["child"]
                .as_str()
                .unwrap()
                .to_owned();
            let receipt =
                super::super::apply(&mut state, &actor, &Mutation::Complete { id: spawn }, now)
                    .unwrap();
            let initial_watch = receipt["auto_watch_id"].as_str().unwrap();
            match ending {
                "end" => {
                    super::super::apply(
                        &mut state,
                        &actor,
                        &Mutation::End {
                            id: child_id.clone(),
                            actor: Some(parent_id),
                        },
                        now,
                    )
                    .unwrap();
                }
                "stop" => watch::stop(&mut state, &actor, initial_watch).unwrap(),
                _ => {
                    let report = mailbox::send(
                        &mut state,
                        &child_actor,
                        &actor,
                        "done",
                        "completed",
                        "report",
                        None,
                        now,
                    )
                    .unwrap();
                    mailbox::apply(
                        &mut state,
                        &actor,
                        None,
                        &mailbox::Command::Confirm {
                            ids: vec![report.id],
                        },
                        now,
                    )
                    .unwrap();
                    if ending == "explicit" {
                        let watch = watch::start(&mut state, &actor, &child_actor, now).unwrap();
                        assert_ne!(watch.id, initial_watch);
                    }
                }
            }
            ledger::save(&path, &state).unwrap();
            let restored = ledger::load(&path).unwrap();
            let expected_watch = restored.watches.first().map(|watch| watch.id.clone());
            let next_id = restored.next_id;
            runtime
                .lock()
                .unwrap()
                .publish_delivery(Arc::new(restored), false);
            let (worker, client) = Worker::spawn(
                Arc::downgrade(&runtime),
                ChangeNotifier::noop(),
                path.clone(),
            )
            .unwrap();
            let result = run(client, authority(&actor), actor, command).unwrap();
            assert_eq!(result["id"], child_id, "{ending}");
            assert_eq!(result["registered"], ending != "end", "{ending}");
            assert_eq!(
                result["watch"]["id"].as_str(),
                expected_watch.as_deref(),
                "{ending}"
            );
            drop(worker);
            let after = ledger::load(&path).unwrap();
            assert_eq!(after.agents.len(), 2, "{ending}");
            assert_eq!(after.next_id, next_id, "{ending}");
            assert!(after.spawns[0].completed);
            assert_eq!(
                after.spawns[0].auto_watch_id.as_deref(),
                Some(initial_watch)
            );
            assert_eq!(
                after
                    .agents
                    .iter()
                    .find(|record| record.id == child_id)
                    .unwrap()
                    .ended,
                ending == "end",
                "{ending}"
            );
            assert_eq!(
                after.watches.first().map(|watch| &watch.id),
                expected_watch.as_ref(),
                "{ending}"
            );
        }
    }
}
