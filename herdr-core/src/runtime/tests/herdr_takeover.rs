//! Moving the operator's older Herdr server as the core decides it (PRD
//! herdr-takeover B1, B3, B4, B6-B9, B12, B14, B17, B19, D-23).
//!
//! The takeover the core asks is a stand-in whose answers each test hands to
//! the runtime itself, so the order of looks and moves is the test's.

use std::sync::{Arc, Mutex, Weak, mpsc};

use super::first_run::{event, first_run_pass, report_here, step};
use super::*;
use crate::herdr_takeover::{FailureCause, HerdrTakeover, Look, Moved, Offer, OfferKind};
use crate::model::FirstRunStep;

/// Answers what it is told; the runtime under test never hears it on its
/// own, since its handle to the runtime leads nowhere.
struct StandIn {
    look: Look,
    /// A look that answers only once the test lets go of the other end.
    held: Option<Mutex<mpsc::Receiver<()>>>,
}

impl HerdrTakeover for StandIn {
    fn look(&self) -> Look {
        if let Some(held) = &self.held {
            let _ = held.lock().unwrap().recv();
        }
        self.look.clone()
    }

    fn hand_over(&self) -> Moved {
        Moved::Failed {
            cause: FailureCause::Unreadable,
            stage: "stand_in".into(),
            detail: "the test answers for it".into(),
        }
    }
}

fn installed() -> Runtime {
    let mut runtime = runtime();
    runtime.install_herdr_takeover(
        Arc::new(StandIn {
            look: Look::Nothing,
            held: None,
        }),
        Weak::new(),
        crate::handle::ChangeNotifier::noop(),
    );
    runtime
}

fn offer(kind: OfferKind, version: &str, protocol: u64) -> Look {
    Look::Offer(Offer {
        kind,
        server_pid: Some(41),
        server_version: version.to_owned(),
        server_protocol: protocol,
        version: "0.9.5".to_owned(),
        failed_before: None,
    })
}

const OLD_PROTOCOL: u64 = 20;
const PROTOCOL: u64 = hide_herdr_client::HERDR_PROTOCOL_REVISION;

fn takeover(runtime: &Runtime) -> serde_json::Value {
    serde_json::to_value(runtime.snapshot()).unwrap()["status"]["herdr_takeover"].clone()
}

fn act(runtime: &mut Runtime, action: &str) -> bool {
    runtime.dispatch_json(&event(
        "herdr_takeover",
        serde_json::json!({"action": action}),
    ))
}

/// B1, D-09, D-17: the wizard opens on the Herdr step while the look has not
/// answered, keeps it for a server to move, and goes on to the agents once
/// the look finds nothing; a daemon with no takeover opens on the agents.
#[test]
fn the_wizard_opens_on_herdr_only_while_there_may_be_a_server_to_move() {
    let mut waiting = installed();
    report_here(&mut waiting, &first_run_pass());
    assert_eq!(step(&waiting), Some(FirstRunStep::Herdr));
    waiting.ingest_herdr_look(Look::Nothing);
    assert_eq!(step(&waiting), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&waiting), serde_json::Value::Null);

    let mut offered = installed();
    offered.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    report_here(&mut offered, &first_run_pass());
    assert_eq!(step(&offered), Some(FirstRunStep::Herdr));
    assert_eq!(
        takeover(&offered),
        serde_json::json!({
            "kind": "move", "stage": "offered", "server_version": "0.8.2", "version": "0.9.5",
            "same_protocol": false, "failure": null, "path_herdr_behind": null,
        })
    );

    let mut plain = runtime();
    report_here(&mut plain, &first_run_pass());
    assert_eq!(step(&plain), Some(FirstRunStep::Agents));
}

/// Engineering rule 10: a look that does not answer in time counts as one
/// that found nothing, so the wizard is never held on its Herdr step.
#[test]
fn a_look_that_does_not_answer_in_time_finds_nothing() {
    let (release, held) = mpsc::channel();
    let slow: Arc<dyn HerdrTakeover> = Arc::new(StandIn {
        look: offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL),
        held: Some(Mutex::new(held)),
    });
    let look = bounded_look_for_test(&slow, std::time::Duration::from_millis(50));
    drop(release);
    assert!(
        matches!(&look, Look::Unreadable { stage, .. } if stage == "timeout"),
        "{look:?}"
    );
    let mut runtime = installed();
    report_here(&mut runtime, &first_run_pass());
    runtime.ingest_herdr_look(look);
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&runtime), serde_json::Value::Null);
}

