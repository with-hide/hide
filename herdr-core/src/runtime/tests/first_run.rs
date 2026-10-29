//! The first-run wizard's progress as the core keeps it (PRD
//! first-run-onboarding D-07, D-08, D-21, B3, B6, B10, B12, B26, B43, B45).

use super::appearance::{runtime_at, ui_state_update};
use super::*;
use crate::model::{FirstRun, FirstRunStep, ScreenIntro};
use hide_kit::{AgentReport, Availability, ComponentState, KitReport, PieceReport};

pub(super) fn agent(id: &str, enabled: bool, skill: ComponentState) -> AgentReport {
    AgentReport {
        id: id.to_owned(),
        label: id.to_owned(),
        availability: Availability::Available,
        enabled,
        chosen: false,
        skill: PieceReport {
            state: skill,
            reason: (skill == ComponentState::Failed).then(|| "permission denied".to_owned()),
            why: None,
            location: None,
        },
        hook: None,
        herdr: None,
        update: None,
        doc_url: "https://example.test/skills".to_owned(),
    }
}

/// This Mac's kit pass: `held` while its record waits for the first-run choice.
pub(super) fn pass(held: bool, agents: Vec<AgentReport>) -> KitReport {
    KitReport {
        agents,
        held_for_onboarding: held,
        ..KitReport::default()
    }
}

pub(super) fn first_run_pass() -> KitReport {
    pass(
        true,
        vec![
            agent("claude-code", false, ComponentState::Off),
            agent("codex", false, ComponentState::Off),
        ],
    )
}

pub(super) fn event(kind: &str, payload: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION, "kind": kind, "payload": payload,
    }))
    .expect("event")
}

pub(super) fn step(runtime: &Runtime) -> Option<FirstRunStep> {
    runtime.snapshot().ui_state.first_run.map(|run| run.step)
}

pub(super) fn report_here(runtime: &mut Runtime, report: &KitReport) {
    // Existing cases exercise the original no-saved-devices route.
    runtime.snapshot.status.herdr_profiles.state = "ready".to_owned();
    let node = runtime.node.clone();
    runtime.ingest_kit_report(node.as_str(), report);
}

/// The kit worker takes what Continue queued and answers with `report`.
pub(super) fn run_queued_pass(runtime: &mut Runtime, report: &KitReport) -> hide_kit::Scope {
    let Some(KitJob::Apply(scope)) = runtime.take_local_kit_job(Instant::now()) else {
        panic!("Continue queues an install on this Mac");
    };
    report_here(runtime, report);
    scope
}

fn agent_ids(set: &std::collections::BTreeSet<String>) -> Vec<&str> {
    set.iter().map(String::as_str).collect()
}

/// B3, D-08: a Mac whose kit record waits opens the wizard on its first step;
/// an install that already had the kit never gets one.
#[test]
fn a_first_run_opens_the_wizard_and_an_existing_install_never_does() {
    let mut first = runtime();
    assert_eq!(step(&first), None);
    report_here(&mut first, &first_run_pass());
    assert_eq!(
        first.snapshot().ui_state.first_run,
        Some(FirstRun {
            version: crate::model::FIRST_RUN_VERSION,
            step: FirstRunStep::Agents
        })
    );
    assert_eq!(
        serde_json::to_value(first.snapshot()).unwrap()["ui_state"]["first_run"],
        serde_json::json!({"version": 1, "step": "agents"})
    );

    let mut existing = runtime();
    report_here(
        &mut existing,
        &pass(
            false,
            vec![agent("claude-code", true, ComponentState::Installed)],
        ),
    );
    assert_eq!(step(&existing), None);
    assert!(!existing.snapshot().ui_state.desktop_notifications);
}

/// B6, B10: Continue moves the wizard only once the kit pass it queued has
/// landed, a second press meanwhile and a report already on its way move
/// nothing, and a restart reopens the step the wizard stood on.
#[test]
fn the_first_step_moves_on_when_its_pass_lands_and_a_restart_reopens_there() {
    let folder = scratch_dir("herdr-core-first-run-");
    let path = folder.path().join("state.json");
    let mut runtime = runtime_at(&path);
    report_here(&mut runtime, &first_run_pass());

    assert!(runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["claude-code"]}),
    )));
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
    assert!(
        !runtime.dispatch_json(&event(
            "agent_onboarding_apply",
            serde_json::json!({"agents": ["codex"]}),
        )),
        "a second press while the pass runs is the same intent"
    );
    // A pass that started before Continue answers while the install waits.
    report_here(&mut runtime, &first_run_pass());
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));

    let scope = run_queued_pass(
        &mut runtime,
        &pass(
            false,
            vec![
                agent("claude-code", true, ComponentState::Installed),
                agent("codex", false, ComponentState::Off),
            ],
        ),
    );
    assert_eq!(agent_ids(&scope.agent_on), ["claude-code"]);
    assert_eq!(step(&runtime), Some(FirstRunStep::Notifications));
    drop(runtime);

    let restarted = runtime_at(&path);
    assert_eq!(step(&restarted), Some(FirstRunStep::Notifications));
}

