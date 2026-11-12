//! PRD agent-sleep through the runtime's own events and session updates.
//! The engine workers are not run here (their boundary has its own tests in
//! `agent_sleep_engine.rs`); what they hand back is fed in through the same
//! `ingest_agent_sleep_*` calls a worker makes.

use super::*;

const CHECKOUT: &str = "/private/tmp/hide-agent-sleep";
const TABS: [&str; 2] = ["w-order:t1", "w-order:t2"];
const SLEEPER: &str = "w-order:t2:p";

fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": kind,
        "payload": payload
    }))
    .unwrap()
}

/// The two-tab session with t1 in front, and, when `seq` is given, an
/// idle Claude agent in t2 that the engine lists with that state sequence.
fn session(seq: Option<u64>) -> SessionSnapshotPayload {
    session_in(CHECKOUT, seq)
}

/// `session` with its tabs and agent in the checkout at `path`.
fn session_in(path: &str, seq: Option<u64>) -> SessionSnapshotPayload {
    let mut payload = tab_order_payload(path, &TABS, &TABS, "w-order:t1");
    if let Some(seq) = seq {
        let owned: SessionSnapshotPayload =
            crate::sidebar::owned_label_fixture(serde_json::json!({"agents": [{
                "id": "reviewer", "pane_id": SLEEPER, "agent": "claude",
                "agent_status": "idle", "state_change_seq": seq, "cwd": path,
                "agent_session": {"kind": "id", "value": "11111111-2222-3333-4444-555555555555"},
                "tokens": {"task": "Review the parser"}
            }]}))
            .unwrap();
        payload.agents.extend(owned.agents);
    }
    payload
}

fn row(runtime: &Runtime) -> serde_json::Value {
    serde_json::to_value(&runtime.snapshot().navigator.agents)
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["pane_id"] == SLEEPER)
        .cloned()
        .expect("the agent row")
}

fn pane(runtime: &Runtime) -> serde_json::Value {
    let pane = runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| workspace.checkouts.iter())
        .flat_map(|checkout| checkout.tabs.iter())
        .flat_map(|tab| tab.panes.iter())
        .find(|pane| pane.id == SLEEPER)
        .expect("the pane");
    serde_json::to_value(pane).unwrap()
}

/// A live runtime whose agent in t2 was put to sleep from the pane menu
/// and whose end worker reported success.
fn asleep() -> (Runtime, String) {
    let (mut runtime, checkout_id) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    assert_eq!(row(&runtime)["group"], "seen");
    assert!(pane(&runtime)["sleep_action"]["available"] == true);
    runtime.dispatch_json(&event(
        "agent_sleep",
        serde_json::json!({"pane_id": SLEEPER}),
    ));
    assert!(runtime.snapshot().status.last_error.is_none());
    assert!(
        row(&runtime).get("sleep").is_none(),
        "the agent is awake until its end lands (B8)"
    );
    assert!(runtime.ingest_agent_sleep_end(SLEEPER, Ok(4)));
    (runtime, checkout_id)
}

// The own engine stops listing an agent the moment its process ends, so the
// session that no longer lists it is applied while the record is still
// ending, which draws no row. The end landing asks for the session again,
// and that session draws the sleeping row from the record (PRD agent-sleep B10).
#[test]
fn a_sleep_whose_agent_left_the_session_before_its_end_landed_draws_its_row_on_the_next_read() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    let state = tempfile::tempdir().unwrap();
    runtime.run_devices_on_own_engine(state.path().to_path_buf());
    runtime.ingest_session(Ok(session(Some(4))));
    runtime.dispatch_json(&event(
        "agent_sleep",
        serde_json::json!({"pane_id": SLEEPER}),
    ));
    runtime.ingest_session(Ok(session(None)));
    assert!(
        !runtime
            .snapshot()
            .navigator
            .agents
            .iter()
            .any(|agent| agent.pane_id == SLEEPER),
        "an ending record draws no row of its own"
    );
    runtime.take_republish_request();
    assert!(runtime.ingest_agent_sleep_end(SLEEPER, Ok(4)));
    assert!(
        runtime.take_republish_request(),
        "the end asks for the session again"
    );
    runtime.ingest_session(Ok(session(None)));
    assert_eq!(row(&runtime)["sleep"]["state"], "sleeping");
}

// 2026-11-06: a reboot started every agent again, forty-two at once, and
// the machine crawled. A restore leaves an agent that had finished its turn
// resting in its pane, a shell, and the core keeps it as a sleeping agent
// from the row it would have had: nothing runs to be ended, a visit wakes it
// into its conversation, and the engine is told it was taken, once.
#[test]
fn an_agent_a_restore_left_resting_is_kept_asleep_in_its_conversation() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(None)));
    let resting = session(Some(0)).agents;
    assert!(runtime.adopt_resting(resting.clone(), None));
    runtime.ingest_session(Ok(session(None)));
    assert_eq!(row(&runtime)["sleep"]["state"], "sleeping");
    assert_eq!(pane(&runtime)["sleep"]["state"], "sleeping");
    let record = runtime.snapshot().ui_state.agent_sleep.records[SLEEPER].clone();
    assert_eq!(record.kind, "claude");
    assert_eq!(
        record.session_id.as_deref(),
        Some("11111111-2222-3333-4444-555555555555"),
        "a wake resumes its conversation"
    );
    assert_eq!(
        record.state_change_seq, None,
        "any agent listed there is new"
    );

    // The engine names it until an agent shows in the pane: the record
    // stays as it is.
    assert!(!runtime.adopt_resting(resting, None));
    assert_eq!(
        runtime.snapshot().ui_state.agent_sleep.records[SLEEPER],
        record
    );
}

// A row keeps a session only once its labels prove it. An agent that may
// sleep without one (agy) would otherwise be kept with none and wake into a
// new conversation; it keeps the one the restore would have resumed.
#[test]
fn a_resting_agent_with_no_proven_session_keeps_the_one_its_restore_named() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(None)));
    let resting: SessionSnapshotPayload = serde_json::from_value(serde_json::json!({"agents": [{
        "id": SLEEPER, "pane_id": SLEEPER, "agent": "antigravity",
        "agent_status": "idle", "state_change_seq": 0, "cwd": CHECKOUT,
        "agent_session": {"kind": "id", "value": "agy-conversation-1"}
    }]}))
    .unwrap();
    assert!(runtime.adopt_resting(resting.agents, None));
    let record = &runtime.snapshot().ui_state.agent_sleep.records[SLEEPER];
    assert_eq!(record.session_id.as_deref(), Some("agy-conversation-1"));
}

// The engine can still list the agent when its sleep ends, and its
// event stream brings the session that no longer does: the end asks for no
// read of its own there.
#[test]
fn a_sleep_end_on_the_engine_asks_for_no_session_read() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    runtime.dispatch_json(&event(
        "agent_sleep",
        serde_json::json!({"pane_id": SLEEPER}),
    ));
    runtime.ingest_session(Ok(session(None)));
    runtime.take_republish_request();
    assert!(runtime.ingest_agent_sleep_end(SLEEPER, Ok(4)));
    assert!(!runtime.take_republish_request());
}

/// The generic dormant owner is exercised with a proven fixture session;
/// production Claude still reaches only the unchanged keep-pane branch.
fn dormant_intent(runtime: &mut Runtime) -> crate::agent_sleep::SleepId {
    use crate::agent_sleep::{DormantPhase, DormantRecord};
    runtime
        .snapshot
        .navigator
        .agents
        .iter_mut()
        .find(|agent| agent.pane_id == SLEEPER)
        .unwrap()
        .row_facts
        .get_or_insert_with(Default::default);
    let agent = runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .find(|agent| agent.pane_id == SLEEPER)
        .unwrap()
        .clone();
    let (tab, owner) = runtime.sleep_place(SLEEPER).unwrap();
    let context = runtime.close_context(tab).unwrap();
    let native_session_id = agent.session_id.unwrap();
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .admit_dormant(DormantRecord {
            source_reference: agent
                .row_facts
                .as_ref()
                .and_then(|facts| facts.native_reference.clone()),
            phase: DormantPhase::SavingClose,
            revision: 1,
            node_id: runtime.node.as_str().into(),
            connection_generation: runtime.live_generation,
            old_pane_id: SLEEPER.into(),
            old_state_change_seq: agent.state_change_seq,
            label_owner: hide_session::label_reference_token("claude", "id", &native_session_id)
                .unwrap(),
            kind: "claude".into(),
            native_session_id,
            identity_label: agent.identity_label,
            cwd: context.checkout_path.clone(),
            context,
            close_key: None,
            closed: false,
            wake_pane_id: None,
            wake_tab_id: None,
            factory: None,
            since_unix_ms: 1,
            transition_started_unix_ms: 1,
            reason: None,
            registration: None,
            launch: Default::default(),
            owner: Some(owner),
            profile: None,
        })
        .unwrap()
}

/// The engine with no workspace open: an unregistered project whose last
/// workspace closed is no longer listed.
fn no_workspaces() -> SessionSnapshotPayload {
    crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [], "workspaces": [], "tabs": [], "panes": [], "layouts": []
    }))
    .unwrap()
}

fn listed(runtime: &Runtime, path: &str) -> bool {
    runtime
        .snapshot
        .navigator
        .workspaces
        .iter()
        .flat_map(|project| &project.checkouts)
        .any(|checkout| checkout.path == path)
}

/// The project list stops showing the sleeping session's checkout, as it
/// does for an unregistered project once its last workspace closes, and for
/// a linked worktree no window has read since hided started.
fn unlist(runtime: &mut Runtime, path: &str) {
    runtime.snapshot.ui_state.workspace_registrations.clear();
    runtime.ingest_session(Ok(no_workspaces()));
    assert!(!listed(runtime, path));
}

/// The session the engine answers, as a session in its own field names.
#[cfg(unix)]
fn dormant_wire_session(cwd: &str, tabs: &[&str]) -> serde_json::Value {
    serde_json::json!({"workspaces":[{"workspace_id":"w-order","number":1,"label":"order","active_tab_id":"w-order:t1","focused":true,"agent_status":"idle","pane_count":tabs.len(),"tab_count":tabs.len(),"tokens":{"hide_owner":crate::checkout_owner::owner_mark(crate::node::TEST_NODE, cwd)}}],
        "tabs":tabs.iter().enumerate().map(|(index, tab)| serde_json::json!({
            "tab_id":tab,"workspace_id":"w-order","number":index+1,"label":"fixture",
            "focused":index==0,"pane_count":1,"agent_status":"idle"
        })).collect::<Vec<_>>(),
        "panes":tabs.iter().map(|tab| serde_json::json!({
            "pane_id":format!("{tab}:p"),"terminal_id":format!("term-{tab}"),"workspace_id":"w-order",
            "tab_id":tab,"cwd":cwd,"focused":false,"agent_status":"idle","revision":0
        })).collect::<Vec<_>>(),
        "layouts":tabs.iter().map(|tab| serde_json::json!({
            "workspace_id":"w-order","tab_id":tab,"zoomed":false,
            "area":{"x":0,"y":0,"width":80,"height":24},"focused_pane_id":format!("{tab}:p"),
            "panes":[{"pane_id":format!("{tab}:p"),"focused":true,"rect":{"x":0,"y":0,"width":80,"height":24}}],"splits":[]
        })).collect::<Vec<_>>(),"agents":[]})
}

fn persisted_dormant(path: &Path, id: &crate::agent_sleep::SleepId) -> serde_json::Value {
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    state["agent_sleep"]["dormant"][id.as_str()].clone()
}

/// B5: real state writes authorize the ordinary close and wake workers.
/// The external server observes each saved intent before accepting its effect.
#[test]
#[cfg(unix)]
fn a_durable_dormant_journey_has_one_resume_authority_and_exact_native_confirmation() {
    durable_dormant_journey("claude", "none");
}

