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

    fn restart(&self) {}
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
            waiting: 0,
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
            "waiting": 0, "herdr_running": true, "failure": null, "reason": null, "refused": [],
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
            "waiting": 0, "herdr_running": false, "failure": "unreadable",
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

/// B15: the conversations an import left open in a Herdr are offered to
/// resume once no Herdr runs, and not while another one runs on.
#[test]
fn waiting_conversations_are_offered_only_once_their_herdr_no_longer_runs() {
    let mut runtime = installed();
    let waiting = |server| Seen {
        look: Look::Offer(Offer {
            workspaces: 0,
            tabs: 0,
            agents: 0,
            waiting: 2,
        }),
        server,
        outcome: None,
    };
    runtime.ingest_herdr_save(waiting(Server::Other));
    assert_eq!(shown(&runtime), serde_json::Value::Null);
    assert!(!act(&mut runtime, "import"));
    runtime.ingest_herdr_save(waiting(Server::Stopped));
    let offer = shown(&runtime);
    assert_eq!(
        (&offer["kind"], &offer["waiting"]),
        (&serde_json::json!("import"), &serde_json::json!(2))
    );
    assert!(act(&mut runtime, "import"));
}

/// B9: an agent whose turn was cut off is told once it is the agent seen in
/// its pane, and one that never shows is given up on rather than kept.
#[test]
fn a_cut_agent_is_told_once_it_shows_and_given_up_on_when_it_never_does() {
    let mut runtime = runtime();
    runtime.owe_transition_letter("h1:p1", "s-1", 1_000);
    runtime.owe_transition_letter("h1:p2", "s-2", 1_000);
    runtime.drive_transition_letters(2_000);
    assert_eq!(runtime.transition_letters_owed(), ["h1:p1", "h1:p2"]);
    let actor = crate::delivery::Actor {
        pane_id: "h1:p1".into(),
        name: "worker".into(),
        kind: "claude".into(),
        device_id: runtime.node().as_str().into(),
        session: None,
    };
    runtime.delivery_observations.insert(
        "h1:p1".into(),
        super::super::agent_sleep::letters::asleep_observation(actor),
    );
    runtime.drive_transition_letters(3_000);
    assert_eq!(runtime.transition_letters_owed(), ["h1:p2"]);
    runtime.drive_transition_letters(1_000 + 10 * 60 * 1000 + 1);
    assert!(runtime.transition_letters_owed().is_empty());
}

/// B18: a device's save is offered under its own name, beside this
/// machine's, moved by its own mover, and goes with its link.
#[test]
fn a_device_s_save_is_offered_under_its_name_and_goes_with_its_link() {
    let mut runtime = installed();
    assert!(runtime.install_device_herdr_import("d1", Arc::new(StandIn), offered(Server::Hide, None)));
    let status = &serde_json::to_value(runtime.snapshot()).unwrap()["status"];
    assert_eq!(status["herdr_import"], serde_json::Value::Null);
    assert_eq!(
        status["herdr_import_devices"],
        serde_json::json!([{
            "kind": "move", "stage": "offered", "workspaces": 2, "tabs": 3, "agents": 1,
            "waiting": 0, "herdr_running": true, "failure": null, "reason": null, "refused": [],
            "device": "d1",
        }])
    );
    // This machine offers nothing, so its move is not the device's.
    assert!(!act(&mut runtime, "move"));
    let device_move = event("herdr_import", serde_json::json!({"action": "move", "device": "d1"}));
    assert!(runtime.dispatch_json(&device_move));
    let devices = |runtime: &Runtime| {
        serde_json::to_value(runtime.snapshot()).unwrap()["status"]["herdr_import_devices"].clone()
    };
    assert_eq!(devices(&runtime)[0]["stage"], "moving");
    runtime.ingest_device_herdr_take("d1", Take::Move, Taken::NotStopped);
    assert_eq!(devices(&runtime)[0]["stage"], "offered");
    assert!(runtime.retire_device_herdr_import("d1"));
    assert_eq!(devices(&runtime), serde_json::json!([]));
    assert!(!runtime.dispatch_json(&device_move));
}
