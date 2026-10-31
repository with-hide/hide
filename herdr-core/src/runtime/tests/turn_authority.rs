//! The row and the letter doorbell read one turn: when the hook, Herdr and
//! the session read disagree, what the screen shows and why a letter waits
//! come from the same decision (`agent_state::turn_projection`), while
//! whether the pane takes input stays its own gate (`docs/delivery.md`, Safe
//! intake). Each case is driven through the path the core takes in a sync
//! pass: the hook's report, the delivery observation of Herdr's list, then
//! the session ingest that lays the hook's and the session read's facts.
//! The expected answers are the documented ranks (`docs/status-model.md`,
//! Where each axis comes from), not the projector's output.

use super::*;
use crate::delivery::doorbell::{Hold, Patience};
use hide_node_link::panes::HookTurn;
use hide_session::turns::{ToolTurnMark, TurnMark, TurnMode};

const PANE: &str = "w1:p1";
const NATIVE: &str = "native-1";

/// What one pane shows Herdr, its hook and its session read.
struct Witnesses {
    kind: &'static str,
    herdr: &'static str,
    hook: Option<HookTurn>,
    /// The session read's records, for the Herdr state the pane is in.
    session: &'static [Read],
}

#[derive(Clone, Copy)]
enum Read {
    /// A turn that ran and finished.
    Finished,
    /// A plan-mode turn that proposed a plan and finished.
    PlanProposed,
    /// An asynchronous question the agent asked and went on working past.
    AsyncQuestion,
}

fn marks(reads: &[Read]) -> Vec<TurnMark> {
    let turn = Some("turn-1".to_owned());
    reads
        .iter()
        .flat_map(|read| match read {
            Read::Finished => vec![
                TurnMark::Started {
                    turn: turn.clone(),
                    mode: TurnMode::Other,
                },
                TurnMark::Completed { turn: turn.clone() },
            ],
            Read::PlanProposed => vec![
                TurnMark::Started {
                    turn: turn.clone(),
                    mode: TurnMode::Plan,
                },
                TurnMark::Plan { turn: turn.clone() },
                TurnMark::Completed { turn: turn.clone() },
            ],
            Read::AsyncQuestion => vec![
                TurnMark::Started {
                    turn: turn.clone(),
                    mode: TurnMode::Other,
                },
                TurnMark::Tools(vec![ToolTurnMark::AskedAsync {
                    call: "call-1".to_owned(),
                    questions: 1,
                    content: None,
                }]),
            ],
        })
        .collect()
}

/// The pane's state after one sync pass, seen by the row and the doorbell.
struct Seen {
    activity: String,
    demand: String,
    blocked: bool,
    /// The doorbell's verdict for a letter that arrived just now.
    fresh: Result<(), Hold>,
    /// The verdict for a letter that has waited out a working agent.
    waited: Result<(), Hold>,
}

/// One sync pass over `witnesses`, then an hour with no key and no change
/// of Herdr's state; the runtime and the pane's agent as the ledger names it.
fn synced(witnesses: &Witnesses) -> (Runtime, crate::delivery::Actor) {
    let mut runtime = runtime();
    if let Some(turn) = witnesses.hook {
        let report = hide_node_link::panes::HookStatusReport {
            runtime: hide_agent_adapter::adapter(witnesses.kind)
                .expect("a known agent")
                .id
                .to_owned(),
            version: hide_agent_hooks::HOOK_VERSION,
            session: Some(NATIVE.to_owned()),
            turn: Some(turn),
            counts: None,
            session_start: false,
            waiting: None,
        };
        runtime
            .ingest_hook_status(crate::node::TEST_NODE, PANE, &report)
            .expect("the report is recorded");
    }
    let (payload, overlay) = sync_inputs(witnesses);
    // The order of a sync pass: delivery observes Herdr's list, then the
    // session ingest lays the facts and the turn.
    runtime.observe_delivery(crate::node::TEST_NODE, &payload, None, Some(&overlay));
    runtime.set_label_overlay(overlay);
    runtime.ingest_session(Ok(payload));
    // Nobody has typed for an hour and Herdr's state is old news.
    let long_ago = unix_milliseconds() - 3_600_000;
    let observation = runtime
        .delivery_observations
        .get_mut(PANE)
        .expect("the pane is observed");
    observation.status_changed_at_unix_ms = long_ago;
    observation.last_input_at_unix_ms = long_ago - 2;
    observation.last_submit_at_unix_ms = long_ago - 1;
    observation.entered_working_at_unix_ms = long_ago;
    assert!(
        observation.projected_turn().is_some(),
        "the ingest after an observation lays its turn"
    );
    let actor = observation.actor.clone();
    (runtime, actor)
}