fn bounded_look_for_test(takeover: &Arc<dyn HerdrTakeover>, within: std::time::Duration) -> Look {
    super::super::herdr_takeover::bounded_look(takeover, within)
}

/// B3, B7, B8, B9: a server Hide cannot read offers the move alone; a second
/// press while it moves starts nothing; a failure names its cause and keeps
/// the step; Not now appears only then and closes the wizard with the move
/// unanswered, which a later kit pass does not reopen; the move made from
/// Overview afterwards brings the agent question back.
#[test]
fn a_server_hide_cannot_read_is_moved_or_its_failure_lets_the_wizard_close() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    report_here(&mut runtime, &first_run_pass());

    assert!(!act(&mut runtime, "later"), "no Not now before a failure");
    assert_eq!(step(&runtime), Some(FirstRunStep::Herdr));
    assert!(act(&mut runtime, "move"));
    assert_eq!(takeover(&runtime)["stage"], "moving");
    assert!(
        !act(&mut runtime, "move"),
        "a second press is the same move"
    );

    runtime.ingest_herdr_move(Moved::Failed {
        cause: FailureCause::Refused,
        stage: "handoff".into(),
        detail: "refused".into(),
    });
    // The look after the move answers with the remembered failure (D-23).
    runtime.ingest_herdr_look(Look::Offer(Offer {
        failed_before: Some(FailureCause::Refused),
        ..match offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL) {
            Look::Offer(offer) => offer,
            _ => unreachable!(),
        }
    }));
    assert_eq!(takeover(&runtime)["stage"], "failed");
    assert_eq!(takeover(&runtime)["failure"], "refused");
    assert_eq!(step(&runtime), Some(FirstRunStep::Herdr));

    assert!(act(&mut runtime, "later"));
    assert_eq!(step(&runtime), None);
    report_here(&mut runtime, &first_run_pass());
    assert_eq!(step(&runtime), None, "the closed wizard stays closed");

    // Overview's button tries again, whatever failed before.
    assert!(act(&mut runtime, "move"));
    runtime.ingest_herdr_move(Moved::Done {
        from_pid: 41,
        to_pid: 42,
        elapsed_ms: 900,
        path_herdr_behind: None,
    });
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&runtime), serde_json::Value::Null);
}

/// B4, B5: a move whose operator's own `herdr` keeps up goes straight on to
/// the agents; one whose `herdr` is behind stays to say so until Continue.
#[test]
fn a_moved_server_stays_on_screen_only_while_the_operators_herdr_is_behind() {
    let mut keeps_up = installed();
    keeps_up.ingest_herdr_look(offer(OfferKind::Move, "0.9.1", PROTOCOL));
    report_here(&mut keeps_up, &first_run_pass());
    act(&mut keeps_up, "move");
    keeps_up.ingest_herdr_move(Moved::Done {
        from_pid: 41,
        to_pid: 42,
        elapsed_ms: 900,
        path_herdr_behind: None,
    });
    assert_eq!(step(&keeps_up), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&keeps_up), serde_json::Value::Null);

    let mut behind = installed();
    behind.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    report_here(&mut behind, &first_run_pass());
    act(&mut behind, "move");
    behind.ingest_herdr_move(Moved::Done {
        from_pid: 41,
        to_pid: 42,
        elapsed_ms: 900,
        path_herdr_behind: Some("0.8.2".into()),
    });
    assert_eq!(takeover(&behind)["stage"], "moved");
    assert_eq!(takeover(&behind)["path_herdr_behind"], "0.8.2");
    // The look at the moved server finds nothing; the step stays to be read.
    behind.ingest_herdr_look(Look::Nothing);
    assert_eq!(step(&behind), Some(FirstRunStep::Herdr));
    assert!(act(&mut behind, "continue"));
    assert_eq!(step(&behind), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&behind), serde_json::Value::Null);
}