#[test]
#[cfg(unix)]
fn a_native_dormant_journey_refuses_a_changed_route_before_close_wake_or_start() {
    for kind in ["pi", "omp"] {
        for phase in [
            "none",
            "before-close",
            "before-wake",
            "before-start",
            "missing-source-before-close",
            "missing-source-before-wake",
            "missing-source-before-start",
        ] {
            durable_dormant_journey(kind, phase);
        }
    }
    for phase in [
        "recovery-before-close",
        "recovery-before-wake",
        "recovery-before-start",
        "hashed-before-close",
        "chained-migration-before-close",
    ] {
        durable_dormant_journey("omp", phase);
    }
}

/// A retained conversation survives retirement, without acquiring a new
/// wake, pane or provider integration in this build.
#[test]
fn a_retired_cursor_sleep_record_is_preserved_but_cannot_wake() {
    use crate::agent_sleep::DormantPhase;
    let mut runtime = close_pane_runtime("pi");
    assert!(runtime.begin_dormant_sleep(SLEEPER, 1));
    let (id, native_id) = {
        let (id, record) = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .iter_mut()
            .next()
            .unwrap();
        record.kind = "cursor".into();
        record.label_owner =
            hide_session::label_reference_token("cursor", "id", &record.native_session_id).unwrap();
        record.validate().unwrap();
        record.closed = true;
        record.phase = DormantPhase::Sleeping;
        (id.clone(), record.native_session_id.clone())
    };
    runtime.close_operations.clear();
    runtime.refresh_dormant_rows();
    assert!(!runtime.snapshot.navigator.sleeping_sessions[0].wake_available);
    assert!(runtime.request_dormant_wake(&id));
    assert_eq!(
        runtime.snapshot.status.last_error.as_ref().unwrap().kind,
        "agent_sleep.wake_unavailable"
    );
    let retained = &runtime.snapshot.ui_state.agent_sleep.dormant[&id];
    assert_eq!(retained.kind, "cursor");
    assert_eq!(retained.native_session_id, native_id);
    assert!(retained.wake_pane_id.is_none());
    assert!(runtime.close_operations.is_empty());
}

/// OpenCode's proof reads its database row: a session moved to another
/// checkout or gone refuses every later effect, and nothing is written there.
#[test]
#[cfg(unix)]
fn an_opencode_dormant_journey_refuses_a_changed_route_before_close_wake_or_start() {
    for phase in [
        "none",
        "before-close",
        "before-wake",
        "before-start",
        "missing-source-before-close",
        "missing-source-before-wake",
        "missing-source-before-start",
    ] {
        durable_dormant_journey("opencode", phase);
    }
}

/// The same proof over OpenCode 2.0's tables: the engine reports the session's id,
/// and Hide proves it from `session_v2`, so sleep is not refused for want of
/// a conversation.
#[test]
#[cfg(unix)]
fn an_opencode_2_dormant_journey_refuses_a_changed_route_before_close_wake_or_start() {
    for phase in [
        "none",
        "before-close",
        "before-wake",
        "before-start",
        "missing-source-before-close",
        "missing-source-before-wake",
        "missing-source-before-start",
    ] {
        durable_dormant_journey_in("opencode", phase, true);
    }
}

/// What a journey's interference does to the native record its proof reads.
#[cfg(unix)]
struct NativeRecord {
    kind: &'static str,
    /// OpenCode's 2.x tables hold the session, not its 1.x ones.
    opencode_2: bool,
    path: std::path::PathBuf,
    folder: std::path::PathBuf,
    before: Vec<u8>,
    other: Vec<u8>,
}

#[cfg(unix)]
impl NativeRecord {
    /// Another session where this one was (a native file), or this session moved to
    /// another checkout (OpenCode).
    fn duplicate(&self) {
        if self.kind == "opencode" {
            self.database()
                .execute(
                    &format!(
                        "UPDATE {} SET directory = '/elsewhere/app'",
                        if self.opencode_2 {
                            "session_v2"
                        } else {
                            "session"
                        }
                    ),
                    [],
                )
                .unwrap();
        } else {
            std::fs::copy(&self.path, self.folder.join("duplicate.jsonl")).unwrap();
        }
    }

    fn remove(&self) {
        if self.kind == "opencode" {
            self.database()
                .execute_batch(if self.opencode_2 {
                    "DELETE FROM session_message; DELETE FROM session_v2;"
                } else {
                    "DELETE FROM part; DELETE FROM message; DELETE FROM session;"
                })
                .unwrap();
        } else {
            std::fs::write(self.folder.join("other.jsonl"), &self.other).unwrap();
            std::fs::remove_file(&self.path).unwrap();
        }
    }

    fn database(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(&self.path).unwrap()
    }

