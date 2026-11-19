//! A machine's store of skills and MCP items as the runtime drives it (PRD
//! skill-mcp-manager): the screen's request reaches the device's store over
//! the host protocol, its row carries the items with their cells, version
//! and update, and the follow and the daily check run with no request.

use super::device_kit::{DEVICE, KitDevice, dispatch, kit, report, settle, with_consent};
use super::*;
use crate::agent_profiles::AnswerState;
use hide_kit::store::{
    Cell, CellState, ChangeOutcome, Column, ItemKind, ItemReport, Source, StoreChange,
    StoreRefusal, StoreRefusalCause, StoreReport, Version,
};
use hide_node_link::protocol::KitAction;

fn switch_off_agy() -> StoreChange {
    StoreChange::Set {
        item: "skill:style".to_owned(),
        column: Some("antigravity".to_owned()),
        on: false,
    }
}

/// One GitHub skill on a tagged commit with a newer tag out, on in Claude's
/// folders, waiting for the shared folder's agents and just switched off
/// for agy.
fn store_report(outcomes: Vec<ChangeOutcome>, checked_at_unix_ms: Option<u64>) -> StoreReport {
    let column = |id: &str, agents: &[&str]| Column {
        id: id.to_owned(),
        agents: agents.iter().map(|agent| (*agent).to_owned()).collect(),
        also: Vec::new(),
    };
    let cell = |column: &str, state, changed_at_unix_ms| Cell {
        column: column.to_owned(),
        state,
        reason: None,
        changed_at_unix_ms,
    };
    StoreReport {
        columns: [(
            ItemKind::Skill,
            vec![
                column("claude", &["claude"]),
                column("agents", &["codex", "grok", "opencode", "pi", "omp"]),
                column("antigravity", &["antigravity"]),
            ],
        )]
        .into_iter()
        .collect(),
        items: vec![ItemReport {
            id: "skill:style".to_owned(),
            kind: ItemKind::Skill,
            name: "style".to_owned(),
            source: Source::Git {
                url: "https://github.com/acme/skills".to_owned(),
                path: "style".to_owned(),
                reference: None,
                commit: "0123456789abcdef".to_owned(),
                tag: Some("v1.0".to_owned()),
            },
            version: Version::Tag("v1.0".to_owned()),
            update: Some(Version::Tag("v1.1".to_owned())),
            on: true,
            folder_missing: false,
            files: Vec::new(),
            cells: vec![
                cell("claude", CellState::On, None),
                cell("agents", CellState::Waiting, None),
                cell("antigravity", CellState::Off, Some(1_700_000_000_000)),
            ],
        }],
        checked_at_unix_ms,
        outcomes,
        ..StoreReport::default()
    }
}

fn answer(shared: &Mutex<Runtime>) -> Option<crate::model::StoreAnswer> {
    shared
        .lock()
        .unwrap()
        .snapshot()
        .status
        .store_answer
        .clone()
}

fn store_calls(helper: &KitDevice) -> Vec<KitAction> {
    helper
        .calls()
        .into_iter()
        .map(|(action, _)| action)
        .filter(|action| matches!(action, KitAction::Store { .. } | KitAction::FollowStore))
        .collect()
}

#[test]
fn a_store_request_reaches_the_device_and_its_row_carries_items_cells_version_and_update() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let outcome = ChangeOutcome {
        change: switch_off_agy(),
        refused: None,
    };
    helper.answer_store(Ok(serde_json::to_value(store_report(
        vec![outcome.clone()],
        None,
    ))
    .unwrap()));
    let shared = with_consent(Some(helper.clone()));
    let request = serde_json::json!({
        "device_id": DEVICE,
        "request_id": "r1",
        "changes": [{"op": "set", "item": "skill:style", "column": "antigravity", "on": false}],
    });

    assert!(dispatch(&shared, "kit_store", request.clone()));
    settle(&shared);

    assert_eq!(
        store_calls(&helper),
        vec![KitAction::Store {
            changes: vec![switch_off_agy()]
        }]
    );
    let store = kit(&shared).store.expect("the row carries the store");
    let item = &store.items[0];
    assert_eq!(item.version, Version::Tag("v1.0".to_owned()));
    assert_eq!(item.update, Some(Version::Tag("v1.1".to_owned())));
    assert_eq!(
        item.cells
            .iter()
            .map(|cell| (cell.column.as_str(), cell.state))
            .collect::<Vec<_>>(),
        vec![
            ("claude", CellState::On),
            ("agents", CellState::Waiting),
            ("antigravity", CellState::Off),
        ]
    );
    assert_eq!(
        store.columns[&ItemKind::Skill][1].agents,
        ["codex", "grok", "opencode", "pi", "omp"],
        "a column names the agents it covers, which the screen joins with the agent rows"
    );
    assert!(
        store.outcomes.is_empty(),
        "a change's outcome is the request's answer, not the row's"
    );
    let answered = answer(&shared).expect("the request is answered");
    assert_eq!(answered.request_id, "r1");
    assert_eq!(answered.state, AnswerState::Done);
    assert_eq!(answered.outcomes, vec![outcome]);

    // The same request again is the same intent: nothing is sent twice.
    assert!(!dispatch(&shared, "kit_store", request));
    settle(&shared);
    assert_eq!(store_calls(&helper).len(), 1);
}