/// B6, B12: a server Hide reads as it is can be left for later from the
/// start, which goes on to the agents without moving it; a restart is the
/// desktop host's, so the core never moves a server that cannot hand over.
#[test]
fn a_server_on_the_same_protocol_can_be_left_and_a_restart_is_not_moved() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Restart, "0.9.1", PROTOCOL));
    report_here(&mut runtime, &first_run_pass());
    assert_eq!(step(&runtime), Some(FirstRunStep::Herdr));
    assert_eq!(takeover(&runtime)["same_protocol"], true);
    assert!(!act(&mut runtime, "move"));
    assert!(act(&mut runtime, "later"));
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
    assert_eq!(
        takeover(&runtime)["kind"],
        "restart",
        "Overview still knows it"
    );
}

/// B18: a newer protocol is shown but never asks the wizard to wait.
#[test]
fn a_newer_protocol_offers_an_update_and_opens_no_herdr_step() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::UpdateHide, "0.10.0", PROTOCOL + 1));
    report_here(&mut runtime, &first_run_pass());
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&runtime)["kind"], "update_hide");
    assert!(!act(&mut runtime, "move"));
}

/// B19, D-24: a browser tab's wizard is the agent step alone; its Continue
/// leaves the Herdr step unanswered and goes on.
#[test]
fn a_browser_tabs_continue_leaves_the_herdr_step() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    report_here(&mut runtime, &first_run_pass());
    assert!(runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["codex"], "finish": true}),
    )));
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&runtime)["stage"], "offered");
}

/// B3: a look that started before the press read the server the move
/// replaces; its answer neither unlocks the button nor takes the step away,
/// and a failure after it still shows.
#[test]
fn a_look_that_started_before_the_move_does_not_undo_it() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    report_here(&mut runtime, &first_run_pass());
    assert!(act(&mut runtime, "move"));
    let before_the_press = 0;
    runtime.ingest_herdr_look_from(
        before_the_press,
        offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL),
    );
    assert_eq!(takeover(&runtime)["stage"], "moving");
    runtime.ingest_herdr_look_from(before_the_press, Look::Nothing);
    assert_eq!(takeover(&runtime)["stage"], "moving");
    assert_eq!(step(&runtime), Some(FirstRunStep::Herdr));
    runtime.ingest_herdr_move(Moved::Failed {
        cause: FailureCause::Refused,
        stage: "handoff".into(),
        detail: "refused".into(),
    });
    assert_eq!(takeover(&runtime)["stage"], "failed");
    assert_eq!(takeover(&runtime)["failure"], "refused");
}

/// D-17: a Herdr step a daemon saved before its look answered leaves for
/// the agents once a later daemon's look finds nothing, so the wizard never
/// sits on a step with nothing to draw.
#[test]
fn a_saved_herdr_step_goes_on_once_a_look_finds_nothing() {
    let mut runtime = runtime();
    report_here(&mut runtime, &first_run_pass());
    runtime.snapshot.ui_state.first_run = Some(crate::model::FirstRun {
        version: 1,
        step: FirstRunStep::Herdr,
    });
    runtime.install_herdr_takeover(
        Arc::new(StandIn {
            look: Look::Nothing,
            held: None,
        }),
        Weak::new(),
        crate::handle::ChangeNotifier::noop(),
    );
    runtime.ingest_herdr_look(Look::Nothing);
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
}

/// A moved step is left only by Continue, whatever the protocol.
#[test]
fn a_moved_step_cannot_be_left_for_later() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Move, "0.9.1", PROTOCOL));
    report_here(&mut runtime, &first_run_pass());
    act(&mut runtime, "move");
    runtime.ingest_herdr_move(Moved::Done {
        from_pid: 41,
        to_pid: 42,
        elapsed_ms: 900,
        path_herdr_behind: Some("0.8.2".into()),
    });
    assert!(!act(&mut runtime, "later"));
    assert_eq!(takeover(&runtime)["stage"], "moved");
}