    /// The conversation as the journey left it: never rewritten by Hide.
    fn assert_untouched(&self, interference: &str) {
        if self.kind == "opencode" {
            let rows: i64 = self
                .database()
                .query_row(
                    if self.opencode_2 {
                        "SELECT count(*) FROM session_message"
                    } else {
                        "SELECT count(*) FROM message"
                    },
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(rows, i64::from(!interference.starts_with("missing-source")));
            return;
        }
        assert_eq!(
            std::fs::read(if self.path.exists() {
                self.path.clone()
            } else {
                self.folder.join("other.jsonl")
            })
            .unwrap(),
            if interference.starts_with("missing-source") {
                self.other.clone()
            } else {
                self.before.clone()
            }
        );
    }
}

#[cfg(unix)]
fn durable_dormant_journey(kind: &'static str, interference: &'static str) {
    durable_dormant_journey_in(kind, interference, false);
}

/// The journey of `kind`; `opencode_2` puts an OpenCode session in the 2.x tables.
#[cfg(unix)]
fn durable_dormant_journey_in(kind: &'static str, interference: &'static str, opencode_2: bool) {
    use crate::agent_sleep::DormantPhase;
    let folder = tempfile::Builder::new()
        .prefix(if interference == "hashed-before-close" {
            "repo-한글x"
        } else {
            ".tmp"
        })
        .tempdir()
        .unwrap();
    let cwd = folder
        .path()
        .canonicalize()
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let (mut runtime, _) = live_tab_order_runtime(&cwd);
    let home = tempfile::tempdir().unwrap();
    let native_id = "11111111-2222-3333-4444-555555555555";
    let native = if kind == "opencode" {
        let sessions = [(native_id, None, cwd.as_str())];
        if opencode_2 {
            crate::fixture::opencode_2_database(home.path(), &sessions);
        } else {
            crate::fixture::opencode_database(home.path(), &sessions);
        }
        let path = home.path().join(".local/share/opencode/opencode.db");
        NativeRecord {
            kind,
            opencode_2,
            folder: path.parent().unwrap().to_path_buf(),
            before: Vec::new(),
            other: Vec::new(),
            path,
        }
    } else {
        let native_path = {
            let folder = crate::fixture::native_session_folder(
                home.path(),
                if kind == "omp" { "omp" } else { "pi" },
                Path::new(&cwd),
            );
            std::fs::create_dir_all(&folder).unwrap();
            let file = folder.join("native.jsonl");
            std::fs::write(
                &file,
                format!(
                    "{}\n",
                    serde_json::json!({"type":"session", "version":3, "id":native_id, "cwd":cwd})
                ),
            )
            .unwrap();
            file
        };
        let native_folder = native_path.parent().unwrap().to_path_buf();
        let native_before = std::fs::read(&native_path).unwrap();
        let other_before = [native_before.as_slice(), b"{\"type\":\"message\",\"id\":\"other-message\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"different history\"}]}}\n"].concat();
        NativeRecord {
            kind,
            opencode_2: false,
            path: native_path,
            folder: native_folder,
            before: native_before,
            other: other_before,
        }
    };
    let native = Arc::new(native);
    let native_folder = native.folder.clone();
    let backup_path = native_folder.join(format!("date_{native_id}-alias.jsonl.123.bak"));
    let backup_bytes = String::from_utf8(native.other.clone())
        .unwrap()
        .replace(native_id, "different-native-owner");
    if kind != "claude" {
        runtime.own_node = Arc::new(hide_node::Local::new(Some(home.path().to_path_buf())));
        runtime.live.as_mut().unwrap().node = runtime.own_node();
    }
    let mut initial = session(Some(4));
    for pane in &mut initial.panes {
        pane.cwd = Some(cwd.clone());
    }
    initial.agents[0].cwd = Some(cwd.clone());
    initial.agents[0].agent = Some(kind.into());
    initial.agents[0].facts = Some(crate::request_view::RowFacts {
        native_session_id: Some(native_id.into()),
        native_reference: Some(if kind == "opencode" {
            crate::sidebar::SessionAgentSessionPayload {
                kind: "id".into(),
                value: native_id.into(),
            }
        } else {
            crate::sidebar::SessionAgentSessionPayload {
                kind: "path".into(),
                value: native.path.display().to_string(),
            }
        }),
        ..Default::default()
    });
    runtime.ingest_session(Ok(initial));
    let id = dormant_intent(&mut runtime);
    // The choice the agent was started with, and the words each CLI takes it
    // by after its resume arguments; OpenCode takes no model.
    let (launch, launch_words): (hide_agent_adapter::LaunchChoice, &[&str]) = match kind {
        "claude" => (
            hide_agent_adapter::LaunchChoice {
                model: Some("sonnet".into()),
                effort: Some("high".into()),
            },
            &["--model", "sonnet", "--effort", "high"],
        ),
        "opencode" => (Default::default(), &[]),
        _ => (
            hide_agent_adapter::LaunchChoice {
                model: Some("fixture-model".into()),
                effort: None,
            },
            &["--model", "fixture-model"],
        ),
    };
    {
        let record = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .get_mut(&id)
            .unwrap();
        record.kind = kind.into();
        record.label_owner = hide_session::label_reference_token(kind, "id", native_id).unwrap();
        record.launch = launch.clone();
        // The wake opens its tab from the owner this record kept, and starts
        // the agent with the choice the record kept.
        assert!(record.owner.is_some());
    }
    if interference == "before-close" {
        native.duplicate();
    }
    if interference == "recovery-before-close" {
        std::fs::write(&backup_path, &backup_bytes).unwrap();
    }
    let migration_alias = match interference {
        "hashed-before-close" => {
            use sha2::{Digest, Sha256};
            // Pinned native regex preserves the literal '-' then replaces
            // this invalid run with another '-', retaining the temp suffix.
            let basename = std::path::Path::new(&cwd)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .replace("한글", "-");
            Some(format!(
                "tmp-{basename}-{:x}",
                Sha256::digest(cwd.replace('\\', "/").as_bytes())
            ))
        }
        "chained-migration-before-close" => {
            let encoded_home = home
                .path()
                .to_str()
                .unwrap()
                .trim_start_matches(['/', '\\'])
                .replace(['/', '\\', ':'], "-");
            let encoded_cwd = cwd
                .trim_start_matches(['/', '\\'])
                .replace(['/', '\\', ':'], "-");
            // Native root-home migration first creates --<cwd>--, then
            // the cwd-specific legacy migration merges it into the default.
            Some(format!("--{encoded_home}--{encoded_cwd}----"))
        }
        _ => None,
    }
    .map(|bucket| {
        let bucket = native_folder.parent().unwrap().join(bucket);
        std::fs::create_dir(&bucket).unwrap();
        let alias = bucket.join(format!("date_{native_id}-alias.jsonl"));
        std::fs::write(&alias, &backup_bytes).unwrap();
        alias
    });
    if interference == "missing-source-before-close" {
        native.remove();
    }
    assert_eq!(runtime.snapshot.ui_state.agent_sleep.dormant[&id].cwd, cwd);
    let path = runtime.state_path.clone();
    let saved_path = path.clone();
    let saved_id = id.clone();
    let saved_cwd = cwd.clone();
    let create_native = Arc::clone(&native);
    let create_backup = backup_path.clone();
    let create_backup_bytes = backup_bytes.clone();
    let (effects, observed) = std::sync::mpsc::channel();
    let mut created = false;
    let fake = FakeEngine::start(move |method, params| match method {
        "export_layout" => {
            assert_eq!(
                persisted_dormant(&saved_path, &saved_id)["phase"],
                "saving_close"
            );
            reply(fake::closed_layout(serde_json::json!({
                "workspace_id":"w-order","tab_id":"w-order:t2","zoomed":false,
                "focused_pane_id":SLEEPER,"root":{"type":"pane","pane_id":SLEEPER,"cwd":saved_cwd,"env":{}}
            })))
        }
        "close_pane" => {
            let saved = persisted_dormant(&saved_path, &saved_id);
            assert_eq!(saved["phase"], "saving_close_ready");
            assert!(saved["close_key"].is_string());
            assert_eq!(params["pane"], SLEEPER);
            effects.send("close").unwrap();
            reply(())
        }
        "session" => {
            // The layout worker reads once before and once after creating its own tab.
            let tabs = if created {
                vec!["w-order:t1", "w-order:t3"]
            } else {
                vec!["w-order:t1"]
            };
            reply(fake::session(dormant_wire_session(&saved_cwd, &tabs)))
        }
        "apply_layout" => {
            if interference == "recovery-before-start" {
                std::fs::write(&create_backup, &create_backup_bytes).unwrap();
            }
            if interference == "missing-source-before-start" {
                create_native.remove();
            }
            if interference == "before-start" {
                create_native.duplicate();
            }
            assert_eq!(
                persisted_dormant(&saved_path, &saved_id)["phase"],
                "saving_wake"
            );
            assert_eq!(params["root"]["cwd"], saved_cwd);
            assert!(
                params["root"]["env"]["HIDE_REOPEN_INTENT"]
                    .as_str()
                    .unwrap()
                    .contains(saved_id.as_str())
            );
            created = true;
            effects.send("create").unwrap();
            reply(fake::closed_layout(serde_json::json!({
                "workspace_id":"w-order","tab_id":"w-order:t3","zoomed":false,
                "focused_pane_id":"w-order:t3:p","root":{"type":"pane","pane_id":"w-order:t3:p","cwd":saved_cwd,"env":params["root"]["env"]}
            })))
        }
        "move_tab" => tab_list(&["w-order:t1", "w-order:t3"]),
        "pane_processes" => reply(fake::processes(&serde_json::json!({
            "shell_pid":42,"foreground_process_group_id":42,
            "foreground_processes":[{"pid":42,"name":"zsh"}]
        }))),
        "start_agent" => {
            let saved = persisted_dormant(&saved_path, &saved_id);
            assert_eq!(saved["phase"], "saving_start");
            assert_eq!(saved["wake_pane_id"], "w-order:t3:p");
            assert_eq!(params["pane"], "w-order:t3:p");
            assert_eq!(params["kind"], kind);
            let resume = [
                match kind {
                    "pi" => "--session",
                    "opencode" => "-s",
                    _ => "--resume",
                },
                "11111111-2222-3333-4444-555555555555",
            ];
            // No saved 권한: each CLI's full-access option (its help,
            // read 2026-11-01) goes before the choice.
            let access: &[&str] = match kind {
                "claude" => &crate::agent_permission::CLAUDE_FULL_ACCESS,
                "codex" => &crate::agent_permission::CODEX_FULL_ACCESS,
                "opencode" => &["--auto"],
                "omp" => &["--auto-approve"],
                "grok" => &["--always-approve"],
                "agy" => &["--dangerously-skip-permissions"],
                _ => &[],
            };
            assert_eq!(
                params["args"],
                serde_json::json!([resume.as_slice(), access, launch_words].concat())
            );
            effects.send("start").unwrap();
            reply(crate::engine::StartedAgent {
                pane_id: "w-order:t3:p".into(),
                terminal_id: Some("term-wake".into()),
            })
        }
        other => panic!("unexpected dormant effect: {other}"),
    });
    runtime.live.as_mut().unwrap().engine = fake.engine();
    let shared = Arc::new(std::sync::Mutex::new(runtime));
    shared.lock().unwrap().live.as_mut().unwrap().runtime = Arc::downgrade(&shared);
    shared.lock().unwrap().write_ui_state().unwrap();
    if matches!(
        interference,
        "before-close"
            | "missing-source-before-close"
            | "recovery-before-close"
            | "hashed-before-close"
            | "chained-migration-before-close"
    ) {
        wait_for("refused native close", || {
            !shared
                .lock()
                .unwrap()
                .snapshot
                .ui_state
                .agent_sleep
                .dormant
                .contains_key(&id)
        });
        // Capturing the old layout is read-only. B2/B8 forbid close or launch
        // effects after the exact native route becomes unconfirmed.
        assert!(observed.try_recv().is_err());
        assert!(
            !fake.methods().iter().any(|method| matches!(
                *method,
                "close_pane" | "close_tab" | "apply_layout" | "start_agent"
            )),
            "unexpected effect: {:?}",
            fake.methods()
        );
        assert_eq!(row(&shared.lock().unwrap())["pane_id"], SLEEPER);
        if let Some(alias) = migration_alias {
            assert_eq!(std::fs::read_to_string(alias).unwrap(), backup_bytes);
        }
        native.assert_untouched(interference);
        return;
    }
    assert_eq!(
        observed.recv_timeout(Duration::from_secs(5)).unwrap(),
        "close"
    );
    let retired_capture;
    {
        let mut runtime = shared.lock().unwrap();
        retired_capture = runtime
            .close_operations
            .values()
            .next()
            .unwrap()
            .request
            .clone();
        assert!(!runtime.snapshot.recent_closed.can_reopen);
        assert!(runtime.snapshot.recent_closed.pending.is_empty());
        runtime.dispatch_json(&event("reopen_closed", serde_json::json!({})));
        assert!(runtime.reopen_after_close.is_none());
        runtime.ingest_session(Ok(tab_order_payload(
            &cwd,
            &["w-order:t1"],
            &["w-order:t1"],
            "w-order:t1",
        )));
        assert_eq!(runtime.snapshot.navigator.sleeping_sessions.len(), 1);
        assert_eq!(
            runtime.snapshot.navigator.sleeping_sessions[0].phase,
            DormantPhase::Sleeping
        );
        assert_eq!(runtime.snapshot.recent_closed.count, 0);
        if interference == "before-wake" {
            native.duplicate();
        }
        if interference == "recovery-before-wake" {
            std::fs::write(&backup_path, &backup_bytes).unwrap();
        }
        if interference == "missing-source-before-wake" {
            native.remove();
        }
        if interference == "unlisted-before-wake" {
            unlist(&mut runtime, &cwd);
            assert!(runtime.snapshot.navigator.sleeping_sessions[0].wake_available);
        }
        assert!(runtime.request_dormant_wake(&id));
        assert!(!runtime.request_dormant_wake(&id));
    }
    if matches!(
        interference,
        "before-wake" | "missing-source-before-wake" | "recovery-before-wake"
    ) {
        wait_for("refused native wake tab", || {
            shared.lock().unwrap().snapshot.ui_state.agent_sleep.dormant[&id].phase
                == DormantPhase::Failed
        });
        assert!(
            !fake
                .methods()
                .iter()
                .any(|method| matches!(*method, "apply_layout" | "start_agent"))
        );
        assert!(
            shared.lock().unwrap().snapshot.ui_state.agent_sleep.dormant[&id]
                .wake_pane_id
                .is_none()
        );
        native.assert_untouched(interference);
        return;
    }
    assert_eq!(
        observed.recv_timeout(Duration::from_secs(5)).unwrap(),
        "create"
    );
    if matches!(
        interference,
        "before-start" | "missing-source-before-start" | "recovery-before-start"
    ) {
        wait_for("refused native wake start", || {
            shared.lock().unwrap().snapshot.ui_state.agent_sleep.dormant[&id].phase
                == DormantPhase::Failed
        });
        assert!(!fake.methods().contains(&"start_agent"));
        assert_eq!(
            shared.lock().unwrap().snapshot.ui_state.agent_sleep.dormant[&id]
                .wake_pane_id
                .as_deref(),
            Some("w-order:t3:p")
        );
        native.assert_untouched(interference);
        return;
    }
    assert_eq!(
        observed.recv_timeout(Duration::from_secs(5)).unwrap(),
        "start"
    );
    {
        let mut runtime = shared.lock().unwrap();
        let work = crate::agent_sleep_engine::DormantWork {
            id: id.clone(),
            record: runtime.snapshot.ui_state.agent_sleep.dormant[&id].clone(),
        };
        let mut stale = work.clone();
        stale.record.connection_generation += 1;
        assert!(!runtime.ingest_dormant_tab(
            &stale,
            crate::agent_sleep_engine::DormantTabOutcome::Unknown
        ));
        assert_eq!(
            runtime.snapshot.ui_state.agent_sleep.dormant[&id],
            work.record
        );
        assert!(!runtime.request_dormant_wake(&id));
        assert_eq!(runtime.snapshot.navigator.sleeping_sessions.len(), 1);
        let mut confirmed = tab_order_payload(
            &cwd,
            &["w-order:t1", "w-order:t3"],
            &["w-order:t1", "w-order:t3"],
            "w-order:t1",
        );
        let mut agent = session(Some(5)).agents.remove(0);
        let wake_name = crate::agent_sleep_engine::dormant_wake_name(&work);
        agent.name = Some(wake_name.clone());
        agent.agent = Some(kind.into());
        agent.pane_id = Some("w-order:t3:p".into());
        agent.cwd = Some(cwd.clone());
        agent.facts = Some(crate::request_view::RowFacts {
            native_session_id: Some(native_id.into()),
            ..Default::default()
        });
        confirmed.agents.push(agent);
        // The woken pane's record is there before its agent shows, so the
        // agent's session is read in its profile's account (B19).
        let recorded = |runtime: &Runtime| {
            runtime
                .snapshot
                .ui_state
                .pane_launches
                .get("w-order:t3:p")
                .map(|record| (record.name.clone(), record.choice.clone()))
        };
        assert_eq!(
            recorded(&runtime),
            Some((wake_name.clone(), launch.clone())),
            "{kind}"
        );
        runtime.ingest_session(Ok(confirmed));
        assert!(runtime.snapshot.navigator.sleeping_sessions.is_empty());
        // The woken pane keeps the choice for the agent's next restart, the
        // CLI's defaults too, so a child it starts knows its account (D-32).
        assert_eq!(
            runtime
                .snapshot
                .ui_state
                .pane_launches
                .get("w-order:t3:p")
                .map(|record| (record.name.as_str(), &record.choice)),
            Some((wake_name.as_str(), &launch)),
            "{kind}"
        );
        assert!(runtime.snapshot.ui_state.agent_sleep.dormant.is_empty());
        assert_eq!(runtime.snapshot.recent_closed.count, 0);
        assert!(persisted_dormant(&path, &id).is_null());
        let (changed, effects) = runtime.ingest_close_capture_result(
            &retired_capture,
            Ok(crate::recent_closed::capture::CloseCaptureOutcome { item: None }),
        );
        assert!(!changed);
        assert!(effects.is_empty());
        assert!(!runtime.ingest_close_effect_result(
            &crate::recent_closed::capture::CloseEffectRequest {
                safety: None,
                failed_terminals: Vec::new(),
                allow_replacement_create: false,
                replacement: None,
                key: retired_capture.key.clone(),
                connection_generation: retired_capture.connection_generation,
                target: retired_capture.target.clone(),
            },
            Ok(())
        ));
        assert!(runtime.close_operations.is_empty());
    }
    let calls = fake.methods();
    assert_eq!(
        calls
            .iter()
            .filter(|called| **called == "report_agent_session")
            .count(),
        0,
        "{calls:?}"
    );
    for method in ["close_pane", "apply_layout", "start_agent"] {
        assert_eq!(
            calls.iter().filter(|called| **called == method).count(),
            1,
            "{calls:?}"
        );
    }
}

/// The wake opens the checkout from the owner its record kept, so it works
/// while no list shows the checkout: Wake from a phone, with no window
/// attached since hided restarted, or in an unregistered project whose last
/// workspace was the sleeping session's. It starts the agent with the model
/// and effort the record kept, as every journey asserts at `agent.start`.
#[test]
#[cfg(unix)]
fn a_sleeping_session_wakes_in_its_checkout_while_no_list_shows_it() {
    durable_dormant_journey("claude", "unlisted-before-wake");
}

/// A live runtime whose agent in t2 is a `kind` session with a proven native
/// file, idle and past the minute decision.
fn close_pane_runtime(kind: &str) -> Runtime {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    let mut payload = session(Some(4));
    payload.agents[0].agent = Some(kind.into());
    runtime.ingest_session(Ok(payload));
    // The row of a kind whose conversation needs its file's proof carries
    // the session id only once that file was read.
    let row = runtime
        .snapshot
        .navigator
        .agents
        .iter_mut()
        .find(|agent| agent.pane_id == SLEEPER)
        .unwrap();
    let id = "11111111-2222-3333-4444-555555555555";
    row.session_id = Some(id.into());
    let facts = row.row_facts.get_or_insert_with(Default::default);
    facts.native_session_id = Some(id.into());
    facts.native_reference = Some(crate::sidebar::SessionAgentSessionPayload {
        kind: "path".into(),
        value: "/fixture/native.jsonl".into(),
    });
    runtime
}

/// A sleep that closes the pane keeps the choice Hide started the agent with
/// in its saved intent, which its wake starts the agent with again.
#[test]
fn a_dormant_sleep_keeps_the_choice_the_agent_was_started_with() {
    let launch = hide_agent_adapter::LaunchChoice {
        model: Some("fixture-model".into()),
        effort: None,
    };
    for (recorded, saved) in [(Some(&launch), launch.clone()), (None, Default::default())] {
        let mut runtime = close_pane_runtime("pi");
        // The name Hide started it under, which the engine reports it by.
        runtime
            .snapshot
            .navigator
            .agents
            .iter_mut()
            .find(|agent| agent.pane_id == SLEEPER)
            .unwrap()
            .started_name = Some("reviewer".into());
        if let Some(recorded) = recorded {
            runtime.record_pane_launch(SLEEPER, "pi", "reviewer", recorded, None, None);
        }
        runtime.dispatch_json(&event(
            "agent_sleep",
            serde_json::json!({"pane_id": SLEEPER}),
        ));
        let record = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .values()
            .next()
            .expect("the saved intent");
        assert_eq!(record.launch, saved);
    }
}

/// Case (b) itself: an agent that slept in a linked worktree is woken through
/// `worktree.open` from its repository's main worktree, and the workspace
/// the engine opens takes the worktree's branch name, while the list shows the
/// repository but not the worktree (no window has read it).
#[test]
#[cfg(unix)]
fn a_sleeping_worktree_session_is_opened_from_its_repository_while_no_list_shows_it() {
    use crate::agent_sleep::{DormantOwner, DormantPhase};
    let repo = super::device_worktrees::repo();
    let (mut runtime, _) = live_tab_order_runtime(&repo.root);
    runtime.ingest_session(Ok(session_in(&repo.linked, Some(4))));
    let id = dormant_intent(&mut runtime);
    {
        let record = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .get_mut(&id)
            .unwrap();
        assert_eq!(
            record.owner,
            Some(DormantOwner {
                repository_root: repo.root.clone(),
                is_git: true,
                label: "feature".into(),
            })
        );
        record.closed = true;
        record.phase = DormantPhase::Sleeping;
    }
    runtime.refresh_dormant_rows();
    runtime.ingest_session(Ok(no_workspaces()));
    assert!(listed(&runtime, &repo.root));
    assert!(!listed(&runtime, &repo.linked));
    assert!(runtime.snapshot.navigator.sleeping_sessions[0].wake_available);

    let fake = FakeEngine::start(|method, _| match method {
        "session" => reply(fake::session(serde_json::json!({}))),
        "open_worktree" => reply(crate::engine::OpenedWorktree {
            tab_label: "1".into(),
            workspace_id: "w9".into(),
            label: "linked".into(),
            tab_id: "w9:t1".into(),
            pane_id: "w9:p1".into(),
            already_open: false,
        }),
        "rename_workspace" => reply(()),
        // The owner is open; what follows is the ordinary wake tab.
        _ => refused("fixture_end", "the fixture stops after the owner"),
    });
    runtime.live.as_mut().unwrap().engine = fake.engine();
    let shared = Arc::new(std::sync::Mutex::new(runtime));
    shared.lock().unwrap().live.as_mut().unwrap().runtime = Arc::downgrade(&shared);
    shared.lock().unwrap().write_ui_state().unwrap();
    assert!(shared.lock().unwrap().request_dormant_wake(&id));
    wait_for("the wake tab's owner", || {
        fake.methods().contains(&"apply_layout")
    });
    let calls = fake.calls();
    let params = |wanted: &str| {
        calls
            .iter()
            .find(|(method, _)| *method == wanted)
            .map(|(_, params)| params.clone())
            .unwrap_or_else(|| panic!("no {wanted} in {calls:?}"))
    };
    assert_eq!(
        params("open_worktree"),
        serde_json::json!({"repository_root": repo.root, "path": repo.linked, "focus": true})
    );
    assert_eq!(params("rename_workspace")["label"], "feature");
}

/// A sleep keeps its checkout's owner as the list names it then, and a Wake
/// asked once the list no longer shows that checkout begins from the record.
#[test]
fn a_sleep_keeps_its_checkouts_owner_for_the_wake() {
    use crate::agent_sleep::{DormantOwner, DormantPhase};
    let mut runtime = close_pane_runtime("pi");
    assert!(runtime.begin_dormant_sleep(SLEEPER, 1));
    let id = {
        let (id, record) = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .iter_mut()
            .next()
            .unwrap();
        assert_eq!(
            record.owner,
            Some(DormantOwner {
                repository_root: CHECKOUT.into(),
                is_git: false,
                label: "order".into(),
            })
        );
        record.closed = true;
        record.phase = DormantPhase::Sleeping;
        id.clone()
    };
    // The close landed: no close of the pane is still waiting.
    runtime.close_operations.clear();
    runtime.refresh_dormant_rows();
    unlist(&mut runtime, CHECKOUT);
    assert!(runtime.snapshot.navigator.sleeping_sessions[0].wake_available);
    assert!(runtime.request_dormant_wake(&id));
    assert!(runtime.snapshot.status.last_error.is_none());
    // The wake's intent was saved and its tab is being opened in the
    // checkout the record names.
    assert_eq!(
        runtime.snapshot.ui_state.agent_sleep.dormant[&id].phase,
        DormantPhase::Creating
    );
}

/// A record an older build saved names no owner, so it wakes only while the
/// list shows its checkout; until then its row offers no Wake and says what
/// to do, instead of a Wake that is refused.
#[test]
fn a_sleeping_session_an_older_build_saved_offers_wake_only_while_its_checkout_is_listed() {
    use crate::agent_sleep::{DormantPhase, DormantRecord};
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let id = dormant_intent(&mut runtime);
    {
        let record = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .get_mut(&id)
            .unwrap();
        let mut saved = serde_json::to_value(&*record).unwrap();
        assert!(saved.as_object_mut().unwrap().remove("owner").is_some());
        *record = serde_json::from_value::<DormantRecord>(saved).unwrap();
        assert!(record.owner.is_none());
        record.closed = true;
        record.phase = DormantPhase::Sleeping;
    }
    let open = || tab_order_payload(CHECKOUT, &TABS[..1], &TABS[..1], "w-order:t1");
    runtime.ingest_session(Ok(open()));
    assert!(runtime.snapshot.navigator.sleeping_sessions[0].wake_available);

    // With no workspace open, the registration still lists the checkout;
    // removing the project takes it off the list, and the row follows at once.
    runtime.ingest_session(Ok(no_workspaces()));
    assert!(runtime.snapshot.navigator.sleeping_sessions[0].wake_available);
    assert!(runtime.retire_workspace_registration("workspace:order"));
    assert!(!listed(&runtime, CHECKOUT));
    let row = &runtime.snapshot.navigator.sleeping_sessions[0];
    assert!(!row.wake_available);
    let next = "This session's checkout is not listed. Open Hide on this computer and add its project if it is missing; Wake returns once the checkout is listed.";
    assert_eq!(row.reason.as_deref(), Some(next));
    assert!(runtime.request_dormant_wake(&id));
    let error = runtime.snapshot.status.last_error.clone().unwrap();
    assert_eq!(
        (error.kind.as_str(), error.message.as_str()),
        ("agent_sleep.wake_unavailable", next)
    );
    assert_eq!(
        runtime.snapshot.ui_state.agent_sleep.dormant[&id].phase,
        DormantPhase::Sleeping
    );

    // A wake that failed keeps saying why it failed.
    let failed = crate::agent_sleep_engine::WAKE_NOT_RESUMED;
    {
        let record = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .get_mut(&id)
            .unwrap();
        record.phase = DormantPhase::Failed;
        record.reason = Some(failed.into());
    }
    runtime.refresh_dormant_rows();
    let row = &runtime.snapshot.navigator.sleeping_sessions[0];
    assert!(!row.wake_available);
    assert_eq!(row.reason.as_deref(), Some(failed));

    // Its workspace open again, the list shows the checkout and Wake returns.
    runtime.ingest_session(Ok(open()));
    assert!(listed(&runtime, CHECKOUT));
    assert!(runtime.snapshot.navigator.sleeping_sessions[0].wake_available);
}

/// Wake is offered only where the wake can begin: over this machine's
/// connection to the engine.
#[test]
fn a_sleeping_session_offers_wake_only_over_a_connection() {
    use crate::agent_sleep::DormantPhase;
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let id = dormant_intent(&mut runtime);
    {
        let record = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .get_mut(&id)
            .unwrap();
        record.closed = true;
        record.phase = DormantPhase::Sleeping;
    }
    runtime.refresh_dormant_rows();
    assert!(runtime.snapshot.navigator.sleeping_sessions[0].wake_available);

    let live = runtime.live.take().unwrap();
    runtime.refresh_dormant_rows();
    let row = &runtime.snapshot.navigator.sleeping_sessions[0];
    assert!(!row.wake_available);
    let unavailable = "This session's reader, checkout or connection is unavailable";
    assert_eq!(row.reason.as_deref(), Some(unavailable));
    assert!(runtime.request_dormant_wake(&id));
    assert_eq!(
        runtime.snapshot.status.last_error.clone().unwrap().message,
        unavailable
    );

    runtime.set_live(live);
    assert!(runtime.snapshot.navigator.sleeping_sessions[0].wake_available);
}

/// An uncertain close is inspected through the real worker, never repaired.
#[test]
#[cfg(unix)]
fn checking_an_uncertain_sent_sleep_keeps_live_topology_and_sends_no_mutation() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let id = dormant_intent(&mut runtime);
    let record = runtime
        .snapshot
        .ui_state
        .agent_sleep
        .dormant
        .get_mut(&id)
        .unwrap();
    record.phase = crate::agent_sleep::DormantPhase::CloseUnknown;
    record.close_key = Some("saved-sent-close".into());
    runtime.refresh_dormant_rows();
    let fake = FakeEngine::start(|method, _| match method {
        "session" => reply(fake::session(dormant_wire_session(CHECKOUT, &TABS))),
        other => panic!("status inspection must not mutate the engine: {other}"),
    });
    runtime.live.as_mut().unwrap().engine = fake.engine();
    let shared = Arc::new(std::sync::Mutex::new(runtime));
    shared.lock().unwrap().live.as_mut().unwrap().runtime = Arc::downgrade(&shared);
    assert!(shared.lock().unwrap().request_dormant_status(&id));
    wait(&shared, "read-only dormant status", |runtime| {
        runtime.dormant_status_check.is_none()
    });
    let runtime = shared.lock().unwrap();
    assert_eq!(fake.methods(), ["session"]);
    assert_eq!(pane(&runtime)["id"], SLEEPER);
    let row = &runtime.snapshot.navigator.sleeping_sessions[0];
    assert_eq!(row.sleep_id, id);
    assert_eq!(row.phase, crate::agent_sleep::DormantPhase::CloseUnknown);
    assert!(!row.wake_available);
    assert!(!row.checking);
    assert_eq!(runtime.snapshot.recent_closed.count, 0);
}