/// B12: a chosen agent whose piece failed keeps the first step on screen, and
/// Continue again sends that agent again and moves on once it is in place.
#[test]
fn a_failed_agent_keeps_the_first_step_and_continue_again_retries_it() {
    let mut runtime = runtime();
    report_here(&mut runtime, &first_run_pass());
    runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["claude-code"]}),
    ));
    run_queued_pass(
        &mut runtime,
        &pass(
            false,
            vec![
                agent("claude-code", true, ComponentState::Failed),
                agent("codex", false, ComponentState::Off),
            ],
        ),
    );
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));

    assert!(runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["claude-code"]}),
    )));
    let scope = run_queued_pass(
        &mut runtime,
        &pass(
            false,
            vec![
                agent("claude-code", true, ComponentState::Installed),
                agent("codex", false, ComponentState::Off),
            ],
        ),
    );
    assert_eq!(agent_ids(&scope.agent_on), ["claude-code"]);
    assert!(scope.agent_off.is_empty(), "codex was already off");
    assert_eq!(step(&runtime), Some(FirstRunStep::Notifications));
}

/// A chosen agent whose copy is too old is not a failure Try again fixes:
/// its hook is refused until the operator updates it, and the wizard goes on
/// with it still chosen (docs/agent-hooks.md, Updating an agent that is too
/// old).
#[test]
fn a_chosen_agent_too_old_does_not_hold_the_first_step() {
    let mut runtime = runtime();
    report_here(&mut runtime, &first_run_pass());
    runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["claude-code"]}),
    ));
    let mut old = agent("claude-code", true, ComponentState::Installed);
    old.hook = Some(PieceReport {
        state: ComponentState::Failed,
        reason: Some("Claude Code 2.1.200 is older than 2.1.278".to_owned()),
        why: Some(hide_kit::PartWhy {
            cause: hide_kit::PartCause::UpdateAgent,
            value: Some("2.1.278".to_owned()),
        }),
        location: None,
    });
    run_queued_pass(
        &mut runtime,
        &pass(false, vec![old, agent("codex", false, ComponentState::Off)]),
    );
    assert_eq!(step(&runtime), Some(FirstRunStep::Notifications));
}

/// A record that did not save asks the question again, and the wizard stays
/// on the step that asks it (B6).
#[test]
fn an_unsaved_answer_keeps_the_first_step() {
    let mut runtime = runtime();
    report_here(&mut runtime, &first_run_pass());
    runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": []}),
    ));
    run_queued_pass(&mut runtime, &first_run_pass());
    assert_eq!(
        runtime.snapshot().ui_state.agent_onboarding,
        Some(crate::model::AgentOnboarding::Pending)
    );
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
}

/// A Continue whose pass could not run (the kit link failed, or the pass
/// listed no agent) keeps the first step, leaves the question unanswered,
/// asks for the agents again, and a second Continue queues the install anew.
#[test]
fn a_pass_that_did_not_run_keeps_the_first_step_and_continue_again_sends_it() {
    for not_run in ["link failed", "no agents"] {
        let mut runtime = runtime();
        report_here(&mut runtime, &first_run_pass());
        runtime.dispatch_json(&event(
            "agent_onboarding_apply",
            serde_json::json!({"agents": ["claude-code"]}),
        ));
        let Some(KitJob::Apply(_)) = runtime.take_local_kit_job(Instant::now()) else {
            panic!("Continue queues an install on this Mac");
        };
        if not_run == "link failed" {
            runtime
                .set_local_kit_unavailable(crate::model::KitUnavailable::Unreadable, "timed out");
        } else {
            report_here(&mut runtime, &pass(true, Vec::new()));
        }
        assert_eq!(step(&runtime), Some(FirstRunStep::Agents), "{not_run}");
        assert_eq!(
            runtime.snapshot().ui_state.agent_onboarding,
            Some(crate::model::AgentOnboarding::Pending),
            "{not_run}: nothing was written, so the question is open again"
        );
        assert!(
            matches!(
                runtime.take_local_kit_job(Instant::now()),
                Some(KitJob::Status)
            ),
            "{not_run}: the agents are read again"
        );

        report_here(&mut runtime, &first_run_pass());
        assert!(runtime.dispatch_json(&event(
            "agent_onboarding_apply",
            serde_json::json!({"agents": ["claude-code"]}),
        )));
        let scope = run_queued_pass(
            &mut runtime,
            &pass(
                false,
                vec![agent("claude-code", true, ComponentState::Installed)],
            ),
        );
        assert_eq!(agent_ids(&scope.agent_on), ["claude-code"], "{not_run}");
        assert_eq!(
            step(&runtime),
            Some(FirstRunStep::Notifications),
            "{not_run}"
        );
    }
}

