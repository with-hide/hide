//! A linked worktree removed outside Hide (`runtime/gone_worktrees.rs`):
//! the Herdr workspace left open in its folder is closed by the core when
//! its node confirms the folder gone and nothing in it is the operator's.
use super::*;
use hide_node_link::catalog::{PathFact, PathFacts, RepositoryPlace};
use hide_node_link::cleanup::PathState;
use hide_node_link::protocol::Call;

/// An absolute path on every system, since the close checks pane folders
/// are absolute before it reads them.
fn at(name: &str) -> String {
    if cfg!(windows) {
        format!("C:/fixture/{name}")
    } else {
        format!("/fixture/{name}")
    }
}

fn repo() -> String {
    at("repo")
}

fn gone() -> String {
    at("repo.worktrees/topic")
}

/// Herdr's session: workspace w1, which Herdr opened on the linked worktree
/// `gone()` of `repo()`, with one tab labelled `label` holding pane w1:p1 in
/// that folder, an agent in it when `agent` names its status. Without
/// `pane` the workspace has closed with its last pane.
fn session(label: &str, agent: Option<&str>, pane: bool) -> serde_json::Value {
    let rect = serde_json::json!({"x": 0, "y": 0, "width": 80, "height": 24});
    let agents = match agent {
        Some(status) => serde_json::json!([{"id": "a1", "pane_id": "w1:p1", "agent": "claude",
            "agent_status": status, "state_change_seq": 1, "lineage_session": "s1"}]),
        None => serde_json::json!([]),
    };
    if !pane {
        return serde_json::json!({
            "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": []
        });
    }
    serde_json::json!({
        "workspaces": [{
            "workspace_id": "w1", "label": "topic", "number": 1, "focused": true,
            "pane_count": 1, "tab_count": 1, "active_tab_id": "w1:t1", "agent_status": "idle",
            "worktree": {"repo_key": repo(), "repo_name": "repo", "repo_root": repo(),
                "checkout_path": gone(), "is_linked_worktree": true}
        }],
        "tabs": [{"workspace_id": "w1", "tab_id": "w1:t1", "number": 1, "label": label,
            "focused": true, "pane_count": 1, "agent_status": "idle"}],
        "panes": [{"workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p1",
            "terminal_id": "fixture-terminal", "focused": true, "revision": 0,
            "agent_status": "idle", "cwd": gone()}],
        "layouts": [{"workspace_id": "w1", "tab_id": "w1:t1", "zoomed": false, "area": rect,
            "focused_pane_id": "w1:p1",
            "panes": [{"pane_id": "w1:p1", "focused": true, "rect": rect}], "splits": []}],
        "agents": agents
    })
}

/// What the core's own node says of the session's paths: the repository is
/// there, and `topic` is gone when `topic_gone`, else never asked about.
fn paths(topic_gone: bool) -> workspace::PathIndex {
    let mut facts = PathFacts::default();
    facts.paths.insert(
        repo(),
        PathFact {
            comparison: repo(),
            exists: true,
            project: None,
            project_unread: None,
            repository: Some(RepositoryPlace {
                root: repo(),
                main_root: repo(),
                branch: Some("main".into()),
                head_oid: None,
            }),
        },
    );
    if topic_gone {
        facts.paths.insert(
            gone(),
            PathFact {
                comparison: gone(),
                exists: false,
                project: None,
                project_unread: None,
                repository: None,
            },
        );
    }
    workspace::PathIndex::new(facts)
}

/// Applies `session` as the sync coordinator would, with the catalog built
/// from `paths` outside the lock.
fn ingest(runtime: &mut Runtime, session: serde_json::Value, paths: workspace::PathIndex) {
    let payload = crate::sidebar::owned_label_fixture(session).expect("gone worktree session");
    let spaces = Runtime::session_spaces(&payload, &paths);
    let catalog = session_sync::PrecomputedCatalog {
        registrations: Vec::new(),
        worktrees: no_worktrees(),
        workspaces: workspace::build_catalog(
            &crate::node::test_node(),
            &[],
            &spaces,
            &no_worktrees(),
            &paths,
        ),
        paths: Arc::new(paths),
    };
    runtime.ingest_session_with_catalog(Ok(payload), Some(catalog));
}

/// The checkouts a close starts for after `session`, read off a runtime with
/// no live Herdr, which decides nothing on its own.
fn closes_after(
    runtime: &mut Runtime,
    session: serde_json::Value,
    paths: workspace::PathIndex,
) -> Vec<(String, Vec<String>)> {
    assert!(runtime.live.is_none());
    ingest(runtime, session, paths);
    runtime
        .gone_worktrees_to_close()
        .into_iter()
        .map(|gone| (gone.path, gone.panes))
        .collect()
}