/// A sleep that closes the only tab of the checkout in front is Hide's own
/// close, not the operator leaving: the screen stays where it is, unlike
/// the operator's close of that tab (docs/UI_BEHAVIOR.md, Agent areas).
#[test]
fn a_dormant_close_of_the_last_tab_in_front_never_moves_the_screen() {
    let (runtime, checkout) = live_tab_order_runtime(CHECKOUT);
    let mut runtime = super::workspace_view::with_new_views(runtime, "dormant-last-tab");
    let elsewhere = super::agent_areas::register_folder(&mut runtime, "elsewhere");
    let mut payload = session(Some(4));
    payload.tabs.retain(|tab| tab.tab_id == "w-order:t2");
    payload
        .layouts
        .retain(|layout| layout.tab_id == "w-order:t2");
    payload.panes.retain(|pane| pane.pane_id == SLEEPER);
    payload.workspaces[0].active_tab_id = Some("w-order:t2".into());
    payload.focused_pane_id = Some(SLEEPER.into());
    super::agent_areas::add_elsewhere_tab(&mut payload);
    runtime.ingest_session(Ok(payload));
    super::agent_areas::choose_checkout(&mut runtime, "workspace:elsewhere", &elsewhere);
    super::agent_areas::choose_checkout(&mut runtime, "workspace:order", &checkout);
    let id = dormant_intent(&mut runtime);
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .dormant
        .get_mut(&id)
        .unwrap()
        .phase = crate::agent_sleep::DormantPhase::Closing;

    runtime.close_local_pane(SLEEPER.into(), false, false);
    let operation = runtime.close_operations.values().next().expect("the close");
    assert!(!operation.request.retain_for_reopen);
    assert_eq!(
        runtime.snapshot().navigator.focused_checkout_id.as_deref(),
        Some(checkout.as_str())
    );
}