/// B12, B44: while this Mac's kit cannot run, Try again on a step that drew
/// no tile records no choice and queues no pass, on the desktop, in a browser
/// tab and in the walk Help reopens alike; it only has the agents read again,
/// and the step's own line says why, so no error reaches a banner.
#[test]
fn try_again_while_the_kit_cannot_run_records_nothing_and_reads_the_agents_again() {
    let mut reopened = runtime();
    reopened.snapshot.ui_state.agent_onboarding = Some(crate::model::AgentOnboarding::Done);
    reopened.snapshot.ui_state.first_run = Some(FirstRun {
        version: crate::model::FIRST_RUN_VERSION,
        step: FirstRunStep::Done,
    });
    reopened.set_local_kit_unavailable(crate::model::KitUnavailable::Unreadable, "timed out");
    assert!(reopened.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": []}),
    )));
    assert!(reopened.snapshot().status.last_error.is_none());
    assert!(matches!(
        reopened.take_local_kit_job(Instant::now()),
        Some(KitJob::Status)
    ));

    for finish in [false, true] {
        let mut runtime = runtime();
        report_here(&mut runtime, &first_run_pass());
        runtime.set_local_kit_unavailable(crate::model::KitUnavailable::Unreadable, "timed out");
        assert!(runtime.dispatch_json(&event(
            "agent_onboarding_apply",
            serde_json::json!({"agents": [], "finish": finish}),
        )));
        assert_eq!(
            step(&runtime),
            Some(FirstRunStep::Agents),
            "finish {finish}"
        );
        assert_eq!(
            runtime.snapshot().ui_state.agent_onboarding,
            Some(crate::model::AgentOnboarding::Pending),
            "finish {finish}: the question stays open"
        );
        assert!(
            runtime
                .snapshot()
                .ui_state
                .agent_onboarding_agents
                .is_empty(),
            "finish {finish}"
        );
        assert!(
            matches!(
                runtime.take_local_kit_job(Instant::now()),
                Some(KitJob::Status)
            ),
            "finish {finish}: only the agents are read again"
        );
        assert!(
            runtime.snapshot().status.last_error.is_none(),
            "finish {finish}"
        );
    }
}

/// The command the app's own kit names to stop its background service.
const STOP: &str = "HIDE_STATE_DIR='/srv/shared/hide' '/opt/hide/resources/hide' stop";

/// A pass that stopped before changing anything, here a state folder
/// `HIDE_STATE_DIR` chose that another account can change, refused by any
/// system's rule.
fn refused_pass() -> KitReport {
    KitReport::refused(hide_kit::KitRefusal {
        cause: hide_kit::RefusalCause::Unsafe,
        path: Some("/srv/shared".to_owned()),
        variable: Some(hide_kit::layout::StateVariable::HideStateDir),
        stop: Some(STOP.to_owned()),
        detail: "the Hide state override cannot be inspected at /srv/shared".to_owned(),
    })
}

fn refused_by(cause: hide_kit::RefusalCause) -> KitReport {
    KitReport::refused(hide_kit::KitRefusal {
        cause,
        path: None,
        variable: None,
        stop: None,
        detail: "the kit's own words".to_owned(),
    })
}

