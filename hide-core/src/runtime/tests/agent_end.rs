//! `hide agent end` (with-hide/hide-private#676): ending a registration
//! closes the pane its agent ran in, unless `--keep-pane`, and keeps the
//! conversation under 보관됨 for 이어가기.
//!
//! The caller is registered on this machine and its worker runs on the
//! connected device `mini`, whose engine is a fake that closes the panes it
//! is asked to; this machine has no engine at all, so a close that went
//! anywhere but the worker's machine would fail instead of passing unseen.

use super::device_catalog::{engine_workspace, session};
use super::*;
use crate::coordination::{AgentRecord, Command};
use crate::delivery::{Actor, ledger, worker::Worker};
use crate::handle::ChangeNotifier;
use crate::runtime::delivery::tests::{authority, fixture};
use crate::session_archive::{ArchivePhase, ArchiveRecord, Resume};
use serde_json::{Value, json};

const DEVICE: &str = "mini";
const PATH: &str = "/srv/repo";
/// The worker's pane as the device's engine names it, and as this core does.
const DEVICE_PANE: &str = "t1";
const PANE: &str = "remote:mini:pane:t1";
const TERMINAL: &str = "terminal-t1";
const SESSION: &str = "native-worker";

/// The device's panes, each with the terminal it runs, and whether the
/// engine refuses the next close.
#[derive(Default)]
struct Panes {
    open: Vec<(String, String)>,
    close_refused: bool,
}

fn device_engine(panes: Arc<Mutex<Panes>>) -> Fake {
    FakeEngine::start(move |method, params| {
        let mut panes = panes.lock().unwrap();
        match method {
            "pane_terminal" => match panes.open.iter().find(|(pane, _)| *pane == params["pane"]) {
                Some((_, terminal)) => reply(terminal.clone()),
                None => refused("pane_not_found", "the pane does not exist"),
            },
            "close_pane" if panes.close_refused => refused("internal_error", "the close timed out"),
            "close_pane" => {
                panes.open.retain(|(pane, _)| *pane != params["pane"]);
                reply(())
            }
            // The delivery store's own passes over the device.
            "session" => reply(fake::session(json!({}))),
            "report_pane_tokens" => reply(()),
            other => panic!("unexpected {other}"),
        }
    })
}

struct Ending {
    _turn: std::sync::MutexGuard<'static, ()>,
    _root: tempfile::TempDir,
    runtime: Arc<Mutex<Runtime>>,
    actor: Actor,
    panes: Arc<Mutex<Panes>>,
    fake: Fake,
    path: PathBuf,
    _worker: Worker,
    client: crate::delivery::worker::Client,
}

