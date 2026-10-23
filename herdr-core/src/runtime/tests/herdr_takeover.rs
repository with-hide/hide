//! Moving the operator's older Herdr server as the core decides it (PRD
//! herdr-takeover B1, B3, B4, B6-B9, B12, B14, B17, B19, D-23, and the
//! operator's decision of 2026-10-23: a live move starts on its own, once
//! per server, and the operator's own `herdr` is brought up after it).
//!
//! The takeover the core asks is a stand-in whose answers each test hands to
//! the runtime itself, so the order of looks and moves is the test's.

use std::sync::{Arc, Mutex, Weak, mpsc};

use super::first_run::{event, first_run_pass, report_here, step};
use super::*;
use crate::herdr_takeover::{
    FailureCause, HerdrTakeover, Look, Moved, Offer, OfferKind, PathHerdr, PathUpdated, Versions,
};
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

    fn versions(&self) -> Versions {
        Versions::default()
    }

    fn update_path_herdr(&self) -> PathUpdated {
        PathUpdated::Gone
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
            "kind": "move", "stage": "moving", "server_version": "0.8.2", "version": "0.9.5",
            "same_protocol": false, "failure": null, "path_herdr": null,
        })
    );

    let mut plain = runtime();
    report_here(&mut plain, &first_run_pass());
    assert_eq!(step(&plain), Some(FirstRunStep::Agents));
}

/// B8, B9, engineering rule 10: a look that does not answer in time counts
/// as one that could not read the server; while Hide cannot use the server
/// it is asked again three times without a word, then the step says so, Not
/// now closes the wizard, and a later look that finds nothing takes the
/// failure away. A server Hide reads keeps it in the log.
#[test]
fn a_look_that_could_not_read_an_unusable_server_shows_its_failure() {
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
    runtime.snapshot.status.herdr.state = "malformed".into();
    report_here(&mut runtime, &first_run_pass());
    for quiet in 1..=3 {
        runtime.ingest_herdr_look(look.clone());
        assert_eq!(
            takeover(&runtime),
            serde_json::Value::Null,
            "quiet retry {quiet}"
        );
        assert_eq!(step(&runtime), Some(FirstRunStep::Herdr));
        assert!(
            runtime.herdr_takeover_looking_for_test(),
            "retry {quiet} is on its way"
        );
    }
    runtime.ingest_herdr_look(look.clone());
    assert_eq!(step(&runtime), Some(FirstRunStep::Herdr));
    assert_eq!(
        takeover(&runtime),
        serde_json::json!({
            "kind": "look", "stage": "failed", "server_version": "", "version": "",
            "same_protocol": false, "failure": "unreadable", "path_herdr": null,
        })
    );
    assert!(act(&mut runtime, "later"));
    assert_eq!(step(&runtime), None);
    runtime.ingest_herdr_look(Look::Nothing);
    assert_eq!(takeover(&runtime), serde_json::Value::Null);

    let mut connected = installed();
    connected.snapshot.status.herdr.state = "connected".into();
    report_here(&mut connected, &first_run_pass());
    connected.ingest_herdr_look(look);
    assert_eq!(step(&connected), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&connected), serde_json::Value::Null);
}

/// Settings > About reads the Herdr this app ships and the operator's own
/// `herdr`, with the command that brings an older one up to it.
#[test]
fn the_versions_a_look_read_reach_the_snapshot() {
    let mut runtime = installed();
    assert!(runtime.ingest_herdr_versions(Versions {
        hide: Some("0.9.5".into()),
        path_herdr: Some(PathHerdr {
            version: "0.7.5".into(),
            protocol: Some(17),
            update: "brew upgrade herdr".into(),
        }),
    }));
    let status = serde_json::to_value(runtime.snapshot()).unwrap()["status"].clone();
    assert_eq!(
        status["herdr_versions"],
        serde_json::json!({
            "hide": "0.9.5",
            "path_herdr": {"version": "0.7.5", "behind": true, "update_command": "brew upgrade herdr"},
        })
    );
    assert!(!runtime.ingest_herdr_versions(Versions {
        hide: Some("0.9.5".into()),
        path_herdr: Some(PathHerdr {
            version: "0.7.5".into(),
            protocol: Some(17),
            update: "brew upgrade herdr".into(),
        }),
    }));
    assert!(runtime.ingest_herdr_versions(Versions::default()));
    let status = serde_json::to_value(runtime.snapshot()).unwrap()["status"].clone();
    assert_eq!(status["herdr_versions"], serde_json::Value::Null);
}

fn bounded_look_for_test(takeover: &Arc<dyn HerdrTakeover>, within: std::time::Duration) -> Look {
    super::super::herdr_takeover::bounded_look(takeover, within).0
}

