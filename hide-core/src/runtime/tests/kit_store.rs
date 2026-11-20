//! A machine's store of skills and MCP items as the runtime drives it (PRD
//! skill-mcp-manager): the screen's request reaches the device's store over
//! the host protocol, its row carries the items with their cells, version
//! and update, and the follow and the daily check run with no request.

use super::device_kit::{DEVICE, KitDevice, dispatch, kit, report, settle, with_consent};
use super::*;
use crate::agent_profiles::AnswerState;
use hide_kit::store::{
    Carried, CarriedFile, CarriedItem, Cell, CellState, ChangeOutcome, Column, ItemKind,
    ItemReport, Source, StoreChange, StoreExport, StoreRefusal, StoreRefusalCause, StoreReport,
    Version,
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
pub(super) fn store_report(
    outcomes: Vec<ChangeOutcome>,
    checked_at_unix_ms: Option<u64>,
) -> StoreReport {
    let column = |id: &str, agents: &[&str]| Column {
        id: id.to_owned(),
        agents: agents.iter().map(|agent| (*agent).to_owned()).collect(),
        also: Vec::new(),
    };
    let cell = |column: &str, state, changed_at_unix_ms| Cell {
        column: column.to_owned(),
        state,
        reason: None,
        failure: None,
        changed_at_unix_ms,
        place: None,
        path: None,
        server: None,
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
            owner: Some("acme".to_owned()),
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
            off_for: Default::default(),
            folder_missing: false,
            description: None,
            files: Vec::new(),
            server: None,
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
        .store_answers
        .last()
        .cloned()
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
fn a_new_profile_list_applies_existing_items_without_repeating_an_unchanged_read() {
    use crate::runtime::kit::ProfilesRead;
    use crate::runtime::kit_store::StoreJob;
    let mut runtime = runtime();
    super::agent_connection::kit_rows(&mut runtime, None);
    let node = runtime.node.to_string();
    runtime.ingest_store_answer(&node, Ok(Some(store_report(Vec::new(), None))));
    let listed = |version| {
        ProfilesRead::Listed(
            serde_json::from_value(serde_json::json!({
                "version": version,
                "profiles": [{"id": "p-work", "kind": "claude-code", "name": "work", "version": 1}]
            }))
            .unwrap(),
        )
    };

    runtime.ingest_profiles(&node, listed(1));
    assert_eq!(
        runtime.take_local_store_job(),
        Some(StoreJob::Pass(Vec::new()))
    );
    runtime.ingest_profiles(&node, listed(1));
    assert_eq!(runtime.take_local_store_job(), None);
    runtime.ingest_profiles(&node, ProfilesRead::Unreadable("not readable".to_owned()));
    assert_eq!(runtime.take_local_store_job(), None);
    runtime.ingest_profiles(&node, listed(2));
    assert_eq!(
        runtime.take_local_store_job(),
        Some(StoreJob::Pass(Vec::new()))
    );
}

#[test]
fn a_store_request_reaches_the_device_and_its_row_carries_items_cells_version_and_update() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let outcome = ChangeOutcome {
        change: switch_off_agy(),
        refused: None,
        made: None,
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
        made: None,
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
            made: None,
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

/// A request queued when the daily check falls due keeps its pass to
/// itself, since its answer is the outcomes its pass ends with; the check
/// runs on a later tick.
#[test]
fn a_daily_check_waits_while_a_request_is_queued_and_the_request_is_answered() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let shared = with_consent(Some(helper.clone()));
    let two_days_ago = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
        - 2 * 24 * 60 * 60 * 1000;
    {
        let mut runtime = shared.lock().unwrap();
        let mut state = runtime.kit_state(DEVICE);
        state.store = Some(store_report(Vec::new(), Some(two_days_ago)));
        runtime.set_kit_state(DEVICE, state);
    }
    helper.answer_store(Ok(serde_json::to_value(store_report(
        vec![ChangeOutcome {
            change: switch_off_agy(),
            refused: None,
            made: None,
        }],
        Some(two_days_ago),
    ))
    .unwrap()));

    let now = Instant::now();
    {
        let mut runtime = shared.lock().unwrap();
        runtime.request_store_change(DEVICE, "r1".to_owned(), vec![switch_off_agy()]);
        runtime.schedule_store(now);
    }
    settle(&shared);

    assert_eq!(
        store_calls(&helper),
        vec![KitAction::Store {
            changes: vec![switch_off_agy()]
        }]
    );
    assert_eq!(answer(&shared).unwrap().state, AnswerState::Done);
    shared.lock().unwrap().schedule_store(now);
    settle(&shared);
    assert_eq!(
        store_calls(&helper)[1..],
        [KitAction::Store {
            changes: vec![StoreChange::CheckUpdates]
        }]
    );
}

#[test]
fn a_request_whose_device_goes_before_its_pass_runs_ends_refused() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let shared = with_consent(Some(helper.clone()));

    {
        let mut runtime = shared.lock().unwrap();
        runtime.request_store_change(DEVICE, "r1".to_owned(), vec![switch_off_agy()]);
        runtime.forget_device_kit_work(DEVICE);
    }
    settle(&shared);

    let answered = answer(&shared).unwrap();
    assert_eq!(
        (answered.request_id.as_str(), answered.state),
        ("r1", AnswerState::Refused)
    );
    assert_eq!(store_calls(&helper), Vec::new());
}

#[test]
fn a_cell_changed_while_its_agent_ran_waits_for_a_new_session_until_that_session_ends() {
    use super::agent_connection::{diagnosis, feed, installed, kit_rows};
    let mut runtime = runtime();
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    feed(&mut runtime, &[("w1:p1", "claude", true)]);
    let claude = hide_agent_adapter::adapter("claude").unwrap().id;
    let mut changed = store_report(Vec::new(), None);
    changed.columns.get_mut(&ItemKind::Skill).unwrap()[0].agents = vec![claude.to_owned()];
    changed.items[0].cells[0].changed_at_unix_ms = Some(1_700_000_000_000);
    let marked = |runtime: &Runtime| {
        runtime.snapshot.navigator.devices[0]
            .kit
            .store_new_session
            .clone()
    };

    runtime.ingest_store_answer(crate::node::TEST_NODE, Ok(Some(changed)));
    assert_eq!(
        marked(&runtime),
        vec![crate::model::StoreCellRef {
            item: "skill:style".to_owned(),
            column: "claude".to_owned(),
        }],
        "only the column whose agent ran when the change landed waits"
    );

    // That session ends and a new one starts: the new one read the change.
    feed(&mut runtime, &[("w1:p2", "claude", true)]);
    assert_eq!(marked(&runtime), Vec::new());
}

#[test]
fn a_copy_from_this_mac_reaches_the_device_whole_and_its_answer_holds_no_file_bytes() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let shared = with_consent(Some(helper.clone()));
    let carried = CarriedItem {
        kind: ItemKind::Skill,
        name: "style".to_owned(),
        source: Source::Folder {
            path: "/Users/me/skills/style".to_owned(),
        },
        on: true,
        off_for: Default::default(),
        latest: None,
        content: Carried::Skill {
            files: vec![CarriedFile {
                path: "SKILL.md".to_owned(),
                executable: false,
                data: "U0tJTEwgYnl0ZXM=".to_owned(),
            }],
        },
    };
    let received = StoreChange::Receive {
        item: Box::new(carried.clone()),
    };
    let outcome = ChangeOutcome {
        change: received.redacted(),
        refused: None,
        made: None,
    };
    helper.answer_store(Ok(serde_json::to_value(store_report(
        vec![outcome.clone()],
        None,
    ))
    .unwrap()));

    assert!(dispatch(
        &shared,
        "kit_store_copy",
        serde_json::json!({"device_id": DEVICE, "request_id": "c1"}),
    ));
    // This Mac's kit worker reads its store and hands the answer back.
    let to = shared.lock().unwrap().take_store_export();
    assert_eq!(to.as_deref(), Some(DEVICE));
    assert!(shared.lock().unwrap().ingest_store_export(
        DEVICE,
        Ok(StoreExport {
            items: vec![carried],
            refused: None,
        }),
    ));
    settle(&shared);

    assert_eq!(
        store_calls(&helper),
        vec![KitAction::Store {
            changes: vec![received.clone()]
        }],
        "the device gets the item with its files"
    );
    let answered = answer(&shared).unwrap();
    assert_eq!(
        (answered.request_id.as_str(), answered.state),
        ("c1", AnswerState::Done)
    );
    assert_eq!(answered.outcomes, vec![outcome]);
    let wire = serde_json::to_string(&shared.lock().unwrap().snapshot().status).unwrap();
    assert!(!wire.contains("U0tJTEwgYnl0ZXM="), "{wire}");

    // The screen cannot hand a device an item of its own making.
    assert!(dispatch(
        &shared,
        "kit_store",
        serde_json::json!({
            "device_id": DEVICE,
            "request_id": "r1",
            "changes": [serde_json::to_value(&received).unwrap()],
        }),
    ));
    assert_eq!(answer(&shared).unwrap().state, AnswerState::Refused);
    settle(&shared);
    assert_eq!(store_calls(&helper).len(), 1);
}

/// A key the operator typed reaches the device's store over the device link
/// and nothing else: not the snapshot every screen and the phone read, not
/// the request's answer and not a diagnostic, which `hide issue` quotes
/// (PRD skill-mcp-manager B13, D-11).
#[test]
fn a_typed_key_reaches_the_devices_store_and_never_the_snapshot_or_the_log() {
    use hide_agent_hooks::mcp::{Reach, Server};
    const KEY: &str = "ghp_core-s3cr3t-value";
    let change = StoreChange::AddMcp {
        server: Server {
            name: "github".to_owned(),
            reach: Reach::Local {
                command: "npx".to_owned(),
                args: vec!["-y".to_owned(), "github-mcp".to_owned()],
                env: vec!["GITHUB_TOKEN".to_owned()],
            },
        },
        keys: [("GITHUB_TOKEN".to_owned(), hide_kit::store::Sealed::new(KEY))]
            .into_iter()
            .collect(),
        from: None,
    };
    let helper = KitDevice::answering(Ok(report(&[])));
    // The device's store answers with the change as an outcome holds it.
    helper.answer_store(Ok(serde_json::to_value(StoreReport {
        outcomes: vec![ChangeOutcome {
            change: change.redacted(),
            refused: None,
            made: None,
        }],
        ..StoreReport::default()
    })
    .unwrap()));
    let shared = with_consent(Some(helper.clone()));
    let request = |id: &str, device: &str| {
        serde_json::json!({
            "device_id": device,
            "request_id": id,
            "changes": [serde_json::to_value(&change).unwrap()],
        })
    };

    let ((), records) = crate::diagnostics::capture(|| {
        assert!(dispatch(&shared, "kit_store", request("k1", DEVICE)));
        settle(&shared);
        // A request refused before it is sent says why, and no more.
        assert!(dispatch(&shared, "kit_store", request("k2", "nowhere")));
    });

    assert_eq!(
        store_calls(&helper),
        vec![KitAction::Store {
            changes: vec![change.clone()]
        }],
        "the device link carries the key to the device's store"
    );
    let mut runtime = shared.lock().unwrap();
    let wire = String::from_utf8(
        crate::runtime::serialize_snapshot_delta(&runtime.snapshot_delta_payload(u64::MAX))
            .unwrap(),
    )
    .unwrap();
    assert!(wire.contains("github"), "the snapshot carries the request");
    assert!(!wire.contains(KEY), "{wire}");
    let answered = runtime
        .snapshot
        .status
        .store_answers
        .last()
        .cloned()
        .unwrap();
    assert!(!format!("{answered:?}").contains(KEY));
    assert!(!format!("{change:?}").contains(KEY));
    assert!(!records.is_empty());
    for record in &records {
        assert!(!record.to_string().contains(KEY), "{record}");
    }
}

#[test]
fn a_copy_export_refusal_reaches_the_screen_with_its_cause_before_any_receive() {
    let helper = KitDevice::answering(Ok(report(&[])));
    let shared = with_consent(Some(helper.clone()));
    assert!(dispatch(
        &shared,
        "kit_store_copy",
        serde_json::json!({"device_id": DEVICE, "request_id": "too-large"})
    ));
    assert_eq!(
        shared.lock().unwrap().take_store_export().as_deref(),
        Some(DEVICE)
    );
    let refusal = hide_kit::store::StoreRefusal {
        cause: hide_kit::store::StoreRefusalCause::TooLarge,
        reason: "the store copy is past the 30 MiB transfer limit".to_owned(),
    };
    assert!(shared.lock().unwrap().ingest_store_export(
        DEVICE,
        Ok(StoreExport {
            items: Vec::new(),
            refused: Some(refusal.clone())
        })
    ));
    let answered = answer(&shared).unwrap();
    assert_eq!(answered.state, AnswerState::Refused);
    assert_eq!(answered.refused, Some(refusal));
    assert!(answered.outcomes.is_empty());
    settle(&shared);
    assert!(store_calls(&helper).is_empty());
    // Retry is a new request and no longer carries the previous refusal.
    assert!(dispatch(
        &shared,
        "kit_store_copy",
        serde_json::json!({"device_id": DEVICE, "request_id": "retry"})
    ));
    assert_eq!(answer(&shared).unwrap().state, AnswerState::Pending);
    assert!(answer(&shared).unwrap().refused.is_none());
}