/// Herdr's list and the session read's overlay one sync pass reads for
/// `witnesses`, at Herdr's state sequence 4.
fn sync_inputs(
    witnesses: &Witnesses,
) -> (SessionSnapshotPayload, crate::labels::overlay::LabelOverlay) {
    let seq = 4;
    let mut turns = hide_session::turns::TurnTracker::default();
    for (offset, mark) in marks(witnesses.session).iter().enumerate() {
        turns.fold(offset as u64 * 10, mark);
    }
    let record = crate::labels::store::PaneRecord {
        owner: hide_session::label_reference_token(witnesses.kind, "id", NATIVE),
        turns: Some(turns),
        turns_seq: Some(seq),
        ..Default::default()
    };
    let overlay = crate::labels::overlay::LabelOverlay::of_records(
        [(&PANE.to_owned(), &record)],
        true,
        false,
    );
    let herdr = serde_json::json!({
        "agents": [{
            "id": "agent-1", "pane_id": PANE, "agent": witnesses.kind,
            "agent_status": witnesses.herdr, "state_change_seq": seq,
            "cwd": "/fixture", "workspace_label": "Fixture",
            "agent_session": {"kind": "id", "value": NATIVE},
            "lineage_session": "lineage-1",
        }],
        "panes": [{"pane_id": PANE, "cwd": "/fixture", "tokens": {}}],
        "tabs": [{"workspace_id": "w1", "tab_id": "t0", "label": ""}],
        "layouts": [{
            "workspace_id": "w1", "tab_id": "t0", "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": PANE,
            "panes": [{"pane_id": PANE, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": [],
        }],
    });
    (
        crate::sidebar::owned_label_fixture(herdr).expect("session payload"),
        overlay,
    )
}

/// The doorbell's verdict at `now` for a letter that arrived at `arrived`.
fn verdict(
    runtime: &Runtime,
    actor: &crate::delivery::Actor,
    arrived_at_unix_ms: u64,
    now: u64,
) -> Result<(), Hold> {
    verdict_with(runtime, actor, arrived_at_unix_ms, now, (false, false))
}

/// The verdict for a letter sent with `(now, forced)`.
fn verdict_with(
    runtime: &Runtime,
    actor: &crate::delivery::Actor,
    arrived_at_unix_ms: u64,
    now: u64,
    (due, forced): (bool, bool),
) -> Result<(), Hold> {
    runtime
        .delivery_bell_verdict(
            actor,
            Patience {
                arrived_at_unix_ms,
                now: due,
                forced,
            },
            now,
        )
        .map(|_| ())
}

fn pass(witnesses: &Witnesses) -> Seen {
    let (runtime, actor) = synced(witnesses);
    let now = unix_milliseconds();
    let fresh = verdict(&runtime, &actor, now, now);
    let waited = verdict(&runtime, &actor, now - 3_600_000, now);
    let row = runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .find(|row| row.pane_id == PANE)
        .expect("the agent is a row");
    Seen {
        activity: row.activity.clone(),
        demand: row.demand.clone(),
        blocked: row.blocked,
        fresh,
        waited,
    }
}

/// Mission-23 disagreement 1: the hook says the turn ended while Herdr still
/// reads the screen as working. The row is stopped, so a letter is rung at
/// rest instead of waiting out five minutes of work Herdr no longer does.
#[test]
fn a_turn_the_hook_ended_while_herdr_reads_work_is_stopped_and_rings_at_rest() {
    let seen = pass(&Witnesses {
        kind: "codex",
        herdr: "working",
        hook: Some(HookTurn::Done),
        session: &[Read::Finished],
    });
    assert_eq!(seen.activity, "stopped");
    assert_eq!(seen.fresh, Ok(()));
}

/// Mission-23 disagreement 2 (#348): the hook's turn runs while Herdr reads
/// a quiet screen. The row works, and a letter waits for the mid-turn wait
/// rather than being rung as if the agent rested.
#[test]
fn a_hook_turn_herdr_reads_as_a_stop_works_and_holds_a_fresh_letter_as_working() {
    let seen = pass(&Witnesses {
        kind: "codex",
        herdr: "idle",
        hook: Some(HookTurn::Working),
        session: &[Read::Finished],
    });
    assert_eq!(seen.activity, "working");
    assert_eq!(seen.fresh, Err(Hold::Working));
    assert_eq!(seen.waited, Ok(()), "the mid-turn wait is over");
}

/// Mission-23 disagreement 3: a hook that reported the turn outranks
/// Herdr's blocked prompt on the screen, yet Herdr refuses input there, so
/// the letter waits on Herdr's readiness, named apart from the turn.
#[test]
fn herdr_blocked_under_a_reported_turn_shows_no_approval_and_holds_as_not_ready() {
    let seen = pass(&Witnesses {
        kind: "codex",
        herdr: "blocked",
        hook: Some(HookTurn::Done),
        session: &[Read::Finished],
    });
    assert_eq!((seen.activity.as_str(), seen.blocked), ("stopped", false));
    assert_eq!(seen.fresh, Err(Hold::NotReady));
}

/// With no hook turn, Herdr's blocked prompt is the approval on both sides.
#[test]
fn herdr_blocked_with_no_hook_turn_is_an_approval_on_the_row_and_for_the_letter() {
    let seen = pass(&Witnesses {
        kind: "codex",
        herdr: "blocked",
        hook: None,
        session: &[Read::Finished],
    });
    assert_eq!((seen.demand.as_str(), seen.blocked), ("approval", true));
    assert_eq!(seen.waited, Err(Hold::Blocked));
}

/// The hook's permission wait outranks Herdr's stop on both sides.
#[test]
fn a_hook_permission_wait_herdr_reads_as_a_stop_is_an_approval_on_both_sides() {
    let seen = pass(&Witnesses {
        kind: "codex",
        herdr: "idle",
        hook: Some(HookTurn::Waiting),
        session: &[Read::Finished],
    });
    assert_eq!((seen.demand.as_str(), seen.blocked), ("approval", true));
    assert_eq!(seen.waited, Err(Hold::Blocked));
}

/// A question the agent asked and worked past stays the operator's on the
/// row, and the letter waits for it, whatever the hook and Herdr say of the
/// work.
#[test]
fn an_open_async_question_under_a_working_hook_is_a_question_and_holds_the_letter() {
    let seen = pass(&Witnesses {
        kind: "codex",
        herdr: "working",
        hook: Some(HookTurn::Working),
        session: &[Read::AsyncQuestion],
    });
    assert_eq!(
        (seen.activity.as_str(), seen.demand.as_str()),
        ("working", "question")
    );
    assert_eq!(seen.waited, Err(Hold::AwaitingOperator));
}

/// Mission-23 disagreement 4: a plan approval read from how the last turn
/// ended is over once the hook says the agent works again; the row shows no
/// approval and the letter follows the work.
#[test]
fn a_plan_approval_read_under_a_working_hook_is_over_on_both_sides() {
    let seen = pass(&Witnesses {
        kind: "codex",
        herdr: "done",
        hook: Some(HookTurn::Working),
        session: &[Read::PlanProposed],
    });
    assert_eq!((seen.activity.as_str(), seen.blocked), ("working", false));
    assert_eq!(seen.fresh, Err(Hold::Working));
    // With no hook turn, the same read holds both.
    let seen = pass(&Witnesses {
        kind: "codex",
        herdr: "done",
        hook: None,
        session: &[Read::PlanProposed],
    });
    assert!(seen.blocked);
    assert_eq!(seen.waited, Err(Hold::AwaitingOperator));
}

/// Disagreement 1 rings a pane Herdr still reads as working, so the gates
/// that keep the operator's composer whole are Herdr's and Hide's own
/// clocks, never the turn: a key routed since the last submission and since
/// Herdr's start of work holds the letter as a draft, a change of Herdr's
/// state within the quiet period holds it as quiet, and a kind that takes
/// letters at its tools is not rung while Herdr reads work, since Herdr
/// reads Grok's plan review as work.
#[test]
fn a_stop_the_hook_reports_under_herdrs_work_still_keeps_the_draft_quiet_and_tool_gates() {
    let witnesses = Witnesses {
        kind: "codex",
        herdr: "working",
        hook: Some(HookTurn::Done),
        session: &[Read::Finished],
    };
    let now = unix_milliseconds();
    let (mut runtime, actor) = synced(&witnesses);
    runtime
        .delivery_observations
        .get_mut(PANE)
        .unwrap()
        .last_input_at_unix_ms = now - 60_000;
    assert_eq!(verdict(&runtime, &actor, now, now), Err(Hold::Draft));
    let (mut runtime, actor) = synced(&witnesses);
    runtime
        .delivery_observations
        .get_mut(PANE)
        .unwrap()
        .status_changed_at_unix_ms = now - 5_000;
    assert_eq!(verdict(&runtime, &actor, now, now), Err(Hold::Quiet));
    let (runtime, actor) = synced(&Witnesses {
        kind: "grok",
        ..witnesses
    });
    assert_eq!(verdict(&runtime, &actor, now, now), Err(Hold::Working));
}

/// The turn the core decides stands only for the Herdr state it was laid
/// under: an observation of a newer state holds the letter as
/// `turn_unprojected` until the session ingest of the same sync pass lays
/// the turn again, which every pass does.
#[test]
fn a_newer_state_is_unprojected_only_until_the_same_pass_ingests_it() {
    let (mut runtime, actor) = synced(&Witnesses {
        kind: "codex",
        herdr: "idle",
        hook: None,
        session: &[],
    });
    let now = unix_milliseconds();
    let herdr: SessionSnapshotPayload = crate::sidebar::owned_label_fixture(serde_json::json!({
        "agents": [{
            "id": "agent-1", "pane_id": PANE, "agent": "codex",
            "agent_status": "idle", "state_change_seq": 5,
            "cwd": "/fixture", "workspace_label": "Fixture",
            "agent_session": {"kind": "id", "value": NATIVE},
            "lineage_session": "lineage-1",
        }],
        "panes": [{"pane_id": PANE, "cwd": "/fixture", "tokens": {}}],
        "tabs": [{"workspace_id": "w1", "tab_id": "t0", "label": ""}],
        "layouts": [{
            "workspace_id": "w1", "tab_id": "t0", "zoomed": false,
            "area": {"x": 0, "y": 0, "width": 80, "height": 24},
            "focused_pane_id": PANE,
            "panes": [{"pane_id": PANE, "rect": {"x": 0, "y": 0, "width": 80, "height": 24}}],
            "splits": [],
        }],
    }))
    .unwrap();
    runtime.observe_delivery(crate::node::TEST_NODE, &herdr, None, None);
    assert_eq!(verdict(&runtime, &actor, now, now), Err(Hold::Unprojected));
    runtime.ingest_session(Ok(herdr));
    // The turn is laid; what holds now is the session read, which has not
    // read the new state.
    assert_eq!(
        verdict(&runtime, &actor, now, now),
        Err(Hold::SessionUnread)
    );
}

/// Review of #1954, P1: a sync pass observes Herdr's list under the lock,
/// releases it, and lays the hook's and the session read's facts after. A
/// question read in that pass while Herdr's state and sequence stay the same
/// must not ring on the turn the previous pass laid, which had no question:
/// between the two steps every letter, a due and a forced one too, holds as
/// `turn_unprojected`, and once the ingest lays the facts the row shows the
/// question and the letter waits for the operator.
#[test]
fn a_question_read_under_an_unchanged_herdr_state_never_rings_on_the_last_turn() {
    let settled = Witnesses {
        kind: "codex",
        herdr: "idle",
        hook: None,
        session: &[Read::Finished],
    };
    let (mut runtime, actor) = synced(&settled);
    let now = unix_milliseconds();
    let long_ago = now - 3_600_000;
    assert_eq!(verdict(&runtime, &actor, long_ago, now), Ok(()));
    let asking = Witnesses {
        session: &[Read::AsyncQuestion],
        ..settled
    };
    let (payload, overlay) = sync_inputs(&asking);
    runtime.observe_delivery(crate::node::TEST_NODE, &payload, None, Some(&overlay));
    // The pause: the lock is free and the facts are not laid yet. Herdr's
    // state and sequence did not move, so only the pass fences the turn.
    let observation = &runtime.delivery_observations[PANE];
    assert_eq!(
        (observation.status.as_str(), observation.state_change_seq),
        ("idle", Some(4))
    );
    for letter in [(false, false), (true, false), (true, true)] {
        assert_eq!(
            verdict_with(&runtime, &actor, long_ago, now, letter),
            Err(Hold::Unprojected),
            "{letter:?}"
        );
    }
    runtime.set_label_overlay(overlay);
    runtime.ingest_session(Ok(payload));
    let row = runtime
        .snapshot
        .navigator
        .agents
        .iter()
        .find(|row| row.pane_id == PANE)
        .expect("the agent is a row");
    assert_eq!(row.demand, "question");
    for letter in [(false, false), (true, false), (true, true)] {
        assert_eq!(
            verdict_with(&runtime, &actor, long_ago, now, letter),
            Err(Hold::AwaitingOperator),
            "{letter:?}"
        );
    }
}