/// A caller registered as `agent-1` on this machine and its worker
/// `agent-2`, a Codex agent in pane `t1` of the device, running `SESSION`.
fn ending() -> Ending {
    let turn = crate::coordination::SPAWN_TURN
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let root = tempfile::tempdir().unwrap();
    let (runtime, mut actor, _, path) = fixture(root.path());
    // The ledger keeps the digest of the native session, which is what the
    // caller's pane must show for it to be that participant.
    actor.session = crate::engine::tokens::session_digest("native-parent");
    let caller: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
        {"id":"sender","pane_id":"sender","agent":"codex","agent_status":"working",
            "state_change_seq":1,"lineage_session":actor.session},
    ]}))
    .unwrap();
    runtime
        .lock()
        .unwrap()
        .observe_delivery_laid(crate::node::TEST_NODE, &caller, None, None);
    let panes = Arc::new(Mutex::new(Panes {
        open: vec![(DEVICE_PANE.to_owned(), TERMINAL.to_owned())],
        close_refused: false,
    }));
    let fake = device_engine(panes.clone());
    {
        let mut guard = runtime.lock().unwrap();
        guard
            .snapshot
            .ui_state
            .device_registrations
            .push(crate::model::DeviceRegistration {
                id: DEVICE.to_owned(),
                label: DEVICE.to_owned(),
                origin: crate::model::LinkOrigin::Dialed {
                    ssh_alias: DEVICE.to_owned(),
                },
                host_consent: None,
            });
        guard.snapshot.status.remote.push(RemoteStatusSnapshot {
            target_id: DEVICE.to_owned(),
            state: "connected".to_owned(),
            message: None,
            session: None,
            files: RemoteFileListSnapshot::idle(),
            catalog: Default::default(),
            unreachable_since_unix_ms: None,
        });
        guard
            .device_machine_ids
            .insert(DEVICE.to_owned(), "machine-mini".to_owned());
        guard.install_remote_control(RemoteControlContext::new(
            DEVICE,
            fake.engine(),
            Arc::downgrade(&runtime),
            ChangeNotifier::noop(),
        ));
        show(&mut guard, Some(SESSION));
    }
    let worker = Actor {
        pane_id: PANE.into(),
        name: "worker".into(),
        kind: "codex".into(),
        device_id: DEVICE.into(),
        session: crate::engine::tokens::session_digest(SESSION),
    };
    let mut state = ledger::Ledger::default();
    for (record, machine, session, pane, terminal, parent) in [
        (
            &actor,
            crate::node::TEST_NODE,
            "native-parent",
            "sender",
            None,
            None,
        ),
        (
            &worker,
            DEVICE,
            SESSION,
            DEVICE_PANE,
            Some(TERMINAL),
            Some("agent-1"),
        ),
    ] {
        crate::coordination::apply(
            &mut state,
            &actor,
            &crate::coordination::Mutation::Register {
                record: AgentRecord {
                    id: String::new(),
                    name: Some(record.name.clone()),
                    machine: machine.into(),
                    host_scope: machine.into(),
                    native_machine: format!("machine-{machine}"),
                    session: session.into(),
                    instance: pane.into(),
                    pane: pane.into(),
                    terminal_id: terminal.map(str::to_owned),
                    parent: parent.map(str::to_owned),
                    origin: None,
                    project: None,
                    actor: record.clone(),
                    ended: false,
                    forked: false,
                },
                check: false,
            },
            1,
        )
        .unwrap();
    }
    ledger::save(&path, &state).unwrap();
    runtime
        .lock()
        .unwrap()
        .publish_delivery(Arc::new(ledger::load(&path).unwrap()), false);
    let (worker, client) = Worker::spawn(
        Arc::downgrade(&runtime),
        ChangeNotifier::noop(),
        path.clone(),
    )
    .unwrap();
    Ending {
        _turn: turn,
        _root: root,
        runtime,
        actor,
        panes,
        fake,
        path,
        _worker: worker,
        client,
    }
}

/// The device's session as the core holds it: pane `t1` in the project's
/// checkout with an agent running `conversation` in it, or, with none, no
/// pane at all.
fn show(runtime: &mut Runtime, conversation: Option<&str>) {
    let tabs: &[(&str, &str)] = match conversation {
        Some(_) => &[(DEVICE_PANE, PATH)],
        None => &[],
    };
    let mut reported = session(vec![engine_workspace(DEVICE, "w1", PATH, tabs)]);
    if let Some(conversation) = conversation {
        reported.agents = project_agents(
            crate::sidebar::owned_label_fixture(json!({"agents": [{
                "id": PANE, "pane_id": PANE, "agent": "codex", "agent_status": "idle",
                "state_change_seq": 1, "cwd": PATH,
                "agent_session": {"kind": "id", "value": conversation},
            }]}))
            .unwrap(),
        )
        .agents;
    }
    runtime.ingest_remote_session(DEVICE, Ok(reported));
}

impl Ending {
    fn end(&self, keep_pane: bool) -> Result<Value, String> {
        crate::coordination::run(
            self.client.clone(),
            authority(&self.actor),
            self.actor.clone(),
            Command::End {
                id: "agent-2".into(),
                actor: None,
                keep_pane,
            },
        )
    }

    fn worker_ended(&self) -> bool {
        ledger::load(&self.path).unwrap().agents[1].ended
    }

    fn open(&self) -> Vec<String> {
        let panes = self.panes.lock().unwrap();
        panes.open.iter().map(|(pane, _)| pane.clone()).collect()
    }

    fn closes(&self) -> usize {
        self.fake
            .methods()
            .iter()
            .filter(|method| **method == "close_pane")
            .count()
    }

