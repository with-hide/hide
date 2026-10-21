use super::{AgentRecord, Command, Mutation, SpawnRecord, resolve_actor, view};
use crate::agent_start::ShellStart;
use crate::delivery::{
    Actor,
    worker::{Authority, Client, Effect},
};
use crate::runtime::delivery::CoordinationContext;
use crate::session_sync::ProjectedAgent;
use hide_herdr_client::{ApiConnector, ApiError, request_with_connector};
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
fn worktree_create_refusal(error: ApiError) -> String {
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
fn agents(connector: &dyn ApiConnector) -> Result<Vec<ProjectedAgent>, String> {
    agents_with_timeout(connector, Duration::from_secs(2))
}
fn agents_with_timeout(
    connector: &dyn ApiConnector,
    timeout: Duration,
) -> Result<Vec<ProjectedAgent>, String> {
    let result = request_with_connector(
        connector,
        "agent.list",
        crate::wire::empty_params(),
        timeout,
    )
    .map_err(|error| format!("{error}"))?;
    crate::wire::agents_response(result).map_err(|_| "native_identity_unavailable".into())
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
    connector: &dyn ApiConnector,
    pane: &str,
    name: Option<&str>,
    kind: &str,
) -> Result<ProjectedAgent, String> {
    // The public socket start acknowledges the typed command before the
    // process and its native session are detected. Wait for that observation,
    // never send another start, and keep every read within one finite deadline.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("native_identity_unavailable".into());
        }
        let observed = agents_with_timeout(connector, remaining.min(Duration::from_secs(2)))?;
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
/// The actor of `agent` on `device`; `on_node` when that device is the core's
/// own node, whose panes the core names without a device prefix.
fn actor_for(agent: &ProjectedAgent, device: &str, on_node: bool) -> Result<Actor, String> {
    let actor = Actor {
        pane_id: if on_node {
            agent.pane_id.clone()
        } else {
            format!("remote:{device}:pane:{}", agent.pane_id)
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
            let ledger = state(&client)?;
            Ok(json!(crate::delivery::answer::AgentList {
                items: ledger
                    .agents
                    .iter()
                    .map(|record| super::agent_view(record, &ledger))
                    .collect(),
            }))
        }
        Command::Show { id } if id == super::HERE => {
            let ledger = state(&client)?;
            Ok(view(super::here(&ledger, &actor)?, &ledger))
        }
        Command::Show { id } => {
            let ledger = state(&client)?;
            let record = ledger
                .agents
                .iter()
                .find(|record| record.id == id)
                .ok_or("agent_unavailable")?;
            Ok(view(record, &ledger))
        }
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
                connector,
                host_scope: actual_scope,
                machine: native_machine,
                on_node,
                ..
            } = context(&client, &machine)?;
            if on_node && host_scope != actual_scope {
                return Err("host_scope_conflict".into());
            }
            let observed = agents(connector.as_ref())?;
            let native = observed
                .iter()
                .find(|agent| agent.pane_id == pane)
                .ok_or("target_unavailable")?;
            if native.lineage_session != crate::wire::session_digest(&session) {
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
                    connector.as_ref(),
                    value["id"].as_str().ok_or("agent_unavailable")?,
                )?;
            }
            Ok(value)
        }
        command @ Command::Spawn { .. } => spawn(&client, &authority, &actor, command),
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
pub(super) fn publish_tokens(
    client: &Client,
    connector: &dyn ApiConnector,
    id: &str,
) -> Result<(), String> {
    let ledger = state(client)?;
    let child = ledger
        .agents
        .iter()
        .find(|record| record.id == id)
        .ok_or("agent_unavailable")?;
    if let Some(related) = super::lineage::related(child, &ledger) {
        super::lineage::write_record(connector, child, related)?;
    }
    Ok(())
}
/// Registers a code-owned identity, a Factory's or Hide's own, as the
/// parent or origin of the agents it starts (D-14); converges on the
/// existing record.
pub(crate) fn register_code_owned(
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
/// agent CLI is installed there. The node link is taken under the runtime
/// lock and called after it is released.
fn check_target(
    client: &Client,
    actor: &Actor,
    device: &str,
    kind: &str,
    intent: &str,
    repository: Option<&str>,
    agent: bool,
) -> Result<(), String> {
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
        return Ok(());
    }
    match call_as::<bool>(
        link.as_ref(),
        Call::AgentInstalled {
            name: hide_agent_adapter::canonical_kind(kind).to_owned(),
        },
        TARGET_CHECK_TIMEOUT,
    ) {
        Ok(true) => Ok(()),
        Ok(false) => Err("agent_not_installed".into()),
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
        mission: _,
    } = &command
    else {
        return Err("invalid_spawn".into());
    };
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
    // first prompt after `--`.
    let mut start_args =
        crate::runtime::agent_choice::launch_arguments(kind, launch).map_err(|detail| {
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
            check_target(
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
        } else {
            let CoordinationContext {
                connector,
                host_scope,
                machine: native_machine,
                on_node,
                ..
            } = context(client, &actor.device_id)?;
            let observed = agents(connector.as_ref())?;
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
        let ledger = state(client)?;
        return Ok(view(
            ledger
                .agents
                .iter()
                .find(|record| Some(&record.id) == reserved.child.as_ref())
                .ok_or("child_unavailable")?,
            &ledger,
        ));
    }
    // Everything the child needs happens on its own device; only the parent's
    // authority and watch stay with the caller.
    let child_device = reserved
        .machine
        .clone()
        .unwrap_or_else(|| actor.device_id.clone());
    let CoordinationContext {
        connector,
        host_scope,
        machine: native_machine,
        codex: codex_daemon,
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
            // How the pane's shell says it has finished starting, recorded
            // with the pane so every later start in it waits the same way.
            let shell;
            // Reconcile a worktree whose creation reply was interrupted. The
            // existing local/device connector and checkout owner are shared with
            // the product's worktree/start path; no spawn-specific SSH exists.
            let listed = request_with_connector(
                connector.as_ref(),
                "worktree.list",
                crate::wire::worktree_list_params(repo)?,
                Duration::from_secs(5),
            )
            .map_err(|error| format!("{error}"))?;
            let existing = crate::wire::listed_worktree_path(listed, branch)?;
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
                let snapshot = request_with_connector(
                    connector.as_ref(),
                    "session.snapshot",
                    crate::wire::empty_params(),
                    Duration::from_secs(5),
                )
                .map_err(|error| format!("{error}"))?;
                let snapshot =
                    crate::wire::snapshot_response(snapshot).map_err(|_| "snapshot_unavailable")?;
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
                let launch = crate::agent_start::shell_launch(node.as_deref(), &reserved.id);
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
                let pane = match existing_pane {
                    Some((pane, launched)) => {
                        shell = ShellStart::of_tab(launched, launch.as_ref());
                        pane
                    }
                    None => {
                        let tab = crate::live::open_owner_tab(
                            connector.as_ref(),
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
            } else {
                let mut params = crate::wire::worktree_create_params(repo, branch, None, false)?;
                params["label"] = json!(format!("hide:{}", reserved.id));
                if let Some(path) = path {
                    params["path"] = json!(path);
                }
                let result = request_with_connector(
                    connector.as_ref(),
                    "worktree.create",
                    params,
                    WORKTREE_CREATE_TIMEOUT,
                )
                .map_err(worktree_create_refusal)?;
                let created = crate::wire::created_worktree(result)?;
                let launch = crate::agent_start::shell_launch(node.as_deref(), &reserved.id);
                let tab = crate::live::replace_tab(
                    connector.as_ref(),
                    crate::live::OwnedTab {
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
            let terminal = pane_terminal(connector.as_ref(), &pane)?;
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
        }
        let pane = reserved.pane.clone().ok_or("spawn_unavailable")?;
        let pane = pane.as_str();
        let shell = ShellStart::launched(reserved.shell.clone());
        let observed = agents(connector.as_ref())?;
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
                if held && !name_released(connector.as_ref(), name) {
                    return Err("native_identity_unavailable".into());
                }
                // Herdr gave that start up. Only a shell back at rest, with
                // nothing of its own in the foreground, takes the start again,
                // and only once: a start typed into a question at startup is
                // typed again at the prompt that followed, and a second one
                // that ends the same way is the answer.
                if reserved.typed >= 2 || shell_alone(connector.as_ref(), pane) != Ok(true) {
                    return Err("agent_not_started".into());
                }
                crate::diagnostic!(json!({
                    "component": "spawn", "kind": "start.resent", "spawn": reserved.id,
                    "pane_id": pane,
                }));
            }
            close_unstarted_attempts(connector.as_ref(), &*state(client)?, &reserved, &observed);
            crate::live::start_agent(
                connector.as_ref(),
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
        // The child's pane keeps its launch choice for the restarts Hide
        // makes of it, under the id the core knows the pane by and the name
        // it was started under. It is kept before the agent shows its
        // session, which an agent that reports it only at its first turn
        // does later, so the record binds to that session when it arrives.
        if let Some(runtime) = client.runtime.upgrade() {
            let pane_id = if on_node {
                pane.to_owned()
            } else {
                crate::session_sync::remote_pane_id(&child_device, pane)
            };
            runtime
                .lock()
                .map_err(|_| "delivery_unavailable")?
                .record_pane_launch(&pane_id, kind, name, &reserved.launch, None);
        }
        let native = match wait_native_identity(connector.as_ref(), pane, Some(name), kind) {
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
            close_refused_pane(connector.as_ref(), &reserved, pane, reason);
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
        publish_tokens(client, connector.as_ref(), &child_id)?;
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
        let ledger = state(client)?;
        Ok(view(
            ledger
                .agents
                .iter()
                .find(|record| record.id == child_id)
                .ok_or("child_unavailable")?,
            &ledger,
        ))
    })();
    // Nothing after this creates anything, so a device that has gone quiet
    // never holds the lock for the probe below.
    drop(single);
    placed.map_err(|reason| {
        if on_device {
            device_unreachable(connector.as_ref(), &child_device, &spawn_id, reason)
        } else {
            reason
        }
    })
}

/// Whether Herdr has let go of `name`. Herdr holds the name of a start whose
/// agent never showed until that start's own deadline, and settles a passed
/// deadline only when the name is read, which this `agent.get` is.
fn name_released(connector: &dyn ApiConnector, name: &str) -> bool {
    crate::wire::agent_target_params(name)
        .map_err(hide_herdr_client::ApiError::Transport)
        .and_then(|params| {
            request_with_connector(connector, "agent.get", params, Duration::from_secs(2))
        })
        .is_err_and(|error| {
            matches!(error, hide_herdr_client::ApiError::Remote { code, .. } if code == "agent_not_found")
        })
}

/// Whether `pane`'s shell alone holds its terminal: no program it started
/// runs in the foreground.
fn shell_alone(connector: &dyn ApiConnector, pane: &str) -> Result<bool, String> {
    crate::wire::pane_process_info_params(pane)
        .and_then(|params| {
            request_with_connector(
                connector,
                "pane.process_info",
                params,
                Duration::from_secs(2),
            )
            .map_err(|error| error.to_string())
        })
        .and_then(crate::wire::pane_process_group)
        .map(|group| group.shell_holds_terminal())
}

/// The terminal Herdr runs in `pane` now.
fn pane_terminal(connector: &dyn ApiConnector, pane: &str) -> Result<String, String> {
    crate::wire::pane_target_params(pane)
        .and_then(|params| {
            request_with_connector(connector, "pane.get", params, Duration::from_secs(5))
                .map_err(|error| error.to_string())
        })
        .and_then(crate::wire::pane_terminal)
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
    connector: &dyn ApiConnector,
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
        let closed = match same_terminal(connector, record, pane)
            .and_then(|()| shell_alone(connector, pane))
        {
            Ok(false) => continue,
            Ok(true) => close_pane(connector, pane),
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
    connector: &dyn ApiConnector,
    reserved: &SpawnRecord,
    pane: &str,
    reason: &str,
) {
    let closed =
        same_terminal(connector, reserved, pane).and_then(|()| close_pane(connector, pane));
    crate::diagnostic!(json!({
        "component": "spawn", "kind": "refused_pane.close", "spawn": reserved.id,
        "pane_id": pane, "refusal": reason, "closed": closed.is_ok(),
        "reason": closed.err(),
    }));
}

/// Whether `pane` is still the terminal `record` recorded with it, so a pane
/// Herdr gave the same id afterwards is never taken for the spawn's.
fn same_terminal(
    connector: &dyn ApiConnector,
    record: &SpawnRecord,
    pane: &str,
) -> Result<(), String> {
    let recorded = record
        .terminal
        .as_deref()
        .ok_or("pane_identity_unrecorded")?;
    if pane_terminal(connector, pane)? == recorded {
        Ok(())
    } else {
        Err("pane_replaced".to_owned())
    }
}

fn close_pane(connector: &dyn ApiConnector, pane: &str) -> Result<(), String> {
    let params = crate::wire::pane_target_params(pane)?;
    request_with_connector(connector, "pane.close", params, Duration::from_secs(5))
        .map(drop)
        .map_err(|error| error.to_string())
}

/// A failure of a spawn's work on a device, as the caller should read it: the
/// device is unavailable when its Herdr no longer answers, whatever step was
/// running; any other failure keeps its own code. The dropped cause is logged
/// with the device and the spawn it belongs to.
fn device_unreachable(
    connector: &dyn ApiConnector,
    device: &str,
    spawn: &str,
    reason: String,
) -> String {
    match request_with_connector(
        connector,
        "agent.list",
        crate::wire::empty_params(),
        Duration::from_secs(2),
    ) {
        Err(
            error @ (hide_herdr_client::ApiError::NotRunning(_)
            | hide_herdr_client::ApiError::Transport(_)),
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
        connector,
        host_scope,
        machine: native_machine,
        on_node,
        ..
    } = self::context(&client, &actor.device_id)?;
    let observed = agents(connector.as_ref())?;
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
        connector,
        host_scope,
        machine: native_machine,
        on_node,
        ..
    } = self::context(&client, &actor.device_id)?;
    let observed = agents_with_timeout(connector.as_ref(), within)?;
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

#[cfg(test)]
mod tests {
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
                worktree_create_refusal(ApiError::Transport(lost.into())),
                WORKTREE_UNCONFIRMED,
                "{lost}"
            );
        }
        assert_eq!(
            worktree_create_refusal(ApiError::Transport("connect failed for /s: refused".into())),
            "connect failed for /s: refused"
        );
        assert_eq!(
            worktree_create_refusal(ApiError::Remote {
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
        let native = wait_native_identity(&herdr.connector(), "w9J:p9R", None, "claude").unwrap();
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
                wait_native_identity(&herdr.connector(), "w9J:p9R", Some("child"), "pi").unwrap();
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
            let child = wait_native_identity(&herdr.connector(), "w2:p1", Some("child"), requested)
                .unwrap();
            assert_eq!(child.agent.as_deref(), Some(reported));
            assert_eq!(
                child.lineage_session,
                crate::wire::session_digest("fixture-child-session")
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
        let child =
            wait_native_identity(&herdr.connector(), "w2:p1", Some("child"), "claude").unwrap();
        assert_eq!(
            child.lineage_session,
            crate::wire::session_digest("fixture-child-session")
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
                session: crate::wire::session_digest("native-factory"),
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
            crate::wire::agents_response(json!({"type": "agent_list", "agents": [
                {"pane_id": "w2:p1", "workspace_id": "w2", "tab_id": "w2:t1",
                    "terminal_id": "t1", "revision": 0, "focused": false,
                    "agent_status": "unknown", "name": "worker", "launch_pending": true},
                {"pane_id": "w2:p2", "workspace_id": "w2", "tab_id": "w2:t2",
                    "terminal_id": "t2", "revision": 0, "focused": false,
                    "agent_status": "idle", "agent": "claude"},
            ]}))
            .unwrap();
        close_unstarted_attempts(&herdr.connector(), &attempts.ledger, &current, &observed);
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
            close_unstarted_attempts(&herdr.connector(), &attempts.ledger, &current, &[]);
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
                    session: crate::wire::session_digest("native-parent"),
                },
            };
            let actor = parent.actor.clone();
            let child_actor = Actor {
                pane_id: "recipient".into(),
                name: "recipient".into(),
                kind: "codex".into(),
                device_id: crate::node::TEST_NODE.into(),
                session: crate::wire::session_digest("native-child"),
            };
            let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
                {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working",
                    "state_change_seq":1,"lineage_session":actor.session},
                {"id":"recipient","pane_id":"recipient","agent":"codex","agent_status":"working",
                    "state_change_seq":1,"lineage_session":child_actor.session},
            ]}))
            .unwrap();
            runtime
                .lock()
                .unwrap()
                .observe_delivery(crate::node::TEST_NODE, &payload, None, None);
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
