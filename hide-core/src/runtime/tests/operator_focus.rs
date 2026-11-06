//! The number a page gives each operator focus, and what the snapshot says
//! about it.
//!
//! The page follows the snapshot's focused pane only once the snapshot
//! includes the last focus the page sent (`OperatorFocusAck`), so the core's
//! side of that is the record: every operator focus it receives is answered
//! with its number, whatever the core did with the focus, and the record
//! cannot grow without bound.

use super::*;

fn numbered_focus_event(pane_id: &str, origin: &str, client_id: &str, sequence: u64) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "focus_pane",
        "payload": {
            "pane_id": pane_id,
            "origin": origin,
            "client_id": client_id,
            "sequence": sequence
        }
    }))
    .expect("numbered focus event")
}

fn acked(runtime: &Runtime) -> Vec<(String, u64)> {
    runtime
        .snapshot
        .focused
        .operator_focus
        .iter()
        .map(|ack| (ack.client_id.clone(), ack.sequence))
        .collect()
}

#[test]
fn an_operator_focus_is_answered_with_its_number_whatever_the_core_did_with_it() {
    let (mut runtime, _checkout) = tab_order_runtime("/tmp/hide-operator-focus");
    // No live engine: the core refuses the focus (`pane.control_unavailable`),
    // and the page is still waiting for the answer.
    runtime.dispatch_json(&numbered_focus_event(
        "w-order:t1:p",
        "operator",
        "page-a",
        3,
    ));
    assert_eq!(
        runtime
            .snapshot
            .status
            .last_error
            .as_ref()
            .map(|error| error.kind.as_str()),
        Some("pane.control_unavailable")
    );
    assert_eq!(acked(&runtime), vec![("page-a".to_owned(), 3)]);
}

#[test]
fn a_page_keeps_the_newest_number_and_each_page_has_its_own() {
    let (mut runtime, _checkout) = tab_order_runtime("/tmp/hide-operator-focus");
    runtime.dispatch_json(&numbered_focus_event(
        "w-order:t1:p",
        "operator",
        "page-a",
        5,
    ));
    runtime.dispatch_json(&numbered_focus_event(
        "w-order:t1:p",
        "operator",
        "page-b",
        1,
    ));
    // A number below the recorded one never lowers it.
    runtime.dispatch_json(&numbered_focus_event(
        "w-order:t1:p",
        "operator",
        "page-a",
        2,
    ));
    assert_eq!(
        acked(&runtime),
        vec![("page-b".to_owned(), 1), ("page-a".to_owned(), 5)],
        "the page that spoke last is newest"
    );
}

#[test]
fn only_an_operator_focus_that_names_its_page_is_recorded() {
    let (mut runtime, _checkout) = tab_order_runtime("/tmp/hide-operator-focus");
    runtime.dispatch_json(&numbered_focus_event(
        "w-order:t1:p",
        "restore",
        "page-a",
        1,
    ));
    runtime.dispatch_json(&operator_focus_event("w-order:t1:p"));
    assert_eq!(acked(&runtime), Vec::new());
    assert_eq!(diagnostic_count(&runtime, "pane.focus.sequence_invalid"), 0);

    runtime.dispatch_json(
        &serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "focus_pane",
            "payload": {"pane_id": "w-order:t1:p", "origin": "operator", "sequence": 4}
        }))
        .expect("a number without a page"),
    );
    assert_eq!(acked(&runtime), Vec::new());
    assert_eq!(diagnostic_count(&runtime, "pane.focus.sequence_invalid"), 1);
}

#[test]
fn the_record_is_capped_and_an_eviction_is_logged() {
    let (mut runtime, _checkout) = tab_order_runtime("/tmp/hide-operator-focus");
    for page in 0..=OPERATOR_FOCUS_CLIENT_LIMIT {
        runtime.dispatch_json(&numbered_focus_event(
            "w-order:t1:p",
            "operator",
            &format!("page-{page}"),
            1,
        ));
    }
    let kept = acked(&runtime);
    assert_eq!(kept.len(), OPERATOR_FOCUS_CLIENT_LIMIT);
    assert_eq!(kept[0].0, "page-1", "the oldest page was dropped");
    assert_eq!(diagnostic_count(&runtime, "pane.focus.sequence_evicted"), 1);
}

#[test]
fn the_snapshot_carries_the_record_next_to_the_focused_pane() {
    let (mut runtime, _checkout) = tab_order_runtime("/tmp/hide-operator-focus");
    let before = runtime.snapshot_delta_payload(0);
    assert_eq!(
        serde_json::to_value(&runtime.snapshot.focused).expect("focused")["operator_focus"],
        serde_json::Value::Null,
        "a page that never numbered a focus sees no field"
    );

    runtime.dispatch_json(&numbered_focus_event(
        "w-order:t1:p",
        "operator",
        "page-a",
        1,
    ));
    let after = runtime.snapshot_delta_payload(before.revision);
    assert!(
        after.rest.is_some(),
        "an acknowledgement alone is a change the page has to receive"
    );
    assert_eq!(
        serde_json::to_value(&runtime.snapshot.focused).expect("focused")["operator_focus"],
        serde_json::json!([{"client_id": "page-a", "sequence": 1}])
    );
}
