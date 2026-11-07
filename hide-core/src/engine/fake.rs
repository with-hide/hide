//! A stand-in engine for tests (PRD engine-bundle-4-herdr-removal D-15).
//!
//! Every [`Engine`] call becomes one named call, the method's own name, with
//! its arguments as JSON, and the test's responder answers it with the
//! method's typed answer ([`reply`]) or an [`EngineError`]. The calls are
//! kept in order, so a test can say what the core asked and how often. The
//! session reads (`session_payload`, `pane_folders`, `pane_places`,
//! `agents`) are made from the one `session` answer, as the core's own
//! engine makes them, so a test answers `session` alone; [`session`] reads
//! a session written as JSON in [`ProjectionState`]'s own field names.
//!
//! ```ignore
//! let engine = FakeEngine::start(|method, args| match method {
//!     "session" => reply(session(json!({"workspaces": [...]}))),
//!     "rename_tab" => reply(()),
//!     other => panic!("unexpected {other} {args}"),
//! });
//! rename(engine.engine().as_ref(), "w1:t1", "work");
//! assert_eq!(engine.methods(), ["rename_tab"]);
//! ```
//!
//! A responder that panics fails the calling test on the thread that made
//! the call; an answer of the wrong type names the method and the type it
//! wanted.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

use super::*;

/// One answer: the method's typed answer, boxed, or the engine's refusal.
pub(crate) type Reply = Result<Box<dyn Any + Send>, EngineError>;

/// The method's answer `value`.
pub(crate) fn reply<T: Any + Send>(value: T) -> Reply {
    Ok(Box::new(value))
}

/// The engine's refusal under `code`.
pub(crate) fn refused(code: &str, message: &str) -> Reply {
    Err(EngineError::Refused {
        code: code.to_owned(),
        message: message.to_owned(),
    })
}

/// A call that was sent and never answered.
pub(crate) fn unanswered(message: &str) -> Reply {
    Err(EngineError::Unanswered(message.to_owned()))
}

/// A session written as JSON in [`ProjectionState`]'s field names, alone or
/// under `snapshot`; a field left out is empty and one it does not name is
/// left out. An agent's lineage fields it leaves out are read from its
/// session and its pane's tokens, as the core's own engine reads them.
pub(crate) fn session(mut value: Value) -> ProjectionState {
    if value.get("snapshot").is_some_and(Value::is_object) {
        value = value["snapshot"].take();
    }
    let mut state: ProjectionState =
        serde_json::from_value(value).expect("the fixture session is a ProjectionState");
    for agent in &mut state.agents {
        let session = agent
            .agent_session
            .as_ref()
            .map(|session| session.value.as_str());
        let lineage = crate::engine::tokens::lineage(&agent.tokens, session);
        agent.lineage_session = agent.lineage_session.take().or(lineage.own_session);
        agent.spawned_from_pane_id = agent
            .spawned_from_pane_id
            .take()
            .or(lineage.spawned_from_pane_id);
        agent.spawned_from_machine_id = agent
            .spawned_from_machine_id
            .take()
            .or(lineage.spawned_from_machine_id);
        agent.declared_parent_session = agent
            .declared_parent_session
            .take()
            .or(lineage.declared_parent_session);
    }
    state
}

/// A tab's layout written as JSON in [`ClosedLayout`]'s field names, alone
/// or under `layout`.
pub(crate) fn closed_layout(mut value: Value) -> ClosedLayout {
    if value.get("layout").is_some_and(Value::is_object) {
        value = value["layout"].take();
    }
    serde_json::from_value(value).expect("the fixture layout is a ClosedLayout")
}