fn local_kit(runtime: &Runtime) -> serde_json::Value {
    let snapshot = serde_json::to_value(runtime.snapshot()).unwrap();
    snapshot["navigator"]["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|device| device["kind"] != "remote")
        .expect("this Mac's row")["kit"]
        .clone()
}

/// A new Mac whose first pass stopped before changing anything still gets
/// the wizard, with why on its first step: the refusal reaches the screen
/// once, with what to change and no part listed four times. Try again runs
/// the same pass again, never a read that could look like success, and
/// records no choice; once a pass runs, the question is asked as usual.
#[test]
fn a_refused_first_pass_opens_the_wizard_with_why_and_try_again_runs_the_pass() {
    let mut runtime = runtime();
    report_here(&mut runtime, &refused_pass());
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
    assert_eq!(
        runtime.snapshot().ui_state.agent_onboarding,
        None,
        "a refused pass says nothing about the record"
    );
    let kit = local_kit(&runtime);
    assert_eq!(
        kit["refused"],
        serde_json::json!({"cause": "unsafe", "path": "/srv/shared", "variable": "HIDE_STATE_DIR", "stop": STOP}),
        "the kit's own words stay in the log"
    );
    assert_eq!(kit["components"], serde_json::json!([]));
    assert_eq!(kit["offers_reinstall"], false);

    for finish in [false, true] {
        assert!(runtime.dispatch_json(&event(
            "agent_onboarding_apply",
            serde_json::json!({"agents": [], "finish": finish}),
        )));
        assert_eq!(local_kit(&runtime)["busy"], true, "finish {finish}");
        let scope = run_queued_pass(&mut runtime, &refused_pass());
        assert!(scope.is_automatic(), "finish {finish}: the same pass again");
        assert_eq!(
            step(&runtime),
            Some(FirstRunStep::Agents),
            "finish {finish}"
        );
        assert_eq!(
            runtime.snapshot().ui_state.agent_onboarding,
            None,
            "finish {finish}: no choice recorded"
        );
        assert!(
            local_kit(&runtime)["refused"].is_object(),
            "finish {finish}"
        );
        assert!(
            runtime.snapshot().status.last_error.is_none(),
            "finish {finish}"
        );
    }

    // The operator fixed the folder: the pass runs and the question is asked.
    assert!(runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": []}),
    )));
    run_queued_pass(&mut runtime, &first_run_pass());
    assert!(local_kit(&runtime)["refused"].is_null());
    assert_eq!(
        runtime.snapshot().ui_state.agent_onboarding,
        Some(crate::model::AgentOnboarding::Pending)
    );
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
}

/// Whatever stopped the first pass, a new Mac's wizard opens with it.
#[test]
fn a_first_pass_refused_for_any_cause_opens_the_wizard() {
    use hide_kit::RefusalCause::{Busy, Projects, Record};
    for cause in [Busy, Record, Projects] {
        let mut runtime = runtime();
        report_here(&mut runtime, &refused_by(cause));
        assert_eq!(step(&runtime), Some(FirstRunStep::Agents), "{cause:?}");
    }
}

/// Clearing a refusal can take a terminal and a restart, so the operator
/// may close the wizard; the same refusal read again keeps it closed, a
/// different one opens it, and the question stays unanswered for the pass
/// that runs. The agent step never closes this way while a pass can run.
#[test]
fn not_now_closes_a_refused_wizard_until_the_refusal_changes() {
    let mut runtime = runtime();
    let close = || event("first_run_advance", serde_json::json!({"from": "agents"}));
    report_here(&mut runtime, &refused_pass());
    assert!(runtime.dispatch_json(&close()));
    assert_eq!(step(&runtime), None);
    assert_eq!(runtime.snapshot().ui_state.agent_onboarding, None);

    report_here(&mut runtime, &refused_pass());
    assert_eq!(step(&runtime), None, "Settings' re-read reopens nothing");

    report_here(&mut runtime, &refused_by(hide_kit::RefusalCause::Busy));
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));

    report_here(&mut runtime, &first_run_pass());
    assert!(!runtime.dispatch_json(&close()));
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
}

/// The kit's own words reach the log once per refusal, not on every read
/// while Settings is open.
#[test]
fn a_refusal_is_logged_when_it_appears_or_changes() {
    let mut runtime = runtime();
    let ((), records) = crate::diagnostics::capture(|| {
        for _ in 0..3 {
            report_here(&mut runtime, &refused_pass());
        }
        report_here(&mut runtime, &refused_by(hide_kit::RefusalCause::Busy));
        report_here(&mut runtime, &refused_by(hide_kit::RefusalCause::Busy));
    });
    let causes: Vec<_> = records
        .iter()
        .filter(|record| record["kind"] == "kit.refused")
        .map(|record| record["cause"].clone())
        .collect();
    assert_eq!(
        causes,
        [serde_json::json!("unsafe"), serde_json::json!("busy")]
    );
}

/// An install that was set up before gets no wizard when a later pass is
/// refused; its machine row says why, and that row's Try again runs the
/// same pass again.
#[test]
fn a_refused_pass_on_an_existing_install_opens_no_wizard_and_its_row_retries_the_pass() {
    let mut runtime = runtime();
    report_here(
        &mut runtime,
        &pass(
            false,
            vec![agent("claude-code", true, ComponentState::Installed)],
        ),
    );
    report_here(&mut runtime, &refused_pass());
    assert_eq!(step(&runtime), None);
    assert!(local_kit(&runtime)["refused"].is_object());

    let node = runtime.node.clone();
    assert!(runtime.dispatch_json(&event(
        "kit_reinstall",
        serde_json::json!({"device_id": node.as_str()}),
    )));
    let scope = run_queued_pass(&mut runtime, &refused_pass());
    assert!(scope.is_automatic());
    assert!(runtime.snapshot().status.last_error.is_none());
}

