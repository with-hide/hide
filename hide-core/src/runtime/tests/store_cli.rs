//! Store requests answered one by one, and `hide skill` waiting on the
//! operator's confirm (PRD skill-mcp-manager B31-B33, D-19): requests merged
//! into one pass are answered each with its own changes' outcomes, the
//! answers kept are bounded, and an agent's add stands as its person's turn
//! until the operator answers it.

use super::device_kit::{DEVICE, KitDevice, dispatch, report, settle, with_consent};
use super::*;
use crate::agent_profiles::AnswerState;
use crate::coordination::human_turn::HumanKind;
use crate::runtime::store_cli::StoreAskWhat;
use crate::store_command::{DECLINED, StoreCommand};
use hide_kit::store::{
    ChangeOutcome, FoundSkill, Preview, SourceSpec, StoreChange, StoreRefusal, StoreRefusalCause,
    StoreReport,
};
use hide_node_link::protocol::KitAction;

const URL: &str = "https://github.com/acme/archify";
const PANE: &str = "w1:p1";

fn source() -> SourceSpec {
    SourceSpec::Git {
        url: URL.to_owned(),
    }
}

fn found(name: &str, taken: bool) -> FoundSkill {
    FoundSkill {
        name: name.to_owned(),
        path: name.to_owned(),
        description: None,
        files: Vec::new(),
        digest: format!("digest-{name}"),
        taken,
    }
}

/// The machine's store after a pass: the source read as two skills, one of
/// them already in the store, and the outcomes of the pass's changes.
fn answered(outcomes: Vec<ChangeOutcome>) -> serde_json::Value {
    serde_json::to_value(StoreReport {
        previews: vec![Preview {
            id: "p1".to_owned(),
            source: source(),
            url: Some(URL.to_owned()),
            commit: Some("0123456".to_owned()),
            tag: None,
            reference: None,
            skills: vec![found("archify", false), found("notes", true)],
            agents: vec!["claude-code".to_owned()],
            at_unix_ms: 1,
        }],
        outcomes,
        ..StoreReport::default()
    })
    .unwrap()
}

fn done(change: StoreChange) -> ChangeOutcome {
    ChangeOutcome {
        change,
        refused: None,
    }
}

fn answer_of(shared: &Mutex<Runtime>, id: &str) -> Option<crate::model::StoreAnswer> {
    let runtime = shared.lock().unwrap();
    let snapshot = runtime.snapshot();
    snapshot
        .status
        .store_answers
        .iter()
        .find(|answer| answer.request_id == id)
        .cloned()
}

fn store_calls(helper: &KitDevice) -> Vec<KitAction> {
    helper
        .calls()
        .into_iter()
        .map(|(action, _)| action)
        .filter(|action| matches!(action, KitAction::Store { .. }))
        .collect()
}

#[test]
fn requests_merged_into_one_pass_are_each_answered_with_their_own_outcomes() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let first = StoreChange::Delete {
        item: "skill:gone".to_owned(),
    };
    let second = StoreChange::CheckUpdates;
    helper.answer_store(Ok(answered(vec![
        ChangeOutcome {
            change: first.clone(),
            refused: Some(StoreRefusal {
                cause: StoreRefusalCause::NotFound,
                reason: "no item skill:gone".to_owned(),
            }),
        },
        done(second.clone()),
    ])));
    let shared = with_consent(Some(helper.clone()));
    {
        // Both are queued before the device's worker can take either.
        let mut runtime = shared.lock().unwrap();
        runtime.request_store_change(DEVICE, "r1".to_owned(), vec![first.clone()]);
        runtime.request_store_change(DEVICE, "r2".to_owned(), vec![second.clone()]);
    }
    settle(&shared);

    assert_eq!(
        store_calls(&helper),
        vec![KitAction::Store {
            changes: vec![first, second]
        }],
        "one pass carries both"
    );
    let r1 = answer_of(&shared, "r1").expect("r1 answered");
    let r2 = answer_of(&shared, "r2").expect("r2 answered");
    assert_eq!(r1.state, AnswerState::Done);
    assert_eq!(r1.outcomes.len(), 1);
    assert_eq!(
        r1.outcomes[0].refused.as_ref().map(|refusal| refusal.cause),
        Some(StoreRefusalCause::NotFound)
    );
    assert_eq!(r2.outcomes, vec![done(StoreChange::CheckUpdates)]);
}