/// A retired sleep record cannot change the original close's purpose.
#[test]
fn a_dormant_close_never_becomes_reopenable_after_archive_retirement() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let id = dormant_intent(&mut runtime);
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .dormant
        .get_mut(&id)
        .unwrap()
        .phase = crate::agent_sleep::DormantPhase::Closing;
    let fake = FakeEngine::start(|method, _| match method {
        "export_layout" => reply(fake::closed_layout(serde_json::json!({
            "workspace_id":"w-order","tab_id":"w-order:t2","zoomed":false,"focused_pane_id":SLEEPER,
            "root":{"type":"pane","pane_id":SLEEPER,"cwd":CHECKOUT,"env":{}}
        }))),
        other => panic!("unexpected retired sleep effect: {other}"),
    });
    runtime.live.as_mut().unwrap().engine = fake.engine();
    runtime.close_local_pane(SLEEPER.into(), false, false);
    let key = runtime.snapshot.ui_state.agent_sleep.dormant[&id]
        .close_key
        .clone()
        .unwrap();
    let operation = runtime.close_operations.get_mut(&key).unwrap();
    operation.phase = "awaiting_topology".into();
    operation.item = Some(ClosedItem::Pane {
        key: key.clone(),
        context: operation.request.context.clone(),
        pane: operation
            .request
            .panes
            .iter()
            .find(|pane| pane.pane_id == SLEEPER)
            .unwrap()
            .clone(),
        placement: crate::recent_closed::PanePlacement {
            neighbor_pane_id: None,
            parent_path: vec![],
            direction: crate::recent_closed::ClosedSplitDirection::Right,
            ratio: 0.5,
            target_was_first: false,
        },
    });
    assert!(!operation.request.retain_for_reopen);
    runtime.snapshot.ui_state.agent_sleep.dormant.remove(&id);
    assert!(runtime.mark_close_topology_confirmed(&key));
    runtime.promote_close_reservations();
    assert_eq!(runtime.snapshot.recent_closed.count, 0);
    assert!(!runtime.snapshot.recent_closed.can_reopen);
    assert!(runtime.snapshot.recent_closed.pending.is_empty());
    fake.wait_for_calls(1, Duration::from_secs(5));
}

#[test]
fn a_failed_actual_state_write_never_sends_a_dormant_close() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let server = FakeEngine::start(|method, _| panic!("unexpected external effect: {method}"));
    runtime.live.as_mut().unwrap().engine = server.engine();
    let id = dormant_intent(&mut runtime);
    // A directory cannot be atomically replaced by the private state file.
    let blocked = tempfile::tempdir().unwrap();
    runtime.state_path = blocked.path().to_path_buf();
    assert!(runtime.write_ui_state().is_err());
    assert!(server.methods().is_empty());
    assert!(runtime.close_operations.is_empty());
    assert!(
        !runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .contains_key(&id)
    );
    assert_eq!(row(&runtime)["pane_id"], SLEEPER);
    assert!(runtime.snapshot.navigator.sleeping_sessions.is_empty());
}

/// The record of an agent the sleep closes is saved before its close, and
/// the save stands on the state folder's directory barrier, which every
/// system gives; Windows refused this save while the barrier was missing
/// there, so the sleep could never close an agent.
#[test]
fn a_dormant_intent_is_saved_through_the_state_folders_barrier() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let id = dormant_intent(&mut runtime);

    runtime.write_ui_state().unwrap();

    assert!(!persisted_dormant(&runtime.state_path, &id)["phase"].is_null());
}

#[test]
fn an_old_successful_save_cannot_close_a_replaced_execution() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let server = FakeEngine::start(|method, _| panic!("unexpected stale effect: {method}"));
    runtime.live.as_mut().unwrap().engine = server.engine();
    let id = dormant_intent(&mut runtime);
    let saved = runtime.snapshot.ui_state.agent_sleep.clone();
    runtime.ingest_session(Ok(session(Some(5))));
    runtime
        .snapshot
        .navigator
        .agents
        .iter_mut()
        .find(|agent| agent.pane_id == SLEEPER)
        .unwrap()
        .row_facts = Some(Default::default());
    assert!(runtime.ingest_dormant_saved(&saved, true));
    assert!(server.methods().is_empty());
    assert!(runtime.close_operations.is_empty());
    assert!(
        !runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .contains_key(&id)
    );
    assert_eq!(row(&runtime)["pane_id"], SLEEPER);
}

/// An interrupted save before close admission sent no close. Checking that
/// recovered intent must release it without touching the original execution.
#[test]
fn checking_an_unsent_recovered_sleep_leaves_the_pane_awake() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let server =
        FakeEngine::start(|method, _| panic!("an unsent sleep needs no external effect: {method}"));
    runtime.live.as_mut().unwrap().engine = server.engine();
    let id = dormant_intent(&mut runtime);
    runtime
        .snapshot
        .ui_state
        .agent_sleep
        .dormant
        .get_mut(&id)
        .unwrap()
        .after_load();
    runtime.refresh_dormant_rows();
    assert_eq!(runtime.snapshot.navigator.sleeping_sessions.len(), 1);
    assert!(runtime.dispatch_json(&event(
        "check_sleeping_session",
        serde_json::json!({"sleep_id": id})
    )));
    assert!(server.methods().is_empty());
    assert!(runtime.snapshot.ui_state.agent_sleep.dormant.is_empty());
    assert!(runtime.snapshot.navigator.sleeping_sessions.is_empty());
    assert_eq!(row(&runtime)["pane_id"], SLEEPER);
    assert!(row(&runtime).get("sleep").is_none());
}

/// A status answer captured for an intent cannot recreate it after a fresh
/// native session confirmation has already removed its archived row.
#[test]
fn a_late_sleep_status_answer_never_resurrects_a_cleared_intent() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let id = dormant_intent(&mut runtime);
    let record = runtime
        .snapshot
        .ui_state
        .agent_sleep
        .dormant
        .get_mut(&id)
        .unwrap();
    record.closed = true;
    record.phase = crate::agent_sleep::DormantPhase::WakeUnknown;
    let work = crate::agent_sleep_engine::DormantWork {
        id: id.clone(),
        record: record.clone(),
    };
    let generation = runtime.live_generation;
    runtime.dormant_status_check = Some((id.clone(), generation));
    runtime.snapshot.ui_state.agent_sleep.dormant.remove(&id);
    runtime.refresh_dormant_rows();
    runtime.ingest_dormant_status(&work, generation, Err("late answer".into()));
    assert!(runtime.snapshot.navigator.sleeping_sessions.is_empty());
    assert!(runtime.snapshot.ui_state.agent_sleep.dormant.is_empty());
    assert!(runtime.dormant_status_check.is_none());
    assert!(runtime.close_operations.is_empty());
}

/// The minute pass must publish the transition to Unknown even with Never
/// selected and no automatic sleep due. The outcome never resends a start.
#[test]
fn an_expired_dormant_operation_publishes_its_status_action_once() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    let id = dormant_intent(&mut runtime);
    let record = runtime
        .snapshot
        .ui_state
        .agent_sleep
        .dormant
        .get_mut(&id)
        .unwrap();
    record.closed = true;
    record.phase = crate::agent_sleep::DormantPhase::Starting;
    record.transition_started_unix_ms = 1;
    runtime.refresh_dormant_rows();
    assert!(runtime.tick_agent_sleep(180_002));
    let sleeping = &runtime.snapshot.navigator.sleeping_sessions[0];
    assert_eq!(
        sleeping.phase,
        crate::agent_sleep::DormantPhase::WakeUnknown
    );
    assert!(!sleeping.wake_available);
    assert!(!runtime.tick_agent_sleep(180_003));
    assert!(runtime.close_operations.is_empty());
}

/// B2: the setting is one event, survives a restart, and a shared UI-state
/// save does not reset it; a value outside the choices is refused. A 24
/// chosen now stays 24, though an earlier build's 24 was its default.
#[test]
fn the_sleep_setting_survives_a_restart_and_a_ui_state_update() {
    let mut runtime = runtime();
    let path = runtime.state_path.clone();
    // The test keeps the state folder: it restarts on that file after the
    // runtime is gone.
    let _state = hold_dirs(&mut runtime);
    assert!(runtime.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": 24})
    )));
    assert_eq!(
        serde_json::to_value(runtime.snapshot()).unwrap()["ui_state"]["agent_sleep_after_hours"],
        24
    );
    assert!(runtime.dispatch_json(&event(
        "ui_state_update",
        serde_json::json!({"accent_hex": "#7DD3FC", "font_size": 14})
    )));
    assert_eq!(
        runtime.snapshot().ui_state.agent_sleep_after_hours,
        Some(24)
    );

    for refused in [5, 48] {
        runtime.dispatch_json(&event(
            "agent_sleep_set",
            serde_json::json!({"after_hours": refused}),
        ));
        assert_eq!(
            runtime
                .snapshot()
                .status
                .last_error
                .as_ref()
                .map(|error| error.kind.as_str()),
            Some("agent_sleep.invalid_setting")
        );
    }
    assert_eq!(
        runtime.snapshot().ui_state.agent_sleep_after_hours,
        Some(24)
    );
    drop(runtime);

    let mut restarted = restarted_on(&path);
    assert_eq!(
        restarted.snapshot().ui_state.agent_sleep_after_hours,
        Some(24)
    );
    assert!(restarted.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": null})
    )));
    assert_eq!(restarted.snapshot().ui_state.agent_sleep_after_hours, None);
    drop(restarted);
    // An Off the operator chose is kept as Off across a restart.
    assert_eq!(
        restarted_on(&path)
            .snapshot()
            .ui_state
            .agent_sleep_after_hours,
        None
    );
}

