//! What the screen offers for moving this machine's Herdr save into the
//! core's own engine, as the core decides it (PRD
//! engine-bundle-4-herdr-removal B2 to B5, B11, B15 to B17).
//!
//! The machine the core asks is a stand-in whose handle to the runtime leads
//! nowhere, so each test hands the looks and moves to the runtime itself.

use std::sync::{Arc, Weak};

use super::first_run::event;
use super::*;
use crate::herdr_import::{
    Look, Machine, Offer, Outcome, RefusedWorkspace, Seen, Server, Take, Taken,
};

struct StandIn;

impl Machine for StandIn {
    fn look(&self) -> Seen {
        Seen::default()
    }

    fn take(&self, _: Take) -> Taken {
        Taken::Failed("the test answers for it".into())
    }

    fn dismiss(&self) {}
}

fn installed() -> Runtime {
    let mut runtime = runtime();
    runtime.install_herdr_import(
        Arc::new(StandIn),
        Weak::new(),
        crate::handle::ChangeNotifier::noop(),
    );
    runtime
}

fn offered(server: Server, outcome: Option<Outcome>) -> Seen {
    Seen {
        look: Look::Offer(Offer {
            workspaces: 2,
            tabs: 3,
            agents: 1,
        }),
        server,
        outcome,
    }
}

fn shown(runtime: &Runtime) -> serde_json::Value {
    serde_json::to_value(runtime.snapshot()).unwrap()["status"]["herdr_import"].clone()
}

fn act(runtime: &mut Runtime, action: &str) -> bool {
    runtime.dispatch_json(&event(
        "herdr_import",
        serde_json::json!({"action": action}),
    ))
}

/// B2, B3: while Hide's own Herdr runs, the move is a question with what it
/// moves, and once asked for it shows moving until this core stops.
#[test]
fn a_running_hide_herdr_is_asked_about_and_its_move_shows_until_the_core_starts_again() {
    let mut runtime = installed();
    runtime.ingest_herdr_save(offered(Server::Hide, None));
    assert_eq!(
        shown(&runtime),
        serde_json::json!({
            "kind": "move", "stage": "offered", "workspaces": 2, "tabs": 3, "agents": 1,
            "herdr_running": true, "failure": null, "reason": null, "refused": [],
        })
    );
    assert!(act(&mut runtime, "move"));
    assert_eq!(shown(&runtime)["stage"], "moving");
    assert!(!runtime.ingest_herdr_take(Take::Move, Taken::Restarting));
    assert_eq!(shown(&runtime)["stage"], "moving");
    // A second press while it moves is the same one.
    assert!(!act(&mut runtime, "move"));
}

/// B4: a Herdr that could not be stopped moves nothing, says so, and the
/// move can be asked for again.
#[test]
fn a_herdr_that_could_not_be_stopped_says_so_and_offers_the_move_again() {
    let mut runtime = installed();
    runtime.ingest_herdr_save(offered(Server::Hide, None));
    assert!(act(&mut runtime, "move"));
    runtime.ingest_herdr_take(Take::Move, Taken::NotStopped);
    assert_eq!(shown(&runtime)["stage"], "offered");
    runtime.ingest_herdr_save(offered(Server::Hide, Some(Outcome::NotStopped)));
    let failed = shown(&runtime);
    assert_eq!(
        (&failed["stage"], &failed["failure"]),
        (
            &serde_json::json!("failed"),
            &serde_json::json!("not_stopped")
        )
    );
    assert!(act(&mut runtime, "move"));
    assert_eq!(shown(&runtime)["stage"], "moving");
}

/// B15, B16: a save no Hide Herdr runs is offered to import, read only, and
/// a move that would stop a Herdr is refused for it.
#[test]
fn a_save_no_hide_herdr_runs_is_offered_to_import_and_never_to_move() {
    let mut runtime = installed();
    runtime.ingest_herdr_save(offered(Server::Other, None));
    let offer = shown(&runtime);
    assert_eq!(
        (&offer["kind"], &offer["herdr_running"]),
        (&serde_json::json!("import"), &serde_json::json!(true))
    );
    assert!(!act(&mut runtime, "move"));
    assert_eq!(shown(&runtime)["stage"], "offered");
    assert!(act(&mut runtime, "import"));
    assert_eq!(shown(&runtime)["stage"], "moving");

    let mut stopped = installed();
    stopped.ingest_herdr_save(offered(Server::Stopped, None));
    assert_eq!(shown(&stopped)["herdr_running"], false);
}

/// B11: a save that could not be read says why with nothing to move, and no
/// save shows nothing at all.
#[test]
fn an_unreadable_save_shows_why_and_no_save_shows_nothing() {
    let mut runtime = installed();
    runtime.ingest_herdr_save(Seen {
        look: Look::Unreadable("session.json is not a Herdr session".into()),
        server: Server::Stopped,
        outcome: None,
    });
    assert_eq!(
        shown(&runtime),
        serde_json::json!({
            "kind": "import", "stage": "failed", "workspaces": 0, "tabs": 0, "agents": 0,
            "herdr_running": false, "failure": "unreadable",
            "reason": "session.json is not a Herdr session", "refused": [],
        })
    );
    assert!(!act(&mut runtime, "import"));
    runtime.ingest_herdr_save(Seen::default());
    assert_eq!(shown(&runtime), serde_json::Value::Null);
}

/// B11: the workspaces a move left out wait with their names and why until
/// the operator reads them.
#[test]
fn a_move_s_left_out_workspaces_wait_until_they_are_read() {
    let mut runtime = installed();
    runtime.ingest_herdr_save(Seen {
        look: Look::Nothing,
        server: Server::Stopped,
        outcome: Some(Outcome::Moved {
            workspaces: 1,
            tabs: 1,
            agents: 0,
            waiting: 0,
            refused: vec![RefusedWorkspace {
                workspace: "w2".into(),
                label: "api".into(),
                reason: "its layout names a pane twice".into(),
            }],
        }),
    });
    let result = shown(&runtime);
    assert_eq!(
        (&result["kind"], &result["stage"], &result["refused"]),
        (
            &serde_json::json!("result"),
            &serde_json::json!("moved"),
            &serde_json::json!([{"label": "api", "reason": "its layout names a pane twice"}])
        )
    );
    assert!(act(&mut runtime, "continue"));
    assert_eq!(shown(&runtime), serde_json::Value::Null);
}

/// A core that does not run its own engine has no machine and refuses.
#[test]
fn a_core_without_its_own_engine_offers_nothing() {
    let mut runtime = runtime();
    assert!(!act(&mut runtime, "move"));
    assert_eq!(shown(&runtime), serde_json::Value::Null);
}