#[test]
fn the_answers_kept_drop_the_oldest_finished_one_past_their_limit() {
    let helper = KitDevice::answering(Ok(report(&[])));
    helper.answer_store(Ok(answered(vec![done(StoreChange::CheckUpdates)])));
    let shared = with_consent(Some(helper));
    for request in 0..=crate::runtime::kit_store::ANSWER_LIMIT {
        dispatch(
            &shared,
            "kit_store",
            serde_json::json!({"device_id": DEVICE, "request_id": format!("r{request}"), "changes": [{"op": "check_updates"}]}),
        );
        settle(&shared);
    }
    let runtime = shared.lock().unwrap();
    let ids = runtime
        .snapshot()
        .status
        .store_answers
        .iter()
        .map(|answer| answer.request_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), crate::runtime::kit_store::ANSWER_LIMIT);
    assert!(!ids.contains(&"r0".to_owned()), "the oldest went first");
    assert!(ids.contains(&format!("r{}", crate::runtime::kit_store::ANSWER_LIMIT)));
}

/// The machine's store read once, then an agent in `PANE` running
/// `hide skill add <URL>`: its source is read and the confirm stands.
fn asked(helper: &Arc<KitDevice>) -> (SharedRuntime, crate::store_command::Prepared, String) {
    helper.answer_store(Ok(answered(vec![done(StoreChange::CheckUpdates)])));
    let shared = with_consent(Some(helper.clone()));
    dispatch(
        &shared,
        "kit_store",
        serde_json::json!({"device_id": DEVICE, "request_id": "read", "changes": [{"op": "check_updates"}]}),
    );
    settle(&shared);
    helper.answer_store(Ok(answered(vec![done(StoreChange::Inspect {
        source: source(),
    })])));
    let prepared = shared
        .lock()
        .unwrap()
        .prepare_store_command(
            DEVICE,
            PANE,
            StoreCommand::Add {
                source: source(),
                names: Vec::new(),
                machine: Some(DEVICE.to_owned()),
            },
        )
        .expect("planned");
    settle(&shared);
    let asks = shared.lock().unwrap().snapshot().status.store_asks.clone();
    assert_eq!(asks.len(), 1, "the confirm stands");
    assert_eq!(
        asks[0].what,
        StoreAskWhat::Install {
            preview: "p1".to_owned(),
            names: vec!["archify".to_owned()],
        },
        "only the skills not in the store yet are asked for"
    );
    assert_eq!(asks[0].pane_id, PANE);
    assert_eq!(
        shared
            .lock()
            .unwrap()
            .human_turns_laid
            .get(PANE)
            .map(|turn| turn.kind),
        Some(HumanKind::Install),
        "the asking agent's row carries it as its person's turn"
    );
    (shared, prepared, asks[0].id.clone())
}

#[test]
fn an_agents_add_waits_on_the_confirm_and_cancel_ends_it_declined_with_nothing_installed() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let (shared, prepared, ask) = asked(&helper);

    dispatch(
        &shared,
        "store_ask_answer",
        serde_json::json!({"id": ask, "confirm": false}),
    );
    assert_eq!(prepared.run(), Err(DECLINED.to_owned()));
    let runtime = shared.lock().unwrap();
    assert!(runtime.snapshot().status.store_asks.is_empty());
    assert!(!runtime.human_turns_laid.contains_key(PANE));
    drop(runtime);
    assert!(
        !store_calls(&helper)
            .iter()
            .any(|call| matches!(call, KitAction::Store { changes } if changes.iter().any(|change| matches!(change, StoreChange::Install { .. })))),
        "nothing was installed"
    );
}

#[test]
fn an_agents_add_installs_the_skills_the_operator_left_ticked_once_confirmed() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let (shared, prepared, ask) = asked(&helper);
    let install = StoreChange::Install {
        preview: "p1".to_owned(),
        names: vec!["archify".to_owned()],
    };
    helper.answer_store(Ok(answered(vec![done(install.clone())])));

    // A name the confirm did not ask for is dropped, never installed.
    dispatch(
        &shared,
        "store_ask_answer",
        serde_json::json!({"id": ask, "confirm": true, "names": ["archify", "notes"]}),
    );
    settle(&shared);
    let value = prepared.run().expect("installed");
    assert_eq!(value["outcomes"][0]["refused"], serde_json::Value::Null);
    assert_eq!(
        store_calls(&helper).last(),
        Some(&KitAction::Store {
            changes: vec![install]
        })
    );
    assert!(!shared.lock().unwrap().human_turns_laid.contains_key(PANE));
}