/// A new store sleeps idle agents after 12 hours. A store an earlier build
/// wrote moves its choice once: no value and its 24-hour default become 12,
/// the 48 and 72 hours that are gone become 24, 12 stays, and its Off (0)
/// stays Off. A choice is written under its own key, Off as 0, and the
/// earlier key is not written again.
#[test]
fn sleep_defaults_to_12_hours_and_an_earlier_stores_choice_moves_once() {
    let mut runtime = runtime();
    let path = runtime.state_path.clone();
    let _state = hold_dirs(&mut runtime);
    assert_eq!(
        runtime.snapshot().ui_state.agent_sleep_after_hours,
        Some(12)
    );
    assert!(runtime.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": null})
    )));
    drop(runtime);
    let state: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(state["agent_sleep_hours"], 0);
    assert!(state.get("agent_sleep_after_hours").is_none());

    // The store an earlier build wrote, with each value it could hold.
    let earlier = |value: serde_json::Value| {
        let mut state = state.clone();
        state.as_object_mut().unwrap().remove("agent_sleep_hours");
        state["agent_sleep_after_hours"] = value;
        std::fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
        restarted_on(&path)
            .snapshot()
            .ui_state
            .agent_sleep_after_hours
    };
    assert_eq!(earlier(serde_json::Value::Null), Some(12));
    assert_eq!(earlier(24.into()), Some(12));
    assert_eq!(earlier(12.into()), Some(12));
    assert_eq!(earlier(48.into()), Some(24));
    assert_eq!(earlier(72.into()), Some(24));
    assert_eq!(earlier(0.into()), None);
}

/// A runtime started again on the state file at `path`, as a relaunch does.
fn restarted_on(path: &std::path::Path) -> Runtime {
    Runtime::new(
        CoreOptions {
            schema_version: SCHEMA_VERSION,
            home: None,
            node_id: crate::node::test_node(),
            app_state_path: path.to_string_lossy().into_owned(),
            host_helper_root: None,
            host_cli_dir: None,
            workspace_views_path: None,
            shortcut_import_path: None,
            local_issues_path: None,
            effects_held: false,
            machine_name: None,
            account_state_folder: false,
        },
        environment::EnvironmentReport {
            statuses: Vec::new(),
            home_path: None,
            codex_home: None,
        },
        std::sync::Arc::new(hide_node::Local::of_process()),
        crate::node::test_devices(),
    )
}

/// B10, B20: a sleeping agent keeps its row, name and place, with its moon
/// (status-marks D-21, B31);
/// an awake row carries no `sleep` key, so an older reader's decode holds.
#[test]
fn a_slept_agent_keeps_its_row_after_the_engine_forgets_it() {
    let (mut runtime, _) = asleep();
    runtime.ingest_session(Ok(session(None)));
    let slept = row(&runtime);
    assert_eq!(
        (
            &slept["id"],
            &slept["agent_kind"],
            &slept["identity_label"],
            &slept["mark"],
            &slept["status_code"],
            &slept["group"],
            &slept["sleep"]["state"],
        ),
        (
            &serde_json::json!("reviewer"),
            &serde_json::json!("claude"),
            &serde_json::json!("Review the parser"),
            &serde_json::json!("asleep"),
            &serde_json::json!("sleeping"),
            &serde_json::json!("seen"),
            &serde_json::json!("sleeping"),
        )
    );
    let pane = pane(&runtime);
    assert_eq!(pane["sleep"]["state"], "sleeping");
    assert!(
        pane.get("sleep_action").is_none(),
        "a sleeping pane offers Wake, not Sleep"
    );
    assert!(
        serde_json::to_value(runtime.snapshot()).unwrap()["ui_state"]
            .get("agent_sleep")
            .is_none(),
        "the records stay off the wire"
    );
}

/// A sleeping agent has ended its process, so no hook can speak from its
/// pane: the pane is not a session Hide cannot hear, and the agent's row in
/// Settings does not count it among the sessions running now.
#[test]
fn a_sleeping_agent_is_neither_counted_nor_called_not_connected() {
    use super::agent_connection::{connection_of, diagnosis, installed, kit_rows, sessions_of};
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    runtime.ingest_session(Ok(session(Some(4))));
    // Awake, the session started before the hook: it runs and is judged.
    assert_eq!(sessions_of(&runtime, "claude-code"), Some(1));
    assert_eq!(
        connection_of(&runtime, SLEEPER).and_then(|connection| connection.reason),
        Some(crate::model::PaneConnectionReason::StartedBeforeHide)
    );

    runtime.dispatch_json(&event(
        "agent_sleep",
        serde_json::json!({"pane_id": SLEEPER}),
    ));
    assert!(runtime.ingest_agent_sleep_end(SLEEPER, Ok(4)));
    runtime.ingest_session(Ok(session(None)));
    assert_eq!(pane(&runtime)["sleep"]["state"], "sleeping");
    assert_eq!(connection_of(&runtime, SLEEPER), None);
    assert_eq!(sessions_of(&runtime, "claude-code"), Some(0));
}

/// #268: a sleep record written before labels carried an owner proves no
/// session, so its row falls back to the provider title with no progress.
#[test]
fn a_legacy_sleep_record_without_a_label_owner_shows_no_stale_label() {
    let (mut runtime, _) = asleep();
    let record = runtime
        .snapshot
        .ui_state
        .agent_sleep
        .records
        .get_mut(SLEEPER)
        .expect("the sleep record");
    assert!(
        record.label_owner.is_some(),
        "a current row stamps its owner"
    );
    record.label_owner = None;
    record.progress = Some("Split the lexer".to_owned());
    runtime.ingest_session(Ok(session(None)));
    let slept = row(&runtime);
    assert_eq!(slept["sleep"]["state"], "sleeping");
    assert_eq!(slept["identity_label"], "Claude");
    assert!(slept.get("progress").is_none_or(serde_json::Value::is_null));
}

/// B12: typed input to a sleeping pane reaches no terminal: the pane's node
/// is told it sleeps, and drops its keys until it wakes
/// (`hide-node` `keys_for_a_sleeping_pane_are_dropped_until_it_wakes`).
#[test]
fn input_to_a_sleeping_pane_goes_nowhere() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    let terminals = record_terminals(&mut runtime);
    runtime.ingest_session(Ok(session(Some(4))));
    runtime.dispatch_json(&event(
        "agent_sleep",
        serde_json::json!({"pane_id": SLEEPER}),
    ));
    let asleep = TerminalControl::Asleep {
        pane: SLEEPER.into(),
        asleep: true,
    };
    // The end has begun, so no key can land behind it: a key typed now would
    // run in the shell the agent leaves, however the pane is still drawn.
    assert!(terminals.take().contains(&asleep));
    assert!(runtime.ingest_agent_sleep_end(SLEEPER, Ok(4)));
    assert!(!terminals.take().contains(&TerminalControl::Asleep {
        pane: SLEEPER.into(),
        asleep: false,
    }));
}

/// Every writer that asks the gate (a phone's reply, an agent's find keys)
/// is refused from the moment the end begins, through asleep, until the
/// agent is back; an end the agent refuses opens the pane again.
#[test]
fn the_input_gate_follows_the_sleep_records_from_the_first_moment_of_the_end() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    let gate = runtime.input_gate();
    runtime.ingest_session(Ok(session(Some(4))));
    assert!(gate.admit(SLEEPER).is_ok(), "an awake agent takes input");

    runtime.dispatch_json(&event(
        "agent_sleep",
        serde_json::json!({"pane_id": SLEEPER}),
    ));
    assert!(
        row(&runtime).get("sleep").is_none(),
        "the row still draws the agent awake while the end runs"
    );
    assert!(gate.admit(SLEEPER).is_err(), "ending refuses input");
    assert!(gate.admit("w1:other").is_ok(), "another pane is not closed");
    assert!(runtime.ingest_agent_sleep_end(SLEEPER, Ok(4)));
    assert!(gate.admit(SLEEPER).is_err(), "asleep refuses input");

    let (mut refused, _) = live_tab_order_runtime(CHECKOUT);
    let gate = refused.input_gate();
    refused.ingest_session(Ok(session(Some(4))));
    refused.dispatch_json(&event(
        "agent_sleep",
        serde_json::json!({"pane_id": SLEEPER}),
    ));
    assert!(gate.admit(SLEEPER).is_err());
    refused.ingest_agent_sleep_end(SLEEPER, Err("did not hand the terminal back".into()));
    assert!(
        gate.admit(SLEEPER).is_ok(),
        "an agent that stayed awake takes input again"
    );
}

/// A restarted engine lists an agent only once its program shows again, so
/// the first reads after a daemon start hold none for a pane that has one: the
/// pane's last look, which keeps an old conversation from sleeping an agent the
/// operator just read, must outlive them (with-hide/hide-private#682).
#[test]
fn a_read_that_lists_no_agent_for_a_pane_keeps_the_panes_last_look() {
    let (mut runtime, checkout_id) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    let seen = |runtime: &Runtime| {
        runtime
            .snapshot()
            .ui_state
            .agent_sleep
            .stamps
            .get(SLEEPER)
            .map(|stamps| stamps.seen_at_unix_ms)
    };
    let looked = seen(&runtime).expect("the visit stamped the pane");
    assert!(looked > 0);

    runtime.ingest_session(Ok(session(None)));
    assert_eq!(seen(&runtime), Some(looked), "the pane is still there");
    runtime.ingest_session(Ok(session(Some(1))));
    assert_eq!(seen(&runtime), Some(looked), "the agent is back");

    let gone = tab_order_payload(CHECKOUT, &["w-order:t1"], &["w-order:t1"], "w-order:t1");
    runtime.ingest_session(Ok(gone));
    assert_eq!(
        seen(&runtime),
        None,
        "a pane the topology lost takes its stamp"
    );
}