#[test]
fn a_store_request_that_cannot_run_is_answered_refused_and_the_last_table_stays() {
    let helper = KitDevice::answering(Ok(report(&[])));
    helper.answer_store(Ok(
        serde_json::to_value(store_report(Vec::new(), None)).unwrap()
    ));
    let shared = with_consent(Some(helper.clone()));
    let request = |id: &str, device: &str| {
        serde_json::json!({
            "device_id": device,
            "request_id": id,
            "changes": [{"op": "check_updates"}],
        })
    };
    assert!(dispatch(&shared, "kit_store", request("r1", DEVICE)));
    settle(&shared);
    assert!(kit(&shared).store.is_some());

    // The helper stops answering: the request ends refused, and the table
    // the last report drew stays.
    helper.answer_store(Err("the helper went away".to_owned()));
    assert!(dispatch(&shared, "kit_store", request("r2", DEVICE)));
    settle(&shared);
    let answered = answer(&shared).unwrap();
    assert_eq!(
        (answered.request_id.as_str(), answered.state),
        ("r2", AnswerState::Refused)
    );
    assert_eq!(kit(&shared).store.unwrap().items.len(), 1);

    // A machine Hide does not know is refused before anything is sent.
    let sent = store_calls(&helper).len();
    assert!(dispatch(&shared, "kit_store", request("r3", "nowhere")));
    let answered = answer(&shared).unwrap();
    assert_eq!(
        (answered.request_id.as_str(), answered.state),
        ("r3", AnswerState::Refused)
    );
    settle(&shared);
    assert_eq!(store_calls(&helper).len(), sent);
}

#[test]
fn a_change_the_store_refuses_is_answered_with_its_cause() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let refused = ChangeOutcome {
        change: StoreChange::Delete {
            item: "skill:gone".to_owned(),
        },
        refused: Some(StoreRefusal {
            cause: StoreRefusalCause::NotFound,
            reason: "no item skill:gone".to_owned(),
        }),
    };
    helper.answer_store(Ok(serde_json::to_value(store_report(
        vec![refused.clone()],
        None,
    ))
    .unwrap()));
    let shared = with_consent(Some(helper));
    dispatch(
        &shared,
        "kit_store",
        serde_json::json!({
            "device_id": DEVICE,
            "request_id": "r1",
            "changes": [{"op": "delete", "item": "skill:gone"}],
        }),
    );
    settle(&shared);
    let answered = answer(&shared).unwrap();
    assert_eq!(answered.state, AnswerState::Done);
    assert_eq!(answered.outcomes, vec![refused]);
}

#[test]
fn a_github_item_is_checked_daily_and_a_followed_folder_every_few_seconds_without_a_request() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let shared = with_consent(Some(helper.clone()));
    let two_days_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 2 * 24 * 60 * 60 * 1000;
    let mut stale = store_report(Vec::new(), Some(two_days_ago));
    stale.follows = true;
    {
        let mut runtime = shared.lock().unwrap();
        let mut state = runtime.kit_state(DEVICE);
        state.store = Some(stale);
        runtime.set_kit_state(DEVICE, state);
    }
    let mut checked = store_report(
        vec![ChangeOutcome {
            change: StoreChange::CheckUpdates,
            refused: None,
        }],
        Some(two_days_ago + 2 * 24 * 60 * 60 * 1000),
    );
    checked.follows = true;
    helper.answer_store(Ok(serde_json::to_value(checked).unwrap()));

    let now = Instant::now();
    shared.lock().unwrap().schedule_store(now);
    settle(&shared);
    assert_eq!(
        store_calls(&helper),
        vec![KitAction::Store {
            changes: vec![StoreChange::CheckUpdates]
        }],
        "an item last checked two days ago is checked with no request"
    );

    // Checked now: only the follow is due a few seconds on, and a follow
    // that found no folder changed leaves the row as it was.
    helper.answer_store(Ok(serde_json::Value::Null));
    let before = kit(&shared);
    shared
        .lock()
        .unwrap()
        .schedule_store(now + Duration::from_secs(4));
    settle(&shared);
    assert_eq!(store_calls(&helper)[1..], [KitAction::FollowStore]);
    assert_eq!(kit(&shared), before);
}