fn the_gone_row(runtime: &Runtime) -> CheckoutSnapshot {
    runtime
        .snapshot()
        .navigator
        .workspaces
        .iter()
        .flat_map(|workspace| &workspace.checkouts)
        .find(|checkout| checkout.path == gone())
        .cloned()
        .expect("the gone worktree is a row")
}

#[test]
fn a_confirmed_gone_worktree_holding_only_a_shell_is_closed_once() {
    let mut runtime = runtime();
    assert_eq!(
        closes_after(&mut runtime, session("", None, true), paths(true)),
        [(gone(), vec!["w1:p1".to_owned()])]
    );
    assert!(!the_gone_row(&runtime).exists);
    assert_eq!(
        closes_after(&mut runtime, session("", None, true), paths(true)),
        [],
        "a second sighting of the same row starts no second close"
    );
}

#[test]
fn a_worktree_its_node_did_not_say_is_gone_is_not_closed() {
    let mut runtime = runtime();
    assert_eq!(
        closes_after(&mut runtime, session("", None, true), paths(false)),
        []
    );
}

#[test]
fn a_gone_worktree_with_an_agent_at_work_is_kept_until_the_agent_finishes() {
    let mut runtime = runtime();
    assert_eq!(
        closes_after(
            &mut runtime,
            session("", Some("working"), true),
            paths(true)
        ),
        []
    );
    assert_eq!(
        closes_after(&mut runtime, session("", Some("done"), true), paths(true)),
        [(gone(), vec!["w1:p1".to_owned()])],
        "a finished agent is closed like a shell"
    );
}

#[test]
fn a_gone_worktree_holding_a_tab_the_operator_named_is_kept() {
    let mut runtime = runtime();
    runtime
        .snapshot
        .ui_state
        .operator_tab_names
        .insert("w1:t1".into(), "notes".into());
    assert_eq!(
        closes_after(&mut runtime, session("notes", None, true), paths(true)),
        []
    );
}

/// The whole path against a fake engine: the close is started off the lock,
/// asks the node again, reads that the pane holds only its shell, and asks
/// the engine to close the pane, after which the workspace is gone.
#[test]
fn the_core_closes_the_pane_herdr_left_in_a_gone_worktree() {
    let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let herdr = FakeEngine::start({
        let closed = Arc::clone(&closed);
        move |method, params| match method {
            "session" => reply(fake::session(session(
                "",
                None,
                !closed.load(Ordering::SeqCst),
            ))),
            "pane_processes" => reply(fake::processes(&serde_json::json!({
                "shell_pid": 42, "foreground_process_group_id": 42,
                "foreground_processes": [{"pid": 42, "name": "zsh"}]}))),
            "close_pane" => {
                assert_eq!(params["pane"], "w1:p1");
                closed.store(true, Ordering::SeqCst);
                reply(())
            }
            other => panic!("unexpected {other}"),
        }
    });
    let shared = SharedRuntime::new(runtime());
    shared.lock().unwrap().live = Some(live::LiveContext {
        socket_path: herdr.scope(),
        runtime: shared.weak(),
        notifier: crate::handle::ChangeNotifier::noop(),
        engine: herdr.engine(),
        node: Arc::new(GoneNode),
    });
    let (_, records) = crate::diagnostics::capture(|| {
        ingest(
            &mut shared.lock().unwrap(),
            session("", None, true),
            paths(true),
        );
    });
    let requested = records
        .iter()
        .find(|record| record["kind"] == "gone_worktree.close_requested")
        .expect("the close is recorded");
    assert_eq!(requested["checkout"], gone());
    assert_eq!(requested["herdr_workspace_ids"], serde_json::json!(["w1"]));
    wait_for("the engine to be asked to close the pane", || {
        herdr.methods().contains(&"close_pane")
    });
}

/// A node on which `gone()` is missing and every other path is itself, with
/// nothing listening.
struct GoneNode;

impl crate::node_access::NodeLink for GoneNode {
    fn call(
        &self,
        call: Call,
        _timeout: Duration,
    ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
        let answer = match call {
            Call::RealPaths { paths } => serde_json::to_value(
                paths
                    .into_iter()
                    .map(|path| {
                        if path == gone() {
                            PathState::Missing
                        } else {
                            PathState::Real { path }
                        }
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap(),
            Call::ListeningPorts => serde_json::json!({"entries": [], "unavailable_reason": null}),
            other => panic!("unexpected node call {other:?}"),
        };
        Ok(crate::node_access::LinkAnswer::Parsed(answer))
    }
}