/// B26: a browser tab's wizard is its first step alone, so applying it ends it.
#[test]
fn a_browser_tab_finishes_the_wizard_with_its_first_step() {
    let mut runtime = runtime();
    report_here(&mut runtime, &first_run_pass());
    runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["codex"], "finish": true}),
    ));
    run_queued_pass(
        &mut runtime,
        &pass(false, vec![agent("codex", true, ComponentState::Installed)]),
    );
    assert_eq!(step(&runtime), Some(FirstRunStep::Done));
}

/// B16, B21, B30: Continue on the notification step of a Mac that already
/// allows Hide turns the switch on with the same press; Not now leaves it off,
/// even after Turn on was pressed while macOS asks; a press that names no
/// switch leaves it as it is; and a stale press neither moves the step nor
/// turns the switch on.
#[test]
fn leaving_the_notification_step_sets_the_switch_its_press_names() {
    let at = |step| {
        let mut runtime = runtime();
        runtime.snapshot.ui_state.first_run = Some(FirstRun {
            version: crate::model::FIRST_RUN_VERSION,
            step,
        });
        runtime
    };
    let advance = |from: &str, on: bool| {
        event(
            "first_run_advance",
            serde_json::json!({"from": from, "notifications": on}),
        )
    };
    let mut allowed = at(FirstRunStep::Notifications);
    assert!(allowed.dispatch_json(&advance("notifications", true)));
    assert_eq!(step(&allowed), Some(FirstRunStep::Project));
    assert!(allowed.snapshot().ui_state.desktop_notifications);
    assert!(!allowed.snapshot().ui_state.desktop_notifications_done);

    let mut not_now = at(FirstRunStep::Notifications);
    not_now.snapshot.ui_state.desktop_notifications_done = true;
    assert!(not_now.dispatch_json(&event(
        "desktop_notifications_set",
        serde_json::json!({"enabled": true}),
    )));
    assert!(not_now.dispatch_json(&advance("notifications", false)));
    assert_eq!(step(&not_now), Some(FirstRunStep::Project));
    assert!(!not_now.snapshot().ui_state.desktop_notifications);
    assert!(!not_now.snapshot().ui_state.desktop_notifications_done);

    let mut kept = at(FirstRunStep::Notifications);
    kept.snapshot.ui_state.desktop_notifications = true;
    kept.snapshot.ui_state.desktop_notifications_done = true;
    assert!(kept.dispatch_json(&event(
        "first_run_advance",
        serde_json::json!({"from": "notifications"}),
    )));
    assert!(kept.snapshot().ui_state.desktop_notifications);
    assert!(kept.snapshot().ui_state.desktop_notifications_done);

    let mut stale = at(FirstRunStep::Project);
    assert!(!stale.dispatch_json(&advance("notifications", true)));
    assert!(!stale.snapshot().ui_state.desktop_notifications);
}

/// B21, B25, B6: the later steps move one at a time from the step on screen;
/// a window still showing an earlier step moves nothing, and the first step
/// ends only through its apply.
#[test]
fn the_wizard_leaves_only_the_step_on_screen() {
    let mut runtime = runtime();
    runtime.snapshot.ui_state.first_run = Some(FirstRun {
        version: crate::model::FIRST_RUN_VERSION,
        step: FirstRunStep::Notifications,
    });
    let advance = |from: &str| event("first_run_advance", serde_json::json!({"from": from}));
    assert!(!runtime.dispatch_json(&advance("project")));
    assert_eq!(step(&runtime), Some(FirstRunStep::Notifications));
    assert!(runtime.dispatch_json(&advance("notifications")));
    assert_eq!(step(&runtime), Some(FirstRunStep::Project));
    assert!(!runtime.dispatch_json(&advance("notifications")));
    assert!(!runtime.dispatch_json(&advance("agents")));
    assert!(runtime.dispatch_json(&advance("project")));
    assert_eq!(step(&runtime), Some(FirstRunStep::Done));
    assert!(!runtime.dispatch_json(&advance("done")));
}