/// B12, D-12: a committed visit to the tab wakes the agent; another event
/// in the same tab is not a visit, and a second wake starts nothing.
#[test]
fn visiting_the_tab_wakes_the_agent_once() {
    let (mut runtime, checkout_id) = asleep();
    runtime.ingest_session(Ok(session(None)));
    assert!(runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2")));
    assert_eq!(row(&runtime)["sleep"]["state"], "waking");
    assert_eq!(row(&runtime)["status_code"], "waking");
    assert!(!runtime.dispatch_json(&event(
        "agent_wake",
        serde_json::json!({"pane_id": SLEEPER})
    )));
}

/// Coexistence B3: a saved conversation remains asleep and named when its
/// terminal failed cold restoration, even if stale agent metadata changed.
#[test]
fn visiting_a_failed_terminal_preserves_sleep_without_starting_an_agent() {
    let (mut runtime, checkout_id) = asleep();
    let identity = row(&runtime)["identity_label"].clone();
    let mut failed = session(Some(99));
    failed
        .panes
        .iter_mut()
        .find(|pane| pane.pane_id == SLEEPER)
        .unwrap()
        .failed_terminal_id = Some("failed-terminal".into());
    runtime.ingest_session(Ok(failed));
    runtime.dispatch_json(&focus_tab_event(&checkout_id, "w-order:t2"));
    assert_eq!(row(&runtime)["sleep"]["state"], "sleeping");
    assert_eq!(row(&runtime)["identity_label"], identity);
    assert_eq!(pane(&runtime)["failed_terminal_id"], "failed-terminal");
    assert!(!runtime.dispatch_json(&event(
        "agent_wake",
        serde_json::json!({"pane_id": SLEEPER})
    )));
    assert_eq!(row(&runtime)["sleep"]["state"], "sleeping");
}

/// B6: a close refusal or lost connection cannot erase the saved failed row.
#[test]
fn failed_terminal_close_errors_keep_the_sleeping_row_and_failure() {
    for (error, phase) in [
        (
            crate::engine::EngineError::Refused {
                code: "refused".into(),
                message: "close refused".into(),
            },
            "refused",
        ),
        (
            crate::engine::EngineError::Transport("acknowledgement timed out".into()),
            "unknown",
        ),
        (
            crate::engine::EngineError::NotRunning("device disconnected".into()),
            "unknown",
        ),
    ] {
        let (mut runtime, _) = asleep();
        let identity = row(&runtime)["identity_label"].clone();
        let mut failed = session(None);
        failed
            .panes
            .iter_mut()
            .find(|pane| pane.pane_id == SLEEPER)
            .unwrap()
            .failed_terminal_id = Some("failed-terminal".into());
        runtime.ingest_session(Ok(failed));
        let mut request = close_capture_request("failed-sleep");
        request.target = crate::recent_closed::capture::CloseCaptureTarget::Pane {
            pane_id: SLEEPER.into(),
        };
        request.context.tab_id = "w-order:t2".into();
        request.context.pane_ids_before_close = vec![SLEEPER.into()];
        request.connection_generation = runtime.live_generation;
        request.failed_terminals = vec![(SLEEPER.into(), "failed-terminal".into())];
        runtime.ensure_pending_close_from_request(&request);
        let effect = runtime.prepare_captured_close_effect(&request.key).unwrap();
        runtime.ingest_close_effect_result(&effect, Err(error));
        assert_eq!(row(&runtime)["identity_label"], identity);
        assert_eq!(row(&runtime)["sleep"]["state"], "sleeping");
        assert_eq!(pane(&runtime)["failed_terminal_id"], "failed-terminal");
        if phase == "unknown" {
            assert_eq!(runtime.snapshot().recent_closed.pending[0].phase, phase);
        } else {
            assert!(runtime.snapshot().recent_closed.pending.is_empty());
        }
        assert_eq!(
            runtime.snapshot().recent_closed.notices[0]
                .pane_id
                .as_deref(),
            Some(SLEEPER)
        );
    }
}

/// B14: a failed wake says why in plain words and offers Retry and Start
/// new session; the engine's own words stay in the log.
#[test]
fn a_failed_wake_says_why_and_can_be_retried_fresh() {
    let (mut runtime, _) = asleep();
    runtime.ingest_session(Ok(session(None)));
    assert!(runtime.dispatch_json(&event(
        "agent_wake",
        serde_json::json!({"pane_id": SLEEPER})
    )));
    assert!(runtime.ingest_agent_wake(
        SLEEPER,
        crate::agent_sleep::WakeMode::Resume,
        crate::agent_sleep_engine::WakeOutcome::Failed {
            reason: crate::agent_sleep_engine::WAKE_NOT_RESUMED.into(),
            detail: "agent_start_failed: exited".into(),
            passing: false,
        },
    ));
    let failed = row(&runtime);
    assert_eq!(failed["sleep"]["state"], "failed");
    assert_eq!(
        failed["sleep"]["reason"],
        crate::agent_sleep_engine::WAKE_NOT_RESUMED
    );
    assert_eq!(failed["status_code"], "sleep_failed");
    assert!(runtime.dispatch_json(&event(
        "agent_wake",
        serde_json::json!({"pane_id": SLEEPER, "fresh": true})
    )));
    assert_eq!(row(&runtime)["sleep"]["state"], "waking");
}

#[test]
fn a_wake_counts_as_a_look_so_its_old_conversation_does_not_sleep_it_again() {
    let (mut runtime, _) = asleep();
    let seen =
        |runtime: &Runtime| runtime.snapshot().ui_state.agent_sleep.stamps[SLEEPER].seen_at_unix_ms;
    let before = unix_milliseconds();
    assert!(seen(&runtime) < before);
    assert!(runtime.dispatch_json(&event(
        "agent_wake",
        serde_json::json!({"pane_id": SLEEPER})
    )));
    assert_eq!(row(&runtime)["sleep"]["state"], "waking");
    assert!(seen(&runtime) >= before);
}

/// The engine answers a wake's start once it has typed the start line; a line the
/// pane's shell never ran (a Windows prompt reading a leftover bracketed
/// paste as keys) brings no agent, and the wake fails with a reason once the
/// start's deadline has passed rather than reading waking for good.
#[test]
fn a_wake_whose_agent_never_appears_fails_at_the_starts_deadline() {
    let (mut runtime, _) = asleep();
    runtime.ingest_session(Ok(session(None)));
    assert!(runtime.dispatch_json(&event(
        "agent_wake",
        serde_json::json!({"pane_id": SLEEPER})
    )));
    let started = unix_milliseconds();
    runtime.ingest_agent_wake(
        SLEEPER,
        crate::agent_sleep::WakeMode::Resume,
        crate::agent_sleep_engine::WakeOutcome::Started,
    );
    runtime.tick_agent_sleep(started + 30_000);
    assert_eq!(
        row(&runtime)["sleep"]["state"],
        "waking",
        "inside the deadline"
    );

    assert!(runtime.tick_agent_sleep(started + 130_000));
    let failed = row(&runtime);
    assert_eq!(failed["sleep"]["state"], "failed");
    assert_eq!(
        failed["sleep"]["reason"],
        crate::agent_sleep_engine::WAKE_NOT_READY
    );
    assert_eq!(failed["status_code"], "sleep_failed");
}

/// B16, B17, B18: a new agent in the pane is awake whatever started it,
/// and a closed pane takes its record with it.
#[test]
fn a_new_agent_clears_the_sleep_and_a_closed_pane_drops_it() {
    let (mut runtime, _) = asleep();
    runtime.ingest_session(Ok(session(Some(4))));
    assert_eq!(
        row(&runtime)["sleep"]["state"],
        "sleeping",
        "the ended agent still listed"
    );
    runtime.ingest_session(Ok(session(Some(9))));
    assert!(row(&runtime).get("sleep").is_none());
    assert_ne!(row(&runtime)["status_code"], "sleeping");

    let (mut runtime, _) = asleep();
    let mut closed = tab_order_payload(CHECKOUT, &TABS[..1], &TABS[..1], "w-order:t1");
    closed.agents.clear();
    runtime.ingest_session(Ok(closed));
    assert!(runtime.snapshot().ui_state.agent_sleep.records.is_empty());
}

/// B18: a restarted engine can reuse the ended agent's sequence.
#[test]
fn a_restarted_runtime_wakes_a_sleeper_when_its_engine_reuses_the_sequence() {
    // A pane need not carry hide_server yet: the connection owns its mark.
    // Older saved records carried no mark either.
    for legacy in [false, true] {
        let (mut runtime, _) = asleep();
        runtime.ingest_session(Ok(session(Some(4))));
        assert_eq!(row(&runtime)["sleep"]["state"], "sleeping");
        let path = runtime.state_path.clone();
        let _state = hold_dirs(&mut runtime);
        drop(runtime);
        if legacy {
            let mut state: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            state["agent_sleep"]["records"][SLEEPER]
                .as_object_mut()
                .unwrap()
                .remove("server_mark");
            std::fs::write(&path, serde_json::to_vec(&state).unwrap()).unwrap();
        }
        let mut restarted = restarted_on(&path);
        restarted.begin_read_connection(None);
        let mut starting = session(Some(4));
        starting.agents[0].start_unshown = true;
        restarted.ingest_session(Ok(starting));
        assert_eq!(row(&restarted)["sleep"]["state"], "sleeping");

        restarted.ingest_session(Ok(session(Some(4))));
        assert!(
            row(&restarted).get("sleep").is_none(),
            "a shown agent after restart is new even when its sequence repeats (legacy={legacy})"
        );
        assert!(restarted.snapshot().ui_state.agent_sleep.records.is_empty());
    }
}

/// B8: an end that did not land leaves the agent awake.
#[test]
fn an_end_that_fails_leaves_the_agent_awake() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    runtime.dispatch_json(&event(
        "agent_sleep",
        serde_json::json!({"pane_id": SLEEPER}),
    ));
    runtime.ingest_agent_sleep_end(SLEEPER, Err("did not hand the terminal back".into()));
    assert!(runtime.snapshot().ui_state.agent_sleep.records.is_empty());
    assert!(row(&runtime).get("sleep").is_none());
    assert_ne!(row(&runtime)["status_code"], "sleeping");
}

/// B4-B6: the minute decision ends an agent that sat seen and off screen
/// past the chosen hours, never the one on screen, at most once a minute,
/// and not at all while the setting is Never.
#[test]
fn the_minute_decision_sleeps_only_an_off_screen_agent_past_the_chosen_hours() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    let mut payload = session(Some(4));
    payload.agents.push(
        crate::sidebar::owned_label_fixture(serde_json::json!({
            "pane_id": "w-order:t1:p", "agent": "codex", "agent_status": "idle",
            "state_change_seq": 2, "cwd": CHECKOUT,
            "agent_session": {"kind": "id", "value": "on-screen-session"}
        }))
        .unwrap(),
    );
    runtime.ingest_session(Ok(payload));
    let later = unix_milliseconds() + 13 * 60 * 60 * 1000;
    let ending = |runtime: &Runtime| {
        runtime
            .snapshot()
            .ui_state
            .agent_sleep
            .records
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };

    runtime.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": null}),
    ));
    assert!(!runtime.tick_agent_sleep(later));
    assert!(ending(&runtime).is_empty(), "Never sleeps nothing");

    runtime.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": 24}),
    ));
    runtime.tick_agent_sleep(later);
    assert!(ending(&runtime).is_empty(), "13 hours is not 24");

    runtime.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": 12}),
    ));
    runtime.tick_agent_sleep(later);
    assert_eq!(ending(&runtime), [SLEEPER], "t1 is on screen, t2 is not");
    assert!(
        row(&runtime).get("sleep").is_none(),
        "awake until the end lands"
    );
}

/// PRD core-host-node-move amendment 3: a core waiting for its move's link
/// puts no due agent to sleep, and the same minute decision does once the
/// link committed it.
#[test]
fn a_pending_core_puts_no_agent_to_sleep_until_its_link_commits() {
    let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
    runtime.ingest_session(Ok(session(Some(4))));
    runtime.dispatch_json(&event(
        "agent_sleep_set",
        serde_json::json!({"after_hours": 12}),
    ));
    runtime.hold_effects();
    let later = unix_milliseconds() + 13 * 60 * 60 * 1000;
    let ending = |runtime: &Runtime| {
        runtime
            .snapshot()
            .ui_state
            .agent_sleep
            .records
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };

    assert!(!runtime.tick_agent_sleep(later));
    assert!(ending(&runtime).is_empty());

    assert!(runtime.release_effects());
    runtime.tick_agent_sleep(later);
    assert_eq!(ending(&runtime), [SLEEPER]);
}

const WOKEN: &str = "w-order:t3:p";
const CHILD_NATIVE: &str = "child-native";

pub(super) fn delivery_agent(id: &str, pane: &str, kind: &str, session: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id, "pane_id": pane, "agent": kind, "agent_status": "idle",
        "state_change_seq": 4, "lineage_session": crate::engine::tokens::session_digest(session)
    })
}

pub(super) fn delivery_panes(agents: Vec<serde_json::Value>) -> SessionSnapshotPayload {
    serde_json::from_value(serde_json::json!({ "agents": agents })).unwrap()
}

/// A lead that delegated a child and watches it, the child asleep with its
/// pane closed, and the delivery store running over a ledger on disk.
struct SleepingChild {
    shared: Arc<std::sync::Mutex<Runtime>>,
    client: crate::delivery::worker::Client,
    worker: crate::delivery::worker::Worker,
    _root: tempfile::TempDir,
    ledger_path: PathBuf,
    node: String,
    id: crate::agent_sleep::SleepId,
    child: crate::delivery::Actor,
    letter: String,
}

