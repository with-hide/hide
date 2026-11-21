//! A spawn whose brief rides its agent's start (`AgentApi::LaunchPrompt`),
//! with-hide/hide#2482: the agent takes the brief as it starts and works on
//! it at once, while the engine reads its start as launching until that turn
//! ends. The child is registered as soon as it is seen working, so the letter
//! it writes to its parent in that turn is taken, not refused.
//!
//! The spawn runs on this machine's fake engine
//! (`beside_spawn::local_spawner`), which reads the started agent in the
//! order the test sets.

use super::beside_spawn::{Ask, BESIDE, child_row, local_spawner};
use super::remote_spawn::AnswerWait;
use super::*;
use crate::delivery::Command;
use crate::runtime::delivery::tests::authority;
use serde_json::{Value, json};

/// The started agent as the engine reads it while its start has not
/// settled (`launch_pending`, `Readiness::Launching`), in `status`.
fn launching(status: &str) -> Value {
    let mut row = child_row();
    row["launch_pending"] = json!(true);
    row["agent_status"] = json!(status);
    row
}

/// The agent shows idle as it starts, then works on its brief, and its start
/// never settles. The spawn answers its child registered with its parent
/// inside an answer time shorter than the wait for input readiness, which a
/// spawn that waited for readiness would spend and leave the child
/// unregistered; and the child's letter to `parent` while the engine still
/// reads it launching and working reaches the parent.
#[test]
fn a_child_seen_working_on_its_launched_brief_is_registered_and_its_letter_to_its_parent_taken() {
    let spawner = local_spawner();
    spawner.reads([launching("idle"), launching("working")]);
    let _answer = AnswerWait::set(crate::coordination::BRIEF_READY_WAIT / 2);
    let child = spawner
        .spawn(
            "launched",
            Ask {
                parent: Some("here"),
                beside: false,
                brief: Some("As your very first action, write to your parent."),
                ..BESIDE
            },
        )
        .expect("the child is registered");
    assert_eq!(child["pane"], "w2:p1");
    assert_eq!(child["parent"], "agent-1");
    let ledger = spawner.ledger();
    let route = ledger.spawns[0]
        .brief_input
        .as_ref()
        .map(|input| input.route);
    assert_eq!(
        route,
        Some(crate::delivery::typed::SendRoute::Launch),
        "the brief rode the start"
    );
    let bound = ledger
        .agents
        .iter()
        .find(|record| child["id"] == record.id.as_str())
        .expect("the child's registration")
        .actor
        .clone();

    // The child's pane, in the caller's checkout, as the engine lists it
    // under the spawn's name.
    {
        let mut runtime = spawner.runtime.lock().unwrap();
        runtime.install_delivery_client(spawner.client.clone());
        let tab = &mut runtime.snapshot.navigator.workspaces[0].checkouts[0].tabs[0];
        let mut pane = tab.panes[0].clone();
        pane.id = "w2:p1".into();
        tab.panes.push(pane);
        let payload: crate::sidebar::SessionSnapshotPayload =
            serde_json::from_value(json!({"agents": [
                {"id": "sender", "pane_id": "sender", "agent": "codex", "agent_status": "working",
                 "state_change_seq": 1, "lineage_session": spawner.actor.session},
                {"id": "worker", "pane_id": "w2:p1", "agent": "codex", "agent_status": "working",
                 "state_change_seq": 1, "lineage_session": bound.session},
            ]}))
            .unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
    }
    let prepared = spawner
        .runtime
        .lock()
        .unwrap()
        .prepare_delivery(
            crate::node::TEST_NODE,
            "w2:p1",
            &authority(&spawner.actor).context,
            None,
            Command::Send {
                target: "parent".into(),
                intent: "first".into(),
                body: "first".into(),
                kind: "progress".into(),
                now: false,
                force: false,
            },
        )
        .expect("the child's letter is admitted");
    let letter = prepared
        .run(Duration::from_secs(10))
        .expect("the letter is recorded");
    assert_eq!(letter["recipient"]["pane_id"], "sender");
    assert_eq!(letter["sender"]["pane_id"], "w2:p1");
}