/// B45: the first step reopened from Help on an install whose question was
/// answered switches only the agents whose switch it changed, the way
/// Settings > Agents does, and opens no wizard progress.
#[test]
fn a_reopened_first_step_switches_only_what_it_changed() {
    let mut runtime = runtime();
    let mut omp = agent("omp", false, ComponentState::Off);
    omp.availability = Availability::Available;
    let mut antigravity = agent("antigravity", true, ComponentState::Installed);
    antigravity.availability = Availability::NotInstalled;
    report_here(
        &mut runtime,
        &pass(
            false,
            vec![
                agent("claude-code", true, ComponentState::Installed),
                agent("codex", true, ComponentState::Installed),
                omp,
                antigravity,
            ],
        ),
    );
    assert!(runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["claude-code", "omp"]}),
    )));
    let Some(KitJob::Apply(scope)) = runtime.take_local_kit_job(Instant::now()) else {
        panic!("the changed agents are queued");
    };
    assert_eq!(agent_ids(&scope.agent_on), ["omp"]);
    assert_eq!(
        agent_ids(&scope.agent_off),
        ["codex"],
        "an agent that is not installed here has no switch, so it stays on"
    );
    assert_eq!(step(&runtime), None);
    assert_eq!(
        runtime.snapshot.ui_state.agent_onboarding_agents,
        ["claude-code", "omp"],
        "the next imported device uses the revised Help choice"
    );
}

/// D-21, B30, B43: the notification switch and the screens introduced once
/// survive a shared UI save and a restart.
#[test]
fn the_notification_switch_and_screen_intros_survive_a_save_and_a_restart() {
    let folder = scratch_dir("herdr-core-first-run-switch-");
    let path = folder.path().join("state.json");
    let mut runtime = runtime_at(&path);
    let notifications = |enabled: bool| {
        event(
            "desktop_notifications_set",
            serde_json::json!({"enabled": enabled}),
        )
    };
    assert!(runtime.dispatch_json(&notifications(true)));
    assert!(!runtime.dispatch_json(&notifications(true)));
    let done = event(
        "desktop_notifications_done_set",
        serde_json::json!({"enabled": true}),
    );
    assert!(runtime.dispatch_json(&done));
    assert!(!runtime.dispatch_json(&done));
    let intro = event(
        "screen_intro_seen",
        serde_json::json!({"screen": "factory"}),
    );
    assert!(runtime.dispatch_json(&intro));
    assert!(!runtime.dispatch_json(&intro));

    // A save built from a snapshot older than both carries neither back.
    let stale = ui_state_update(
        &runtime,
        serde_json::json!({"desktop_notifications": false, "desktop_notifications_done": false, "screen_intros_seen": []}),
    );
    runtime.dispatch_json(&stale);
    assert!(runtime.snapshot().ui_state.desktop_notifications);
    assert!(runtime.snapshot().ui_state.desktop_notifications_done);
    drop(runtime);

    let mut restarted = runtime_at(&path);
    assert!(restarted.snapshot().ui_state.desktop_notifications);
    assert!(restarted.snapshot().ui_state.desktop_notifications_done);
    restarted.dispatch_json(&event(
        "screen_intro_seen",
        serde_json::json!({"screen": "settings"}),
    ));
    assert_eq!(
        restarted
            .snapshot()
            .ui_state
            .screen_intros_seen
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [ScreenIntro::Factory],
        "an unknown screen is refused"
    );
}

/// Operator decision 2026-10-13: preserve both old on/off choices, then
/// persist either new switch independently, including Done-only.
#[test]
fn the_old_shared_choice_migrates_and_each_notification_choice_survives_restart() {
    for old in [false, true] {
        let folder = scratch_dir("herdr-core-notification-migration-");
        let path = folder.path().join("state.json");
        let mut runtime = runtime_at(&path);
        runtime.dispatch_json(&event(
            "desktop_notifications_set",
            serde_json::json!({"enabled": !old}),
        ));
        runtime.dispatch_json(&event(
            "desktop_notifications_set",
            serde_json::json!({"enabled": old}),
        ));
        drop(runtime);
        let mut stored: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        stored
            .as_object_mut()
            .unwrap()
            .remove("desktop_notifications_done");
        std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let mut runtime = runtime_at(&path);
        assert_eq!(runtime.snapshot().ui_state.desktop_notifications, old);
        assert_eq!(runtime.snapshot().ui_state.desktop_notifications_done, old);
        for (needs_you, done) in [(true, false), (false, true), (true, true), (false, false)] {
            runtime.dispatch_json(&event(
                "desktop_notifications_set",
                serde_json::json!({"enabled": needs_you}),
            ));
            runtime.dispatch_json(&event(
                "desktop_notifications_done_set",
                serde_json::json!({"enabled": done}),
            ));
            drop(runtime);
            runtime = runtime_at(&path);
            assert_eq!(runtime.snapshot().ui_state.desktop_notifications, needs_you);
            assert_eq!(runtime.snapshot().ui_state.desktop_notifications_done, done);
        }
    }
}