    fn archive(&self) -> Vec<ArchiveRecord> {
        let runtime = self.runtime.lock().unwrap();
        let archive = &runtime.snapshot.ui_state.session_archive;
        archive.records.values().cloned().collect()
    }
}

/// #676: the worker's pane closes, which ends its holder, its shell and the
/// agent under them, and its conversation waits under 보관됨, where
/// 이어가기 resumes it; ending it again closes nothing more.
#[test]
fn ending_an_agent_closes_its_pane_and_keeps_its_conversation_for_resume() {
    let ending = ending();
    let answer = ending.end(false).unwrap();
    assert_eq!(answer["id"], "agent-2");
    assert_eq!(answer["pane"], "closed");
    assert_eq!(answer["conversation"], "archived");
    assert!(ending.worker_ended());
    assert!(ending.open().is_empty(), "the worker's pane closed");

    let [record] = ending.archive().try_into().expect("one archived session");
    assert_eq!(record.phase, ArchivePhase::Archived);
    let member = record.root();
    assert_eq!(
        (
            member.machine.as_str(),
            member.kind.as_str(),
            member.native_session_id.as_str(),
            member.checkout_path.as_str(),
        ),
        (DEVICE, "codex", SESSION, PATH)
    );
    assert_eq!(member.registration, None, "the registration ended");
    {
        let mut runtime = ending.runtime.lock().unwrap();
        let member = |runtime: &Runtime| {
            let archive = &runtime.snapshot.ui_state.session_archive;
            let record = archive.records.values().next().expect("still archived");
            record.root().clone()
        };
        // Until the device's next session read the closed pane is still
        // listed with the conversation; that is no resume of it.
        assert!(!member(&runtime).closed);
        runtime.advance_session_archive();
        assert_eq!(member(&runtime).resume, Resume::Idle);
        show(&mut runtime, None);
        runtime.advance_session_archive();
        assert!(member(&runtime).closed, "its pane left the listing");
        let rows = &runtime.snapshot.navigator.archived_sessions;
        assert!(rows.len() == 1 && rows[0].listed, "under 보관됨: {rows:?}");
        // A pane that shows the conversation from then on resumed it, even
        // one the engine gave the same id, and the record leaves 보관됨.
        let (_, records) = crate::diagnostics::capture(|| show(&mut runtime, Some(SESSION)));
        assert!(
            records
                .iter()
                .any(|record| record["kind"] == "session_archive.resumed"
                    && record["pane_id"] == PANE),
            "{records:?}"
        );
        runtime.advance_session_archive();
        assert!(runtime.snapshot.ui_state.session_archive.records.is_empty());
        show(&mut runtime, None);
    }

    let again = ending.end(false).unwrap();
    assert_eq!(again["pane"], "gone");
    assert_eq!(again["conversation"], "none");
    assert_eq!(ending.closes(), 1);
    assert!(ending.archive().is_empty());
}

/// `--keep-pane` ends only the registration, as every end did before.
#[test]
fn keeping_the_pane_ends_only_the_registration() {
    let ending = ending();
    let answer = ending.end(true).unwrap();
    assert_eq!(answer["pane"], "kept");
    assert_eq!(answer["conversation"], "none");
    assert!(ending.worker_ended());
    assert_eq!(ending.open(), [DEVICE_PANE]);
    assert_eq!(ending.closes(), 0);
    assert!(ending.archive().is_empty());
}

/// A pane that shows another conversation now is no longer the
/// registration's to close.
#[test]
fn a_pane_running_another_conversation_is_left_open() {
    let ending = ending();
    show(&mut ending.runtime.lock().unwrap(), Some("another"));
    let answer = ending.end(false).unwrap();
    assert_eq!(answer["pane"], "other_agent");
    assert!(ending.worker_ended());
    assert_eq!(ending.open(), [DEVICE_PANE]);
    assert_eq!(ending.closes(), 0);
    assert!(ending.archive().is_empty());
}

/// A pane id that now names another terminal is not the agent's pane.
#[test]
fn a_pane_id_now_running_another_terminal_is_left_open() {
    let ending = ending();
    ending.panes.lock().unwrap().open[0].1 = "terminal-later".into();
    let answer = ending.end(false).unwrap();
    assert_eq!(answer["pane"], "gone");
    assert_eq!(ending.closes(), 0);
    assert_eq!(ending.open(), [DEVICE_PANE]);
}

