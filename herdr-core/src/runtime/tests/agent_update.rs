//! An agent whose copy is older than Hide's minimum: its pane says to update
//! it, the session that wore that badge reads `started_before_hide` once the
//! agent is updated, and the read after the update installs what was
//! refused (docs/agent-hooks.md, Updating an agent that is too old;
//! docs/status-model.md, Not connected, and what fixes it).

use super::agent_connection::{connection_of, diagnosis, feed, installed, kit_rows};
use super::first_run::{agent, pass};
use super::*;
use crate::model::PaneConnectionReason;
use hide_agent_hooks::HookStatus;
use hide_kit::ComponentState;

fn update(command: &str) -> hide_kit::AgentUpdate {
    hide_kit::AgentUpdate {
        installed: "1.0.0".to_owned(),
        minimum: "2.0.0".to_owned(),
        path: "/fixture/bin/agent".to_owned(),
        command: Some(command.to_owned()),
    }
}

/// This Mac's kit row for `id` changed by `change`.
fn set_row(
    runtime: &mut Runtime,
    id: &str,
    change: impl FnOnce(&mut crate::model::KitAgentSnapshot),
) {
    let mut kit = runtime.kit_state(crate::node::TEST_NODE);
    change(kit.agents.iter_mut().find(|row| row.id == id).unwrap());
    runtime.set_kit_state(crate::node::TEST_NODE, kit);
}

fn reason(runtime: &Runtime, pane: &str) -> Option<PaneConnectionReason> {
    connection_of(runtime, pane).and_then(|connection| connection.reason)
}

#[test]
fn a_pane_of_an_agent_too_old_says_to_update_it_whatever_the_agent() {
    let mut runtime = runtime();
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(HookStatus::NotInstalled, installed()));
    set_row(&mut runtime, "claude-code", |row| {
        row.update = Some(update("claude update"))
    });
    set_row(&mut runtime, "opencode", |row| {
        row.update = Some(update("opencode upgrade"))
    });
    feed(
        &mut runtime,
        &[
            ("w1:p1", "claude", false),
            ("w1:p2", "opencode", false),
            ("w1:p3", "codex", true),
        ],
    );

    let claude = connection_of(&runtime, "w1:p1").unwrap();
    assert_eq!(claude.reason, Some(PaneConnectionReason::UpdateAgent));
    assert!(
        !claude.can_reopen,
        "a restart of the old copy connects nothing"
    );
    assert_eq!(claude.update, Some(update("claude update")));
    assert_eq!(
        connection_of(&runtime, "w1:p2").and_then(|connection| connection.update),
        Some(update("opencode upgrade"))
    );
    assert_eq!(
        connection_of(&runtime, "w1:p3").map(|connection| connection.connected),
        Some(true)
    );

    // An agent switched off is the operator's choice; its pane says nothing.
    set_row(&mut runtime, "opencode", |row| row.enabled = false);
    assert_eq!(connection_of(&runtime, "w1:p2"), None);
}

#[test]
fn the_session_that_wore_the_badge_reads_restart_once_after_the_update() {
    let mut runtime = runtime();
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(installed(), installed()));
    set_row(&mut runtime, "opencode", |row| {
        row.update = Some(update("opencode upgrade"))
    });
    feed(
        &mut runtime,
        &[("w1:p1", "opencode", false), ("w1:p2", "opencode", false)],
    );
    assert_eq!(
        reason(&runtime, "w1:p1"),
        Some(PaneConnectionReason::UpdateAgent)
    );

    // The update lands and the next pass puts Hide's plugin in: the running
    // session predates it, and no other agent than Claude Code and Codex is
    // reopened by Hide, so it says to restart it.
    let plugin = crate::model::KitPieceSnapshot {
        state: ComponentState::Installed,
        why: None,
        location: None,
    };
    set_row(&mut runtime, "opencode", |row| {
        row.update = None;
        row.hook = Some(plugin);
    });
    let restart = connection_of(&runtime, "w1:p1").unwrap();
    assert_eq!(
        restart.reason,
        Some(PaneConnectionReason::StartedBeforeHide)
    );
    assert!(!restart.can_reopen);

    // Restarted, it connects, and the badge is forgotten with the session.
    feed(
        &mut runtime,
        &[("w1:p1", "opencode", true), ("w1:p2", "opencode", false)],
    );
    assert_eq!(
        connection_of(&runtime, "w1:p1").map(|connection| connection.connected),
        Some(true)
    );
    assert_eq!(
        reason(&runtime, "w1:p2"),
        Some(PaneConnectionReason::StartedBeforeHide)
    );
    // A session that never wore the badge keeps OpenCode's rule: no chip.
    feed(&mut runtime, &[("w1:p3", "opencode", false)]);
    assert_eq!(connection_of(&runtime, "w1:p3"), None);
}

#[test]
fn a_session_that_newly_needs_an_update_asks_for_one_kit_read() {
    let mut runtime = runtime();
    kit_rows(&mut runtime, None);
    runtime.ingest_hook_diagnosis(diagnosis(HookStatus::NotInstalled, installed()));
    set_row(&mut runtime, "claude-code", |row| {
        row.update = Some(update("claude update"))
    });
    runtime.local_kit_check_requested = false;
    feed(&mut runtime, &[("w1:p1", "claude", false)]);
    assert!(
        runtime.local_kit_check_requested,
        "a session wearing the badge for the first time"
    );

    runtime.local_kit_check_requested = false;
    feed(&mut runtime, &[("w1:p1", "claude", false)]);
    assert!(
        !runtime.local_kit_check_requested,
        "the same session asks once"
    );

    // The agent started again, perhaps after its update: one more read.
    feed(&mut runtime, &[("w1:p2", "claude", false)]);
    assert!(runtime.local_kit_check_requested);
}

#[test]
fn a_read_that_finds_the_agent_updated_queues_the_install_and_an_install_queues_nothing_more() {
    let mut runtime = runtime();
    let too_old = |state| {
        let mut row = agent("pi", true, ComponentState::Installed);
        row.hook = Some(hide_kit::PieceReport {
            state,
            reason: None,
            why: None,
            location: None,
        });
        row
    };
    let mut refused = too_old(ComponentState::Failed);
    refused.update = Some(update("pi update"));
    runtime.ingest_kit_report(crate::node::TEST_NODE, &pass(false, vec![refused]));
    runtime.local_kit_pending = None;

    // A read after the update: no update needed, the extension not in yet.
    runtime.ingest_kit_report(
        crate::node::TEST_NODE,
        &pass(false, vec![too_old(ComponentState::NotInstalled)]),
    );
    assert_eq!(
        runtime.local_kit_pending,
        Some(hide_kit::Scope::automatic())
    );
    assert!(runtime.kit_state(crate::node::TEST_NODE).busy);

    // The install's own report leaves nothing to repair.
    runtime.local_kit_pending = None;
    runtime.ingest_kit_report(
        crate::node::TEST_NODE,
        &pass(false, vec![too_old(ComponentState::Installed)]),
    );
    assert_eq!(runtime.local_kit_pending, None);
}