#[test]
fn back_preserves_choices_and_never_queues_an_install_or_changes_notifications() {
    let folder = scratch_dir("herdr-core-first-run-back-");
    let path = folder.path().join("state.json");
    let mut runtime = runtime_at(&path);
    report_here(&mut runtime, &first_run_pass());
    runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["claude-code"]}),
    ));
    run_queued_pass(
        &mut runtime,
        &pass(
            false,
            vec![
                agent("claude-code", true, ComponentState::Installed),
                agent("codex", false, ComponentState::Off),
            ],
        ),
    );
    runtime.dispatch_json(&event(
        "first_run_advance",
        serde_json::json!({"from": "notifications", "notifications": false}),
    ));
    runtime.dispatch_json(&event("first_run_draft", serde_json::json!({"project_device": "studio", "project_path": "~/my-project", "project_form": {"view": "clone", "url": "https://example.test/repo.git", "parent": "~/work", "name": "my-project"}})));
    assert!(runtime.dispatch_json(&event(
        "first_run_back",
        serde_json::json!({"from": "project"})
    )));
    assert!(
        !runtime.dispatch_json(&event(
            "first_run_back",
            serde_json::json!({"from": "project"})
        )),
        "a stale Back cannot move the next step"
    );
    assert!(runtime.dispatch_json(&event(
        "first_run_back",
        serde_json::json!({"from": "notifications"})
    )));
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
    assert!(!runtime.snapshot.ui_state.desktop_notifications);
    assert!(runtime.take_local_kit_job(Instant::now()).is_none());
    drop(runtime);
    let restarted = runtime_at(&path);
    assert_eq!(step(&restarted), Some(FirstRunStep::Agents));
    let choices = &restarted.snapshot.ui_state.first_run_choices;
    assert_eq!(
        choices.agents.as_deref(),
        Some(["claude-code".to_owned()].as_slice())
    );
    assert_eq!(choices.project_device.as_deref(), Some("studio"));
    assert_eq!(choices.project_path, "~/my-project");
    assert_eq!(choices.project_form.view, "clone");
    assert_eq!(choices.project_form.url, "https://example.test/repo.git");
    assert!(choices.notifications_answered);
}

#[test]
fn setup_url_drafts_never_publish_or_restore_credentials_and_keep_clean_urls() {
    let folder = scratch_dir("herdr-core-first-run-url-draft-");
    let path = folder.path().join("state.json");
    let mut runtime = runtime_at(&path);
    report_here(&mut runtime, &first_run_pass());
    for url in [
        "https://example.test/team/repo.git",
        "example.test:team/repo.git",
        "file:///work/repo.git",
        "https://alice:draft-password@example.test/team/repo.git",
        "https://draft-token@example.test/team/repo.git",
        "https://example.test/team/repo.git?token=draft-token",
        "https://example.test/team/repo.git#draft-token",
        "git@example.test:team/repo.git",
        "https://alice:draft-password@",
        "draft-password",
    ] {
        runtime.dispatch_json(&event(
            "first_run_draft",
            serde_json::json!({
                "project_form": {"view": "clone", "url": url, "parent": "~/work"}
            }),
        ));
        let clean = matches!(
            url,
            "https://example.test/team/repo.git"
                | "example.test:team/repo.git"
                | "file:///work/repo.git"
        );
        let expected = if clean { url } else { "" };
        let snapshot = serde_json::to_value(runtime.snapshot()).unwrap();
        assert_eq!(
            snapshot["ui_state"]["first_run_choices"]["project_form"]["url"],
            expected
        );
        drop(runtime);
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(!stored.contains("draft-password") && !stored.contains("draft-token"));
        runtime = runtime_at(&path);
        assert_eq!(
            runtime.snapshot.ui_state.first_run_choices.project_form.url,
            expected
        );
    }

    // A store from before this boundary is sanitized before the first snapshot.
    drop(runtime);
    let mut legacy: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    legacy["first_run_choices"]["project_form"]["url"] =
        serde_json::json!("https://alice:draft-password@example.test/repo.git?token=draft-token");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let restarted = runtime_at(&path);
    let published = serde_json::to_string(restarted.snapshot()).unwrap();
    assert!(!published.contains("draft-password") && !published.contains("draft-token"));
    assert!(
        restarted
            .snapshot
            .ui_state
            .first_run_choices
            .project_form
            .url
            .is_empty()
    );
}

