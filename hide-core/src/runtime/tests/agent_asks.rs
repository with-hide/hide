//! An agent's own open questions from its pane (docs/status-model.md, A
//! question still waiting): the fold and the prompt's read name the caller's
//! own session and no other, a question the operator opened again stays
//! theirs, and a receipt the session's record brings back marks a question
//! given.

use super::*;
use crate::handle::ChangeNotifier;
use crate::labels::worker::SightedMemoryReceipt;
use crate::links::ledger::{LedgerUpdate, MissedStatus, SessionIdentity};
use crate::runtime::delivery::tests::{authority, fixture};

fn codex(session: &str) -> SessionIdentity {
    SessionIdentity {
        device_id: crate::node::TEST_NODE.into(),
        agent: "codex".into(),
        session_id: session.into(),
    }
}

/// A question the operator was asked in one of the fixture's Codex
/// sessions, as a judged turn end leaves it.
fn asked(store: &std::path::Path, session: &str, id: &str, operator_only: bool) {
    rusqlite::Connection::open(store)
        .unwrap()
        .execute(
            "INSERT INTO ledger_missed(device,agent,id,missed,kind,text,status,at,turn_offset,operator_only) \
            VALUES(?1,'codex',?2,?3,'\"ask\"','App Store Connect에 로그인해 주세요','\"open\"',1000,1,?4)",
            rusqlite::params![crate::node::TEST_NODE, session, id, operator_only],
        )
        .unwrap();
}

fn row(store: &std::path::Path, session: &str, id: &str) -> (String, Option<String>, bool) {
    rusqlite::Connection::open(store)
        .unwrap()
        .query_row(
            "SELECT status,reason,given_at IS NOT NULL FROM ledger_missed WHERE id=?1 AND missed=?2",
            rusqlite::params![session, id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
}

#[test]
fn an_agent_folds_only_its_own_sessions_question_and_its_prompt_reads_only_its_own() {
    let root = tempfile::tempdir().unwrap();
    let (runtime, sender, _, _) = fixture(root.path());
    runtime
        .lock()
        .unwrap()
        .install_worker_context(Arc::downgrade(&runtime), ChangeNotifier::noop());
    let worker = super::super::links::spawn_worker(&runtime, ChangeNotifier::noop()).unwrap();
    let node = crate::node::TEST_NODE;
    let pane = sender.pane_id.clone();
    let context = authority(&sender).context;

    // With Hide AI's summaries off nothing keeps the list, so a prompt
    // reads none; on, only the pane's own session is read.
    assert!(
        runtime
            .lock()
            .unwrap()
            .asks_give_scope(node, &pane, "sender-session")
            .is_none()
    );
    runtime.lock().unwrap().ai_settings = Some(hide_ai::settings::AiSettings {
        enabled: true,
        chosen: true,
        agent_summary: true,
        ..Default::default()
    });
    assert!(
        runtime
            .lock()
            .unwrap()
            .asks_give_scope(node, &pane, "recipient-session")
            .is_none()
    );
    let give = runtime
        .lock()
        .unwrap()
        .asks_give_scope(node, &pane, "sender-session")
        .unwrap();
    assert_eq!(give.session, codex("sender-session"));
    let store = give.store.clone();
    // The worker opens the record, and makes its tables, as it starts.
    wait_for("the link record", || {
        rusqlite::Connection::open(&store).is_ok_and(|connection| {
            connection
                .query_row("SELECT count(*) FROM ledger_missed", [], |_| Ok(()))
                .is_ok()
        })
    });
    asked(&store, "sender-session", "missed-1-0", true);
    asked(&store, "recipient-session", "missed-2-0", false);

    let given = give.context().unwrap();
    assert!(given.contains("- missed-1-0 "), "{given}");
    assert!(given.contains("hide session fold-ask missed-1-0 --reason"));
    assert!(!given.contains("missed-2-0"));

    // The receipt the session's record brings back marks it given; Memory's
    // receipts stay for Memory.
    let receipt = |text: &str| SightedMemoryReceipt {
        context_reset: false,
        provider: "codex",
        session_id: "sender-session".into(),
        cwd: "/checkouts/fixture".into(),
        offset: 3,
        text: text.into(),
    };
    let mut receipts = vec![
        receipt("<hide-asks-receipt items=\"missed-1-0\" />"),
        receipt("<hide-memory-receipt event=\"UserPromptSubmit\" />"),
    ];
    runtime
        .lock()
        .unwrap()
        .take_asks_receipts(node, &mut receipts);
    assert_eq!(receipts.len(), 1);
    assert!(receipts[0].text.starts_with("<hide-memory-receipt"));
    wait_for("the question given", || {
        row(&store, "sender-session", "missed-1-0").2
    });
    assert_eq!(give.context(), None);

    // The fold names the caller's session: another session's question is
    // no question of its, while its own operator-only one folds.
    let fold = |id: &str, reason: &str| {
        let scope = runtime
            .lock()
            .unwrap()
            .fold_ask_scope(node, &pane, &context)
            .unwrap();
        assert_eq!(scope.session, codex("sender-session"));
        scope.run(id, reason)
    };
    assert_eq!(
        fold("missed-2-0", "운영자가 대신 머지함"),
        Err("ask_unknown".to_owned())
    );
    assert_eq!(row(&store, "recipient-session", "missed-2-0").0, "\"open\"");
    assert_eq!(
        fold("missed-1-0", " "),
        Err("ask_reason_invalid".to_owned())
    );
    assert_eq!(
        fold("missed-1-0", "운영자가 로그인했다고 리드가 알림").unwrap()["state"],
        "folded"
    );
    assert_eq!(
        row(&store, "sender-session", "missed-1-0"),
        (
            "\"folded\"".to_owned(),
            Some("운영자가 로그인했다고 리드가 알림".to_owned()),
            true
        )
    );
    assert_eq!(
        fold("missed-1-0", "다시").unwrap()["state"],
        "already_closed"
    );

    // Opened again by the operator, it is theirs, and the next prompt says so.
    worker
        .client()
        .ledger(LedgerUpdate::MissedStatus {
            session: codex("sender-session"),
            id: "missed-1-0".into(),
            status: MissedStatus::Open,
        })
        .unwrap();
    wait_for("the question opened again", || {
        row(&store, "sender-session", "missed-1-0").0 == "\"open\""
    });
    assert_eq!(
        fold("missed-1-0", "또 접음"),
        Err("ask_reopened".to_owned())
    );
    let given = give.context().unwrap();
    assert!(given.contains(
        "missed-1-0 \"App Store Connect에 로그인해 주세요\" (the operator opened this again"
    ));
    assert!(!given.contains("fold-ask missed-1-0"));
}