/// A look that offers a live move starts it with no press, whatever the
/// protocol; a restart, which ends running commands, waits for the press.
#[test]
fn a_move_offer_starts_the_move_without_a_press() {
    for protocol in [OLD_PROTOCOL, PROTOCOL] {
        let mut runtime = installed();
        assert!(runtime.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", protocol)));
        assert_eq!(takeover(&runtime)["stage"], "moving", "protocol {protocol}");
        assert!(!act(&mut runtime, "move"), "a press is the same move");
    }
    let mut restart = installed();
    restart.ingest_herdr_look(offer(OfferKind::Restart, "0.6.2", OLD_PROTOCOL));
    assert_eq!(takeover(&restart)["stage"], "offered");
}

/// One automatic try per server: a failed automatic move offers the button
/// and is not tried again on its own for that server, whether its record
/// says so (D-23) or not, while another server is moved on its own.
#[test]
fn a_failed_automatic_move_offers_the_button_and_does_not_retry_the_same_server() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    runtime.ingest_herdr_move(Moved::Failed {
        cause: FailureCause::NotConfirmed,
        stage: "confirm".into(),
        detail: "another program runs it".into(),
    });
    assert_eq!(takeover(&runtime)["stage"], "failed");
    assert_eq!(takeover(&runtime)["failure"], "not_confirmed");

    // The record was not written: the same server is offered as if new.
    runtime.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    assert_eq!(takeover(&runtime)["stage"], "offered");
    assert!(act(&mut runtime, "move"), "the button moves it");
    assert_eq!(takeover(&runtime)["stage"], "moving");
    runtime.ingest_herdr_move(Moved::Failed {
        cause: FailureCause::Refused,
        stage: "handoff".into(),
        detail: "refused".into(),
    });

    // Another server on the socket is a server Hide has not tried.
    runtime.ingest_herdr_look(Look::Offer(Offer {
        server_pid: Some(77),
        ..match offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL) {
            Look::Offer(offer) => offer,
            _ => unreachable!(),
        }
    }));
    assert_eq!(takeover(&runtime)["stage"], "moving");
}

/// B7, B8, B9: a move under way takes no second press; a failure names its
/// cause and keeps the step; Not now appears only then and closes the
/// wizard with the move unanswered, which a later kit pass does not reopen;
/// the move made from Overview afterwards brings the agent question back.
#[test]
fn a_server_hide_cannot_read_is_moved_or_its_failure_lets_the_wizard_close() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    report_here(&mut runtime, &first_run_pass());

    assert!(!act(&mut runtime, "later"), "no Not now while it moves");
    assert_eq!(step(&runtime), Some(FirstRunStep::Herdr));
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

fn moved(path_herdr_behind: Option<&str>) -> Moved {
    Moved::Done {
        from_pid: 41,
        to_pid: 42,
        elapsed_ms: 900,
        path_herdr_behind: path_herdr_behind.map(str::to_owned),
    }
}

/// B4, B5: the wizard shows the moved result until Continue, with the
/// operator's own `herdr` as Hide's update of it left it; outside the
/// wizard a move shows only a `herdr` that could not be brought up.
#[test]
fn a_moved_server_shows_its_result_and_how_the_operators_herdr_was_brought_up() {
    let mut keeps_up = installed();
    keeps_up.ingest_herdr_look(offer(OfferKind::Move, "0.9.1", PROTOCOL));
    report_here(&mut keeps_up, &first_run_pass());
    keeps_up.ingest_herdr_move(moved(None));
    assert_eq!(takeover(&keeps_up)["stage"], "moved");
    assert_eq!(takeover(&keeps_up)["path_herdr"], serde_json::Value::Null);
    keeps_up.ingest_herdr_look(Look::Nothing);
    assert_eq!(
        step(&keeps_up),
        Some(FirstRunStep::Herdr),
        "the step stays to be read"
    );
    assert!(act(&mut keeps_up, "continue"));
    assert_eq!(step(&keeps_up), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&keeps_up), serde_json::Value::Null);

    let mut updated = installed();
    updated.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    report_here(&mut updated, &first_run_pass());
    updated.ingest_herdr_move(moved(Some("0.8.2")));
    assert_eq!(
        takeover(&updated)["path_herdr"],
        serde_json::json!({"stage": "updating", "version": "0.8.2"})
    );
    assert!(updated.ingest_path_herdr_update(PathUpdated::Current {
        version: "0.9.5".into()
    }));
    assert_eq!(
        takeover(&updated)["path_herdr"],
        serde_json::json!({"stage": "updated", "version": "0.9.5"})
    );
    assert_eq!(step(&updated), Some(FirstRunStep::Herdr));

    let mut behind = installed();
    behind.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    report_here(&mut behind, &first_run_pass());
    behind.ingest_herdr_move(moved(Some("0.8.2")));
    behind.ingest_path_herdr_update(PathUpdated::Behind {
        version: "0.10.0".into(),
        ran: Some("brew upgrade herdr".into()),
        detail: "Homebrew's latest speaks another protocol".into(),
    });
    assert_eq!(
        takeover(&behind)["path_herdr"],
        serde_json::json!({"stage": "behind", "version": "0.10.0"})
    );
    assert!(act(&mut behind, "continue"));
    assert_eq!(step(&behind), Some(FirstRunStep::Agents));
    assert_eq!(takeover(&behind), serde_json::Value::Null);
    assert!(
        !behind.ingest_path_herdr_update(PathUpdated::Gone),
        "an answer after Continue changes nothing"
    );

    // No wizard: nothing to show unless the `herdr` stays behind.
    let mut outside = installed();
    outside.ingest_herdr_look(offer(OfferKind::Move, "0.9.1", PROTOCOL));
    outside.ingest_herdr_move(moved(None));
    assert_eq!(takeover(&outside), serde_json::Value::Null);
    outside.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    outside.ingest_herdr_move(moved(Some("0.8.2")));
    assert_eq!(takeover(&outside)["path_herdr"]["stage"], "updating");
    outside.ingest_path_herdr_update(PathUpdated::Current {
        version: "0.9.5".into(),
    });
    assert_eq!(takeover(&outside), serde_json::Value::Null);
    outside.ingest_herdr_look(offer(OfferKind::Move, "0.8.3", OLD_PROTOCOL));
    outside.ingest_herdr_move(moved(Some("0.8.3")));
    outside.ingest_path_herdr_update(PathUpdated::Behind {
        version: "0.8.3".into(),
        ran: None,
        detail: "it is left to the operator".into(),
    });
    assert_eq!(
        takeover(&outside)["path_herdr"],
        serde_json::json!({"stage": "behind", "version": "0.8.3"})
    );
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