/// A close the engine does not take leaves the registration ended, as an end
/// always did, and says so; the same command closes the pane once the engine
/// takes it.
#[test]
fn a_refused_close_is_answered_and_the_same_command_closes_the_pane() {
    let ending = ending();
    ending.panes.lock().unwrap().close_refused = true;
    let (refusal, records) = crate::diagnostics::capture(|| ending.end(false));
    assert_eq!(refusal, Err("pane_close_failed".to_owned()));
    assert!(
        records
            .iter()
            .any(|record| record["kind"] == "agent.end_pane_failed"
                && record["agent_id"] == "agent-2"),
        "{records:?}"
    );
    assert!(ending.worker_ended());
    assert_eq!(ending.open(), [DEVICE_PANE]);
    assert!(ending.archive().is_empty(), "nothing closed, nothing kept");

    ending.panes.lock().unwrap().close_refused = false;
    let answer = ending.end(false).unwrap();
    assert_eq!(answer["pane"], "closed");
    assert_eq!(answer["conversation"], "archived");
    assert!(ending.open().is_empty());
}

/// A full archive keeps no conversation and still closes the pane, which is
/// what the end is for; the answer and the diagnostic log both say the
/// conversation was not kept.
#[test]
fn a_full_archive_closes_the_pane_and_says_the_conversation_was_not_kept() {
    let ending = ending();
    let members = (0..crate::session_archive::MAX_ARCHIVED_SESSIONS)
        .map(|n| {
            crate::session_archive::fixture_member(&format!("old-{n}"), (n > 0).then_some("old-0"))
        })
        .collect();
    ending
        .runtime
        .lock()
        .unwrap()
        .snapshot
        .ui_state
        .session_archive
        .admit(
            crate::session_archive::ArchiveId::new().unwrap(),
            crate::session_archive::fixture_record(members, 1),
        )
        .unwrap();
    let (answer, records) = crate::diagnostics::capture(|| ending.end(false));
    let answer = answer.unwrap();
    assert_eq!(answer["pane"], "closed");
    assert_eq!(answer["conversation"], "archive_full");
    assert!(ending.open().is_empty());
    assert_eq!(ending.archive().len(), 1, "nothing older was evicted");
    assert!(
        records
            .iter()
            .any(|record| record["kind"] == "agent.end_not_archived"
                && record["agent_id"] == "agent-2"
                && record["pane_id"] == DEVICE_PANE),
        "{records:?}"
    );
}

/// An agent of this machine ended right after its first turn, before the
/// label reader confirmed its session file, still has its conversation kept:
/// the session it registered with is the one its row shows.
#[test]
fn an_agent_ended_before_its_session_file_was_confirmed_keeps_its_conversation() {
    use super::tree_close::{EVERY, tree_catalog, tree_payload, tree_runtime};
    let mut runtime = tree_runtime(&[]);
    let mut payload = tree_payload(&EVERY, &[]);
    for agent in &mut payload.agents {
        agent.agent_session = Some(crate::sidebar::SessionAgentSessionPayload {
            kind: "id".into(),
            value: format!("s-{}", agent.pane_id.as_deref().unwrap().replace(':', "-")),
        });
    }
    runtime.ingest_session_with_catalog(Ok(payload), Some(tree_catalog()));
    let row = |pane: &str| {
        runtime
            .snapshot
            .navigator
            .agents
            .iter()
            .find(|row| row.pane_id == pane)
            .unwrap()
            .clone()
    };
    assert!(row("w1:p2").row_facts.is_none(), "no file confirmed yet");
    let registered = crate::engine::tokens::session_digest("s-w1-p2");
    let crate::runtime::session_archive::EndPlan::Close { keep: Some(ended) } =
        runtime.end_pane_plan("w1:p2", registered.as_deref())
    else {
        panic!("the conversation is kept");
    };
    assert_eq!(ended.member.native_session_id, "s-w1-p2");
    assert_eq!(ended.member.source_reference, None);
}