#[test]
fn finishing_setup_clears_the_project_draft_before_saving() {
    let folder = scratch_dir("herdr-core-first-run-finished-draft-");
    let path = folder.path().join("state.json");
    let mut runtime = runtime_at(&path);
    runtime.snapshot.ui_state.first_run = Some(FirstRun {
        version: crate::model::FIRST_RUN_VERSION,
        step: FirstRunStep::Project,
    });
    runtime.dispatch_json(&event("first_run_draft", serde_json::json!({
        "project_device": "build", "project_path": "~/work/repo",
        "project_form": {"view": "clone", "url": "https://example.test/repo.git", "parent": "~/work", "name": "repo"}
    })));
    runtime.dispatch_json(&event(
        "first_run_advance",
        serde_json::json!({"from": "project"}),
    ));
    drop(runtime);
    let restarted = runtime_at(&path);
    let choices = &restarted.snapshot.ui_state.first_run_choices;
    assert_eq!(step(&restarted), Some(FirstRunStep::Done));
    assert_eq!(choices.project_device, None);
    assert!(choices.project_path.is_empty());
    assert_eq!(
        choices.project_form,
        crate::model::FirstRunProjectForm::default()
    );
}

#[test]
fn only_a_successful_empty_discovery_skips_devices_and_back_follows_that_route() {
    let mut runtime = runtime();
    runtime.snapshot.ui_state.first_run = Some(FirstRun {
        version: crate::model::FIRST_RUN_VERSION,
        step: FirstRunStep::Devices,
    });
    for state in ["idle", "loading", "failed"] {
        runtime.snapshot.status.herdr_profiles.state = state.to_owned();
        assert!(
            !runtime.settle_first_run_devices(),
            "{state} is not an empty saved list"
        );
        assert_eq!(step(&runtime), Some(FirstRunStep::Devices));
    }
    runtime.snapshot.status.herdr_profiles.state = "ready".to_owned();
    assert!(runtime.settle_first_run_devices());
    assert_eq!(step(&runtime), Some(FirstRunStep::Notifications));
    assert!(runtime.dispatch_json(&event(
        "first_run_back",
        serde_json::json!({"from": "notifications"})
    )));
    assert_eq!(step(&runtime), Some(FirstRunStep::Agents));
}

#[test]
fn an_in_flight_apply_keeps_its_choice_and_a_late_report_cannot_move_another_step() {
    let mut runtime = runtime();
    report_here(&mut runtime, &first_run_pass());
    runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["claude-code"]}),
    ));
    assert!(!runtime.dispatch_json(&event(
        "first_run_draft",
        serde_json::json!({"agents": ["codex"]})
    )));
    assert_eq!(
        runtime.snapshot.ui_state.first_run_choices.agents,
        Some(vec!["claude-code".to_owned()])
    );
    // A different accepted navigation owner superseded this pass while it ran.
    runtime.snapshot.ui_state.first_run.as_mut().unwrap().step = FirstRunStep::Project;
    run_queued_pass(
        &mut runtime,
        &pass(
            false,
            vec![agent("claude-code", true, ComponentState::Installed)],
        ),
    );
    assert_eq!(step(&runtime), Some(FirstRunStep::Project));
}

#[test]
fn a_saved_profile_keeps_the_devices_route_even_when_disabled() {
    let mut runtime = runtime();
    report_here(&mut runtime, &first_run_pass());
    runtime
        .snapshot
        .status
        .herdr_profiles
        .profiles
        .push(crate::model::HerdrProfileSnapshot {
            id: "disabled".to_owned(),
            label: "Build server".to_owned(),
            target: "build.example".to_owned(),
            session: "review".to_owned(),
            wsl_distro: None,
            enabled: false,
            added_as: None,
            problem: Some("disabled".to_owned()),
            device_id: None,
            connect_state: "saved".to_owned(),
            connect_refusal: None,
        });
    runtime.dispatch_json(&event(
        "agent_onboarding_apply",
        serde_json::json!({"agents": ["claude-code"]}),
    ));
    run_queued_pass(
        &mut runtime,
        &pass(
            false,
            vec![agent("claude-code", true, ComponentState::Installed)],
        ),
    );
    assert_eq!(step(&runtime), Some(FirstRunStep::Devices));
    assert!(runtime.dispatch_json(&event(
        "first_run_advance",
        serde_json::json!({"from": "devices"})
    )));
    assert!(runtime.dispatch_json(&event(
        "first_run_back",
        serde_json::json!({"from": "notifications"})
    )));
    assert_eq!(step(&runtime), Some(FirstRunStep::Devices));
    assert!(runtime.snapshot.ui_state.device_registrations.is_empty());
}