/// B12, D-29: a restart of a server Hide cannot read leaves no Not now until
/// the desktop host says its stop did not end the server; the failure stays
/// through looks at that same server, Not now then closes the wizard with
/// the restart unanswered, and another server on the socket forgets it.
#[test]
fn a_restart_that_could_not_stop_the_server_lets_the_wizard_close() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Restart, "0.8.2", OLD_PROTOCOL));
    report_here(&mut runtime, &first_run_pass());
    assert!(!act(&mut runtime, "later"), "no Not now before a failure");

    assert!(act(&mut runtime, "stop_failed"));
    assert_eq!(takeover(&runtime)["stage"], "failed");
    assert_eq!(takeover(&runtime)["failure"], "stop_failed");
    runtime.ingest_herdr_look(offer(OfferKind::Restart, "0.8.2", OLD_PROTOCOL));
    assert_eq!(
        takeover(&runtime)["failure"],
        "stop_failed",
        "the server that did not stop still answers"
    );

    assert!(act(&mut runtime, "later"));
    assert_eq!(step(&runtime), None);
    assert_eq!(
        takeover(&runtime)["kind"],
        "restart",
        "Overview still offers it"
    );

    runtime.ingest_herdr_look(Look::Unreadable {
        stage: "timeout".into(),
        detail: "a slow server".into(),
    });
    runtime.ingest_herdr_look(offer(OfferKind::Restart, "0.8.2", OLD_PROTOCOL));
    assert_eq!(
        takeover(&runtime)["failure"],
        "stop_failed",
        "a look that did not answer in time does not say the server is gone"
    );

    runtime.ingest_herdr_look(Look::Offer(Offer {
        server_pid: Some(77),
        ..match offer(OfferKind::Restart, "0.8.2", OLD_PROTOCOL) {
            Look::Offer(offer) => offer,
            _ => unreachable!(),
        }
    }));
    assert_eq!(takeover(&runtime)["stage"], "offered");
    assert_eq!(takeover(&runtime)["failure"], serde_json::Value::Null);
    assert_eq!(step(&runtime), None, "the closed wizard stays closed");

    // A restart from Overview leaves no server to offer for: the agent
    // question the wizard left is asked now.
    runtime.ingest_herdr_look(Look::Nothing);
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
}

/// D-29: only a restart can fail to stop; a move's failure is the core's own.
#[test]
fn a_stop_failure_is_refused_where_no_restart_is_offered() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    assert!(!act(&mut runtime, "stop_failed"));
    assert_eq!(takeover(&runtime)["stage"], "moving");
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
    assert_eq!(takeover(&runtime)["stage"], "moving");
}

/// B3: a look that started before the move read the server the move
/// replaces; its answer neither unlocks the button nor takes the step away,
/// and a failure after it still shows.
#[test]
fn a_look_that_started_before_the_move_does_not_undo_it() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Move, "0.8.2", OLD_PROTOCOL));
    report_here(&mut runtime, &first_run_pass());
    assert_eq!(takeover(&runtime)["stage"], "moving");
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
    runtime.ingest_herdr_move(moved(Some("0.8.2")));
    assert!(!act(&mut runtime, "later"));
    assert_eq!(takeover(&runtime)["stage"], "moved");
}

/// B12: after the desktop host restarts the server, the shell asks for a
/// look, so the offer goes even when the connection state never changed.
#[test]
fn a_look_the_shell_asks_for_runs_without_a_connection_change() {
    let mut runtime = installed();
    runtime.ingest_herdr_look(offer(OfferKind::Restart, "0.9.1", PROTOCOL));
    assert!(!runtime.herdr_takeover_looking_for_test());
    act(&mut runtime, "look");
    assert!(runtime.herdr_takeover_looking_for_test());
    runtime.ingest_herdr_look(Look::Nothing);
    assert_eq!(takeover(&runtime), serde_json::Value::Null);
}