impl SleepingChild {
    fn new() -> Self {
        use crate::coordination::AgentRecord;
        let root = tempfile::tempdir().unwrap();
        let ledger_path = root.path().join("delivery-ledger.json");
        let (mut runtime, _) = live_tab_order_runtime(CHECKOUT);
        let node = runtime.node.as_str().to_owned();
        runtime.ingest_session(Ok(session(Some(4))));
        runtime
            .snapshot
            .navigator
            .agents
            .iter_mut()
            .find(|agent| agent.pane_id == SLEEPER)
            .unwrap()
            .row_facts
            .get_or_insert_with(Default::default);
        let awake = || {
            delivery_panes(vec![
                delivery_agent("lead", "lead", "codex", "lead-native"),
                delivery_agent("reviewer", SLEEPER, "claude", CHILD_NATIVE),
            ])
        };
        runtime.observe_delivery_laid(&node, &awake(), Some("scope"), None);
        let mut ledger = crate::delivery::ledger::Ledger::default();
        for (id, pane, parent) in [
            ("agent-1", "lead", None),
            ("agent-2", SLEEPER, Some("agent-1")),
        ] {
            ledger.agents.push(AgentRecord {
                id: id.into(),
                name: Some(
                    (if pane == SLEEPER {
                        "reviewer"
                    } else {
                        "lead-agent"
                    })
                    .into(),
                ),
                machine: node.clone(),
                host_scope: "scope".into(),
                native_machine: "fixture-machine".into(),
                session: format!("{}-native", if pane == SLEEPER { "child" } else { "lead" }),
                instance: pane.into(),
                pane: pane.into(),
                parent: parent.map(Into::into),
                origin: None,
                project: None,
                actor: runtime.delivery_observations[pane].actor.clone(),
                ended: false,
                terminal_id: None,
                forked: false,
            });
        }
        ledger.next_id = 3;
        let lead = ledger.agents[0].actor.clone();
        let child = ledger.agents[1].actor.clone();
        let at = crate::delivery::worker::now();
        crate::delivery::watch::start(&mut ledger, &lead, &child, at).unwrap();
        let letter = crate::delivery::mailbox::send(
            &mut ledger,
            &lead,
            &child,
            "ask",
            "review the parser",
            "request",
            None,
            at,
        )
        .unwrap();
        ledger.validate().unwrap();
        runtime.delivery_ledger = Ok(Arc::new(ledger));
        // The engine's panes are read once the ledger can judge a pane that left.
        runtime.observe_delivery_laid(&node, &awake(), Some("scope"), None);

        // The sleep saves the registration it closes the pane under.
        assert!(runtime.begin_dormant_sleep(SLEEPER, 1));
        assert!(runtime.snapshot.status.last_error.is_none());
        let (id, saved) = {
            let (id, record) = runtime
                .snapshot
                .ui_state
                .agent_sleep
                .dormant
                .iter()
                .next()
                .unwrap();
            (id.clone(), record.registration.clone().unwrap())
        };
        assert_eq!((saved.id.as_str(), &saved.actor), ("agent-2", &child));
        let record = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .get_mut(&id)
            .unwrap();
        record.closed = true;
        record.phase = crate::agent_sleep::DormantPhase::Sleeping;
        // The folder it sleeps in stands, so the watch on it waits for the
        // wake rather than ending as for a deleted checkout.
        record.cwd = root.path().to_str().unwrap().to_owned();
        runtime.refresh_dormant_rows();
        assert_eq!(runtime.snapshot.navigator.sleeping_sessions.len(), 1);

        let shared = Arc::new(std::sync::Mutex::new(runtime));
        // The store alone, so the test's passes are the ones that run: the
        // doorbell and watch producers pass on their own clocks, and one of
        // theirs between the wake and the test's look moved the registration
        // before the test saw it asked for, or wrote the file while the test
        // compared it with the published ledger.
        let (worker, client) = crate::delivery::worker::Worker::store(
            Arc::downgrade(&shared),
            crate::handle::ChangeNotifier::noop(),
            ledger_path.clone(),
        )
        .unwrap();
        Self {
            shared,
            client,
            worker,
            _root: root,
            ledger_path,
            node,
            id,
            child,
            letter: letter.id,
        }
    }

    /// A pass of the delivery store over what the runtime has asked of it.
    fn pass(&self) {
        self.client
            .submit(
                crate::delivery::worker::Effect::HumanClaim,
                Duration::from_secs(5),
            )
            .unwrap();
    }

    /// The engine no longer lists the closed pane.
    fn pane_closed(&self) {
        self.shared.lock().unwrap().observe_delivery_laid(
            &self.node,
            &delivery_panes(vec![delivery_agent("lead", "lead", "codex", "lead-native")]),
            Some("scope"),
            None,
        );
    }

    /// The core started the conversation again in a new pane, which the engine
    /// lists as an agent of `session` and the session read shows. It runs
    /// under the caller's lock, so the caller can look at the wake before the
    /// store's own upkeep, which runs while a move is asked for, can pass.
    fn woken(&self, runtime: &mut Runtime, session: &str) {
        let record = runtime
            .snapshot
            .ui_state
            .agent_sleep
            .dormant
            .get_mut(&self.id)
            .unwrap();
        record.phase = crate::agent_sleep::DormantPhase::Starting;
        record.wake_pane_id = Some(WOKEN.into());
        record.wake_tab_id = Some("w-order:t3".into());
        runtime.observe_delivery_laid(
            &self.node,
            &delivery_panes(vec![
                delivery_agent("lead", "lead", "codex", "lead-native"),
                delivery_agent("reviewer", WOKEN, "claude", session),
            ]),
            Some("scope"),
            None,
        );
        let mut confirmed = tab_order_payload(
            CHECKOUT,
            &["w-order:t1", "w-order:t3"],
            &["w-order:t1", "w-order:t3"],
            "w-order:t1",
        );
        let mut agent = session_agent();
        agent.pane_id = Some(WOKEN.into());
        confirmed.agents.push(agent);
        runtime.ingest_session(Ok(confirmed));
    }

    /// After the pass that moved it, the woken pane holds the registration,
    /// the letter and the watch, in the published ledger and in the file.
    fn assert_handed_over(self) {
        // Stopped first, so the file is the one the store's last pass wrote.
        drop(self.worker);
        let runtime = self.shared.lock().unwrap();
        // The pass answers after it publishes, and the publish settles the
        // record, so there is nothing to wait for.
        assert!(runtime.snapshot.ui_state.agent_sleep.dormant.is_empty());
        assert!(runtime.delivery_registration_rebinds().is_empty());
        let ledger = runtime.delivery_state().unwrap();
        let record = ledger
            .agents
            .iter()
            .find(|record| record.id == "agent-2")
            .unwrap();
        assert!(!record.ended);
        assert_eq!(record.pane, WOKEN);
        assert_eq!(record.instance, WOKEN);
        assert_eq!(record.parent.as_deref(), Some("agent-1"));
        assert_eq!(record.actor.pane_id, WOKEN);
        assert_eq!(record.actor.session, self.child.session);
        assert_eq!(ledger.watches.len(), 1);
        assert_eq!(ledger.watches[0].target.pane_id, WOKEN);
        let letter = ledger
            .letters
            .iter()
            .find(|letter| letter.id == self.letter)
            .unwrap();
        assert_eq!(letter.recipient.pane_id, WOKEN);
        assert_eq!(letter.state, crate::delivery::ledger::State::Pending);
        assert_eq!(
            crate::delivery::ledger::load(&self.ledger_path).unwrap(),
            *ledger
        );
        // The old pane stays closed: only the woken one is a live address now.
        assert!(crate::coordination::resolve_actor(&ledger, SLEEPER).is_none());
        assert_eq!(
            crate::coordination::resolve_actor(&ledger, "reviewer")
                .unwrap()
                .pane_id,
            WOKEN
        );
    }
}

/// The woken pane's agent as the session read lists it: the saved
/// conversation, with the facts that confirm it.
fn session_agent() -> crate::sidebar::SessionAgentPayload {
    let mut agent = session(Some(5)).agents.remove(0);
    agent.facts = Some(crate::request_view::RowFacts {
        native_session_id: Some("11111111-2222-3333-4444-555555555555".into()),
        ..Default::default()
    });
    agent
}

/// A delegated child put to sleep closes its pane and wakes in another. The
/// registration, the letters addressed to it and the watch on it belong to
/// the conversation, so the pane that wakes it takes them over: nothing is
/// guessed from a name, and nothing ends while it sleeps.
#[test]
fn a_sleeping_childs_registration_letters_and_watch_pass_to_the_pane_that_wakes_it() {
    let sleeping = SleepingChild::new();

    // While it sleeps, the registration and the watch on it stay, and
    // nothing is queued to end them.
    sleeping.pane_closed();
    {
        let mut runtime = sleeping.shared.lock().unwrap();
        assert!(runtime.delivery_registrations_gone().is_empty());
        assert!(runtime.delivery_watch_work().iter().all(|work| !work.gone));
    }
    sleeping.pass();
    {
        let runtime = sleeping.shared.lock().unwrap();
        let ledger = runtime.delivery_state().unwrap();
        assert!(ledger.agents.iter().all(|record| !record.ended));
        assert_eq!(ledger.watches.len(), 1);
        assert_eq!(
            ledger.letters[0].state,
            crate::delivery::ledger::State::Pending
        );
    }

    {
        // The record stays until the ledger shows the pane took the
        // registration over, and it asks for exactly that move.
        let mut runtime = sleeping.shared.lock().unwrap();
        sleeping.woken(&mut runtime, CHILD_NATIVE);
        assert!(
            runtime
                .snapshot
                .ui_state
                .agent_sleep
                .dormant
                .contains_key(&sleeping.id)
        );
        assert!(runtime.snapshot.navigator.sleeping_sessions.is_empty());
        let rebinds = runtime.delivery_registration_rebinds();
        assert_eq!(rebinds.len(), 1);
        assert_eq!(rebinds["agent-2"].from, sleeping.child);
        assert_eq!(rebinds["agent-2"].to.pane_id, WOKEN);
        assert_eq!(rebinds["agent-2"].to.name, "reviewer");
        assert!(
            !runtime
                .delivery_registrations_gone()
                .contains_key("agent-2")
        );
    }
    sleeping.pass();
    sleeping.assert_handed_over();
}

/// The orders the store's own passes make around a wake, as the doorbell
/// and watch producers made them on CI: a pass that read the ledger before
/// the wake publishes after it, and the next pass lands before the core
/// looks again. The record outlasts the first, which does not show the move,
/// and the second hands everything to the woken pane.
#[test]
fn a_pass_begun_before_the_wake_keeps_the_record_and_the_next_one_moves_the_registration() {
    let sleeping = SleepingChild::new();
    sleeping.pane_closed();
    {
        let mut runtime = sleeping.shared.lock().unwrap();
        let before = runtime.delivery_state().unwrap();
        sleeping.woken(&mut runtime, CHILD_NATIVE);
        runtime.publish_delivery(before, false);
        assert!(
            runtime
                .snapshot
                .ui_state
                .agent_sleep
                .dormant
                .contains_key(&sleeping.id)
        );
        let rebinds = runtime.delivery_registration_rebinds();
        assert_eq!(rebinds.len(), 1);
        assert_eq!(rebinds["agent-2"].to.pane_id, WOKEN);
    }
    sleeping.pass();
    sleeping.assert_handed_over();
}

/// A pane that is not the same conversation never inherits the registration:
/// it ends with the pane that held it, as any registration whose pane is gone.
#[test]
fn a_woken_pane_of_another_conversation_inherits_nothing_and_the_registration_ends() {
    let sleeping = SleepingChild::new();
    sleeping.pane_closed();
    sleeping.woken(&mut sleeping.shared.lock().unwrap(), "another-native");
    sleeping.pass();
    wait(
        &sleeping.shared,
        "the sleeping record to be released",
        |runtime| runtime.snapshot.ui_state.agent_sleep.dormant.is_empty(),
    );
    sleeping.pass();
    let runtime = sleeping.shared.lock().unwrap();
    assert!(runtime.delivery_registration_rebinds().is_empty());
    let ledger = runtime.delivery_state().unwrap();
    let record = ledger
        .agents
        .iter()
        .find(|record| record.id == "agent-2")
        .unwrap();
    assert!(record.ended);
    assert_eq!(record.pane, SLEEPER);
    assert!(ledger.watches.is_empty());
}