/// A pane's processes written as JSON (`shell_pid`,
/// `foreground_process_group_id`, `foreground_processes: [{pid, name}]`),
/// alone or under `process_info`.
pub(crate) fn processes(value: &Value) -> PaneProcessGroup {
    let value = value.get("process_info").unwrap_or(value);
    let pid = |value: &Value| value.as_u64().map(|pid| pid as u32);
    let foreground = value["foreground_processes"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    PaneProcessGroup {
        shell_pid: pid(&value["shell_pid"]),
        foreground_process_group_id: pid(&value["foreground_process_group_id"]),
        foreground_pids: foreground
            .iter()
            .filter_map(|process| pid(&process["pid"]))
            .collect(),
        foreground_names: foreground
            .iter()
            .map(|process| process["name"].as_str().unwrap_or_default().to_owned())
            .collect(),
    }
}

/// An agent written as JSON (`pane_id`, `name`, `agent`, `agent_status`,
/// `state_change_seq`, `agent_session.value`, `interactive_ready`,
/// `launch_pending`) as the delivery check reads it.
pub(crate) fn delivery_agent(agent: &Value) -> DeliveryAgent {
    let text = |key: &str| agent[key].as_str().map(str::to_owned);
    let pane_id = text("pane_id").expect("the fixture agent names its pane");
    DeliveryAgent {
        name: text("name").unwrap_or_else(|| pane_id.clone()),
        pane_id,
        kind: text("agent"),
        session: agent["agent_session"]["value"]
            .as_str()
            .and_then(tokens::session_digest),
        status: text("agent_status").unwrap_or_else(|| "unknown".to_owned()),
        state_change_seq: agent["state_change_seq"].as_u64().unwrap_or_default(),
        readiness: readiness(
            agent["interactive_ready"].as_bool(),
            agent["launch_pending"].as_bool(),
        ),
    }
}

/// A fixture agent's readiness from its Herdr-shaped flags: launching while
/// `launch_pending`, not ready when `interactive_ready` is false, and ready
/// otherwise, including a fixture that names neither.
fn readiness(interactive_ready: Option<bool>, launch_pending: Option<bool>) -> Readiness {
    match (interactive_ready, launch_pending) {
        (_, Some(true)) => Readiness::Launching,
        (Some(false), _) => Readiness::NotReady,
        _ => Readiness::Ready,
    }
}

/// The same agent as the check agent sleep makes reads it.
pub(crate) fn agent_state(agent: &Value) -> AgentState {
    AgentState {
        agent: agent["agent"].as_str().map(str::to_owned),
        status: agent["agent_status"]
            .as_str()
            .unwrap_or("unknown")
            .to_owned(),
        state_change_seq: agent["state_change_seq"].as_u64().unwrap_or_default(),
    }
}

type Respond = dyn FnMut(&'static str, &Value) -> Reply + Send;

struct Shared {
    scope: std::path::PathBuf,
    respond: Mutex<Box<Respond>>,
    calls: Mutex<Vec<(&'static str, Value)>>,
    arrived: std::sync::Condvar,
    wake: Mutex<Option<Wake>>,
    /// The definitions the core handed this engine, as a real engine keeps
    /// them; the built-in ones until it does.
    definitions: Mutex<Option<Arc<hide_agent_adapter::definition::Catalog>>>,
}

/// The stand-in engine; [`FakeEngine::engine`] hands the core its
/// [`SharedEngine`](super::SharedEngine).
pub(crate) struct FakeEngine<'g> {
    shared: Arc<Shared>,
    gates: Vec<Gate<'g>>,
}

impl FakeEngine<'static> {
    /// An engine whose every call `respond` answers.
    pub(crate) fn start(
        respond: impl FnMut(&'static str, &Value) -> Reply + Send + 'static,
    ) -> Arc<Self> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let number = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Arc::new(Self {
            shared: Arc::new(Shared {
                scope: std::env::temp_dir()
                    .join(format!("hide-fake-engine-{}-{number}", std::process::id())),
                respond: Mutex::new(Box::new(respond)),
                calls: Mutex::default(),
                arrived: std::sync::Condvar::new(),
                wake: Mutex::default(),
                definitions: Mutex::default(),
            }),
            gates: Vec::new(),
        })
    }

    /// An engine that is not running: every call fails as a call to a
    /// stopped engine does.
    pub(crate) fn absent() -> Arc<Self> {
        Self::start(|method, _| Err(EngineError::NotRunning(format!("{method}: no engine"))))
    }

    /// A live context on this engine for a runtime under test: its workers
    /// cannot report back, and its node is this process.
    pub(crate) fn live(self: &Arc<Self>) -> crate::live::LiveContext {
        crate::live::LiveContext {
            socket_path: self.scope(),
            runtime: std::sync::Weak::new(),
            notifier: crate::handle::ChangeNotifier::noop(),
            engine: self.engine(),
            node: Arc::new(hide_node::Local::of_process()),
        }
    }

    /// The engine as the core holds it.
    pub(crate) fn engine(self: &Arc<Self>) -> super::SharedEngine {
        Arc::clone(self) as super::SharedEngine
    }
}

impl FakeEngine<'_> {
    /// The folder that names this engine where the core keys something by
    /// its engine, as a machine's engine folder does; nothing is made there.
    pub(crate) fn scope(&self) -> std::path::PathBuf {
        self.shared.scope.clone()
    }

    /// Every call so far, in order, with its arguments.
    pub(crate) fn calls(&self) -> Vec<(&'static str, Value)> {
        lock(&self.shared.calls).clone()
    }

    /// The name of every call so far, in order.
    pub(crate) fn methods(&self) -> Vec<&'static str> {
        lock(&self.shared.calls)
            .iter()
            .map(|(method, _)| *method)
            .collect()
    }

    /// The arguments of every call of `method`, in order.
    pub(crate) fn calls_of(&self, method: &str) -> Vec<Value> {
        lock(&self.shared.calls)
            .iter()
            .filter(|(name, _)| *name == method)
            .map(|(_, args)| args.clone())
            .collect()
    }

    /// Waits until `count` calls have arrived, from whichever thread makes
    /// them; panics after `timeout`, naming the calls that did arrive.
    pub(crate) fn wait_for_calls(&self, count: usize, timeout: std::time::Duration) {
        let calls = lock(&self.shared.calls);
        let (calls, waited) = self
            .shared
            .arrived
            .wait_timeout_while(calls, timeout, |calls| calls.len() < count)
            .unwrap_or_else(PoisonError::into_inner);
        assert!(
            !waited.timed_out(),
            "{count} engine calls did not arrive within {timeout:?}: {:?}",
            calls.iter().map(|(method, _)| *method).collect::<Vec<_>>()
        );
    }

    fn check(&self) -> Result<(), EngineError> {
        for gate in &self.gates {
            (gate.before)()?;
        }
        for gate in &self.gates {
            (gate.at_send)()?;
        }
        Ok(())
    }

    fn ask<T: Any>(&self, method: &'static str, args: Value) -> Answer<T> {
        self.check()?;
        lock(&self.shared.calls).push((method, args.clone()));
        self.shared.arrived.notify_all();
        let answer = (lock(&self.shared.respond))(method, &args)?;
        match answer.downcast::<T>() {
            Ok(answer) => Ok(*answer),
            Err(_) => panic!(
                "the fake engine's answer to {method} is not a {}",
                std::any::type_name::<T>()
            ),
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn debug(value: &impl std::fmt::Debug) -> Value {
    Value::String(format!("{value:?}"))
}

impl<'g> Engine for FakeEngine<'g> {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn gated<'a>(&'a self, gate: Gate<'a>) -> Box<dyn Engine + 'a> {
        let mut gates: Vec<Gate<'a>> = self.gates.clone();
        gates.push(gate);
        Box::new(FakeEngine {
            shared: Arc::clone(&self.shared),
            gates,
        })
    }

    fn changes(&self, wake: Wake) {
        *lock(&self.shared.wake) = Some(wake);
    }

    fn set_agent_definitions(&self, catalog: Arc<hide_agent_adapter::definition::Catalog>) {
        *lock(&self.shared.definitions) = Some(catalog);
    }

    fn start_program_name(&self, kind: &str) -> Option<String> {
        let held = lock(&self.shared.definitions).clone();
        let definitions = held
            .as_ref()
            .map_or(hide_agent_adapter::definition::builtins(), |catalog| {
                catalog.definitions.as_slice()
            });
        hide_agent_adapter::definition::find(definitions, kind)
            .and_then(|definition| definition.executables.first().cloned())
    }

    fn hook_session(&self, pane: &str, kind: &str, session: &str) -> Answer<bool> {
        self.ask(
            "hook_session",
            json!({"pane": pane, "kind": kind, "session": session}),
        )
    }

    fn session(&self, _wait: Wait) -> Answer<ProjectionState> {
        self.ask("session", json!({}))
    }

    fn session_payload(&self, wait: Wait) -> Answer<SessionSnapshotPayload> {
        Ok(self.session(wait)?.project())
    }

    fn pane_folders(&self, wait: Wait) -> Answer<Vec<(String, Option<String>)>> {
        Ok(self
            .session(wait)?
            .panes
            .into_iter()
            .map(|pane| (pane.pane_id, pane.cwd))
            .collect())
    }

    fn pane_places(&self, wait: Wait) -> Answer<Vec<PanePlace>> {
        Ok(self
            .session(wait)?
            .panes
            .into_iter()
            .map(|pane| PanePlace {
                pane_id: pane.pane_id,
                cwd: pane.cwd,
                foreground_cwd: None,
            })
            .collect())
    }

    fn agents(&self, wait: Wait) -> Answer<Vec<ProjectedAgent>> {
        Ok(self.session(wait)?.agents)
    }

    fn ping(&self, _wait: Wait) -> Answer<ServerPong> {
        self.ask("ping", json!({}))
    }

    fn create_workspace(
        &self,
        _wait: Wait,
        cwd: &str,
        label: &str,
        focus: bool,
        env: Env,
    ) -> Answer<(String, String, String)> {
        self.ask(
            "create_workspace",
            json!({"cwd": cwd, "label": label, "focus": focus, "env": env}),
        )
    }

    fn workspace_active_tab(&self, _wait: Wait, workspace: &str) -> Answer<String> {
        self.ask("workspace_active_tab", json!({"workspace": workspace}))
    }

    fn workspace_tokens(&self, _wait: Wait, token: &str) -> Answer<Vec<(String, Option<String>)>> {
        self.ask("workspace_tokens", json!({"token": token}))
    }

    fn rename_workspace(&self, _wait: Wait, workspace: &str, label: &str) -> Answer<()> {
        self.ask(
            "rename_workspace",
            json!({"workspace": workspace, "label": label}),
        )
    }

    fn focus_workspace(&self, _wait: Wait, workspace: &str) -> Answer<()> {
        self.ask("focus_workspace", json!({"workspace": workspace}))
    }

    fn report_workspace_token(
        &self,
        _wait: Wait,
        workspace: &str,
        token: &str,
        value: Option<&str>,
    ) -> Answer<()> {
        self.ask(
            "report_workspace_token",
            json!({"workspace": workspace, "token": token, "value": value}),
        )
    }

    fn create_tab(
        &self,
        _wait: Wait,
        workspace: &str,
        cwd: &str,
        label: &str,
        focus: bool,
        env: Env,
    ) -> Answer<(String, String)> {
        self.ask(
            "create_tab",
            json!({"workspace": workspace, "cwd": cwd, "label": label, "focus": focus, "env": env}),
        )
    }

    fn launch_tab(
        &self,
        _wait: Wait,
        workspace: &str,
        replace: Option<&str>,
        label: &str,
        focus: bool,
        cwd: &str,
        env: Env,
        command: &[String],
    ) -> Answer<(String, String)> {
        self.ask(
            "launch_tab",
            json!({"workspace": workspace, "replace": replace, "label": label, "focus": focus,
                   "cwd": cwd, "env": env, "command": command}),
        )
    }

    fn launch_checkout_tab(
        &self,
        _wait: Wait,
        workspace: &str,
        seed_tab: Option<&str>,
        cwd: &str,
        label: &str,
        env: Env,
        command: Option<&[String]>,
        claim: &str,
    ) -> Answer<(String, String)> {
        self.ask(
            "launch_checkout_tab",
            json!({"workspace": workspace, "seed_tab": seed_tab, "cwd": cwd, "label": label,
                   "env": env, "command": command, "claim": claim}),
        )
    }

    fn close_tab(&self, _wait: Wait, tab: &str) -> Answer<()> {
        self.ask("close_tab", json!({"tab": tab}))
    }

    fn focus_tab(&self, _wait: Wait, tab: &str) -> Answer<()> {
        self.ask("focus_tab", json!({"tab": tab}))
    }

    fn rename_tab(&self, _wait: Wait, tab: &str, label: &str) -> Answer<()> {
        self.ask("rename_tab", json!({"tab": tab, "label": label}))
    }

    fn move_tab(&self, _wait: Wait, tab: &str, index: usize) -> Answer<Vec<String>> {
        self.ask("move_tab", json!({"tab": tab, "index": index}))
    }

    fn pane_layout(&self, _wait: Wait, pane: &str) -> Answer<SessionLayoutPayload> {
        self.ask("pane_layout", json!({"pane": pane}))
    }

    fn export_layout(&self, _wait: Wait, tab: &str) -> Answer<ClosedLayout> {
        self.ask("export_layout", json!({"tab": tab}))
    }

    fn apply_layout(
        &self,
        _wait: Wait,
        workspace: &str,
        tab: Option<&str>,
        tab_label: Option<&str>,
        root: &ClosedLayoutNode,
    ) -> Answer<ClosedLayout> {
        self.ask(
            "apply_layout",
            json!({"workspace": workspace, "tab": tab, "tab_label": tab_label, "root": root}),
        )
    }

    fn split_pane(
        &self,
        _wait: Wait,
        pane: &str,
        direction: crate::live::PaneSplitDirection,
        cwd: Option<&str>,
    ) -> Answer<String> {
        self.ask(
            "split_pane",
            json!({"pane": pane, "direction": debug(&direction), "cwd": cwd}),
        )
    }

    fn split_pane_at(
        &self,
        _wait: Wait,
        pane: &str,
        direction: ClosedSplitDirection,
        cwd: &str,
        ratio: f32,
        env: Env,
    ) -> Answer<String> {
        self.ask(
            "split_pane_at",
            json!({"pane": pane, "direction": direction, "cwd": cwd, "ratio": ratio, "env": env}),
        )
    }

    fn split_beside(
        &self,
        _wait: Wait,
        parent: &str,
        cwd: Option<&str>,
        env: Env,
    ) -> Answer<String> {
        self.ask(
            "split_beside",
            json!({"parent": parent, "cwd": cwd, "env": env}),
        )
    }

    fn split_replacement(&self, _wait: Wait, pane: &str, cwd: &str, env: Env) -> Answer<String> {
        self.ask(
            "split_replacement",
            json!({"pane": pane, "cwd": cwd, "env": env}),
        )
    }

    fn close_pane(&self, _wait: Wait, pane: &str) -> Answer<()> {
        self.ask("close_pane", json!({"pane": pane}))
    }

    fn focus_pane(&self, _wait: Wait, pane: &str) -> Answer<()> {
        self.ask("focus_pane", json!({"pane": pane}))
    }

    fn rename_pane(&self, _wait: Wait, pane: &str, label: Option<&str>) -> Answer<()> {
        self.ask("rename_pane", json!({"pane": pane, "label": label}))
    }

    fn toggle_zoom(&self, _wait: Wait, pane: &str) -> Answer<bool> {
        self.ask("toggle_zoom", json!({"pane": pane}))
    }

    fn set_zoom(&self, _wait: Wait, pane: &str, zoom: Zoom) -> Answer<()> {
        self.ask("set_zoom", json!({"pane": pane, "zoom": zoom == Zoom::On}))
    }

    fn resize_pane(
        &self,
        _wait: Wait,
        pane: &str,
        direction: crate::live::PaneResizeDirection,
        amount: f32,
    ) -> Answer<bool> {
        self.ask(
            "resize_pane",
            json!({"pane": pane, "direction": debug(&direction), "amount": amount}),
        )
    }

    fn swap_panes(&self, _wait: Wait, first: &str, second: &str) -> Answer<()> {
        self.ask("swap_panes", json!({"first": first, "second": second}))
    }

    fn move_pane_to_new_tab(
        &self,
        _wait: Wait,
        pane: &str,
        workspace: &str,
        label: &str,
    ) -> Answer<PaneJoinResult> {
        self.ask(
            "move_pane_to_new_tab",
            json!({"pane": pane, "workspace": workspace, "label": label}),
        )
    }

    fn move_pane_beside(
        &self,
        _wait: Wait,
        pane: &str,
        target_tab: &str,
        target_pane: &str,
        down: bool,
    ) -> Answer<PaneJoinResult> {
        self.ask(
            "move_pane_beside",
            json!({"pane": pane, "target_tab": target_tab, "target_pane": target_pane, "down": down}),
        )
    }

    fn pane_session(
        &self,
        _wait: Wait,
        pane: &str,
    ) -> Answer<(Option<String>, Option<PaneSession>)> {
        self.ask("pane_session", json!({"pane": pane}))
    }

    fn pane_terminal(&self, _wait: Wait, pane: &str) -> Answer<String> {
        self.ask("pane_terminal", json!({"pane": pane}))
    }

    fn pane_token(&self, _wait: Wait, pane: &str, token: &str) -> Answer<Option<String>> {
        self.ask("pane_token", json!({"pane": pane, "token": token}))
    }

    fn pane_agent_status(&self, _wait: Wait, pane: &str) -> Answer<String> {
        self.ask("pane_agent_status", json!({"pane": pane}))
    }

    fn pane_scroll(
        &self,
        _wait: Wait,
        pane: &str,
    ) -> Answer<Option<crate::find::pane::PaneScroll>> {
        self.ask("pane_scroll", json!({"pane": pane}))
    }

    fn report_pane_tokens(
        &self,
        _wait: Wait,
        pane: &str,
        tokens: &BTreeMap<String, Option<String>>,
    ) -> Answer<()> {
        self.ask(
            "report_pane_tokens",
            json!({"pane": pane, "tokens": tokens}),
        )
    }

    fn report_agent_session(
        &self,
        _wait: Wait,
        pane: &str,
        kind: &str,
        session: &str,
        seq: u64,
    ) -> Answer<()> {
        self.ask(
            "report_agent_session",
            json!({"pane": pane, "kind": kind, "session": session, "seq": seq}),
        )
    }

    fn read_pane(&self, _wait: Wait, pane: &str, source: &str, lines: u32) -> Answer<PaneText> {
        self.ask(
            "read_pane",
            json!({"pane": pane, "source": source, "lines": lines}),
        )
    }

    fn read_screen(&self, _wait: Wait, pane: &str) -> Answer<PaneText> {
        self.ask("read_screen", json!({"pane": pane}))
    }

    fn read_rows(&self, _wait: Wait, pane: &str, lines: u32) -> Answer<PaneText> {
        self.ask("read_rows", json!({"pane": pane, "lines": lines}))
    }

    fn send_text(&self, _wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.ask("send_text", json!({"pane": pane, "text": text}))
    }

    fn send_keys(&self, _wait: Wait, pane: &str, keys: &[&str]) -> Answer<()> {
        self.ask("send_keys", json!({"pane": pane, "keys": keys}))
    }

    fn submit_text(&self, _wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.ask("submit_text", json!({"pane": pane, "text": text}))
    }

    fn press_key(&self, _wait: Wait, pane: &str, key: &str) -> Answer<()> {
        self.ask("press_key", json!({"pane": pane, "key": key}))
    }

    fn press_pane_keys(&self, _wait: Wait, pane: &str, keys: &[keys::PaneKey]) -> Answer<()> {
        self.ask("press_pane_keys", json!({"pane": pane, "keys": keys}))
    }

    fn paste_text(&self, _wait: Wait, pane: &str, text: &str) -> Answer<()> {
        self.ask("paste_text", json!({"pane": pane, "text": text}))
    }

    fn pane_processes(&self, _wait: Wait, pane: &str) -> Answer<PaneProcessGroup> {
        self.ask("pane_processes", json!({"pane": pane}))
    }

    fn resumed_session(
        &self,
        _wait: Wait,
        pane: &str,
        agent: hide_session::Agent,
    ) -> Answer<Option<(String, String)>> {
        self.ask(
            "resumed_session",
            json!({"pane": pane, "agent": debug(&agent)}),
        )
    }

    fn current_pane(&self, _wait: Wait, pane: &str) -> Answer<String> {
        self.ask("current_pane", json!({"pane": pane}))
    }

    fn pane_identity(&self, _wait: Wait, pane: &str) -> Answer<PaneProof> {
        self.ask("pane_identity", json!({"pane": pane}))
    }

    fn inspect_pane(&self, _wait: Wait, pane: &str) -> Answer<hide_node_link::panes::PaneIdentity> {
        self.ask("inspect_pane", json!({"pane": pane}))
    }

    /// Puts a start's arguments in order as every engine does, without a
    /// call: the start itself is the call a test answers.
    fn prepare_start(&self, mut launch: AgentLaunch) -> Result<PreparedStart, String> {
        launch.kind = hide_agent_adapter::canonical_kind(&launch.kind).to_owned();
        launch.args = crate::codex_launch::start_arguments(
            &launch.kind,
            launch.codex_daemon,
            std::mem::take(&mut launch.args),
        )?;
        let line = launch.kind.len() + launch.args.iter().map(|arg| arg.len() + 1).sum::<usize>();
        Ok(PreparedStart::new(launch, line))
    }

    fn replace_outdated_holder(&self, _wait: Wait, pane: &str) -> Answer<bool> {
        self.ask("replace_outdated_holder", json!({ "pane": pane }))
    }

    fn start_agent(&self, _wait: Wait, start: &PreparedStart) -> Answer<StartedAgent> {
        let launch = start.launch();
        self.ask(
            "start_agent",
            json!({
                "pane": launch.pane_id,
                "name": launch.name,
                "kind": launch.kind,
                "args": launch.args,
                "codex_daemon": debug(&launch.codex_daemon),
                "program": launch.program.as_ref().map(|file| json!([file.program, file.leading])),
            }),
        )
    }

    fn agent_state(&self, _wait: Wait, target: &str) -> Answer<AgentState> {
        self.ask("agent_state", json!({"target": target}))
    }

    fn delivery_agent(&self, _wait: Wait, target: &str) -> Answer<DeliveryAgent> {
        self.ask("delivery_agent", json!({"target": target}))
    }

    fn prompt_agent(&self, _wait: Wait, target: &str, text: &str) -> Answer<()> {
        self.ask("prompt_agent", json!({"target": target, "text": text}))
    }

    fn create_worktree(
        &self,
        _wait: Wait,
        cwd: &str,
        branch: &str,
        base: Option<&str>,
        label: Option<&str>,
        path: Option<&str>,
        focus: bool,
    ) -> Answer<CreatedWorktree> {
        self.ask(
            "create_worktree",
            json!({"cwd": cwd, "branch": branch, "base": base, "label": label, "path": path,
                   "focus": focus}),
        )
    }

    fn worktree_path(&self, _wait: Wait, cwd: &str, branch: &str) -> Answer<Option<String>> {
        self.ask("worktree_path", json!({"cwd": cwd, "branch": branch}))
    }

    fn open_worktree(
        &self,
        _wait: Wait,
        path: &str,
        repository_root: &str,
        focus: bool,
    ) -> Answer<OpenedWorktree> {
        self.ask(
            "open_worktree",
            json!({"path": path, "repository_root": repository_root, "focus": focus}),
        )
    }

    fn remove_worktree(&self, _wait: Wait, workspace: &str) -> Answer<()> {
        self.ask("remove_worktree", json!({"workspace": workspace}))
    }
}
