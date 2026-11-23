//! Who may send each kind of client event (PRD D-53, D-67).
//!
//! Every event kind a screen can send is in [`CLASSES`] with its class, and
//! a connection is a [`Client`] of one kind. An event goes on only when its
//! class admits the connection's kind; any other, and a kind the table does
//! not hold, is answered on that connection alone with a [`refusal`] frame
//! the page ignores, and logged. A refused event never reaches the core, so
//! it never lands in the `status.last_error` every screen shows.
//!
//! The table is the one judgment: `server.rs` routes an admitted event to
//! its handler and no longer asks who sent it. The test at the bottom fails
//! when the core decodes a kind the table does not hold.

use axum::extract::ws::Message;
use serde_json::json;

/// What kind of connection sent an event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Client {
    /// A window: the desktop app's or a browser tab of this machine, or a
    /// linked node's window the old node role relays here (release B
    /// removes that path, PRD D-42).
    Window { desktop: bool },
    /// Another machine's view of this core, over a peer connection's screen
    /// stream (D-53).
    Remote,
    /// A client with this machine's token that draws no screen.
    Tool,
}

/// Which connections may send a kind.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Class {
    /// Any screen of this core, another machine's view of it included.
    Screen,
    /// A window a person looks at, here or on another machine.
    Window,
    /// A window of this machine only: approving what reaches this machine
    /// (pairing, external tools, phones), its focus and its own counts.
    OwnWindow,
    /// This machine's desktop window only: its notifications.
    Desktop,
    /// A client of this machine, never another machine's view of it: what
    /// changes this machine's devices or reaches past Hide (an outside app,
    /// Finder).
    Local,
    /// Sent by the daemon itself; never a client's.
    Daemon,
}

impl Class {
    pub(crate) fn admits(self, client: Client) -> bool {
        match self {
            Self::Screen => true,
            Self::Window => matches!(client, Client::Window { .. } | Client::Remote),
            Self::OwnWindow => matches!(client, Client::Window { .. }),
            Self::Desktop => client == Client::Window { desktop: true },
            Self::Local => matches!(client, Client::Window { .. } | Client::Tool),
            Self::Daemon => false,
        }
    }
}

/// Families a handler reads by their prefix; a kind in [`CLASSES`] wins
/// over its family.
const FAMILIES: &[(&str, Class)] = &[
    // The pairing of machines: share, allow, pause, remove and the request
    // the other way (D-53, B52).
    ("pairing_", Class::OwnWindow),
    // The phone's pairing. What another machine's screen may still do (remove
    // a phone, retry the relay, set when phones are notified, watch the tab)
    // is in the table, which `mobile_event_reaches` reads for a linked
    // node's screen too (D-53, D-70).
    ("mobile_", Class::OwnWindow),
];

/// The class of `kind`, or `None` for a kind no one classified.
pub(crate) fn class_of(kind: &str) -> Option<Class> {
    CLASSES
        .binary_search_by(|(listed, _)| listed.cmp(&kind))
        .ok()
        .map(|at| CLASSES[at].1)
        .or_else(|| {
            FAMILIES
                .iter()
                .find(|(prefix, _)| kind.starts_with(prefix))
                .map(|(_, class)| *class)
        })
}

/// Why an event was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Refused {
    /// No class names the kind.
    Unclassified,
    /// The kind's class does not admit this connection.
    NotHere,
}

impl Refused {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Unclassified => "unclassified",
            Self::NotHere => "not_here",
        }
    }
}

/// Whether `client` may send `kind`.
pub(crate) fn admit(kind: &str, client: Client) -> Result<(), Refused> {
    match class_of(kind) {
        None => Err(Refused::Unclassified),
        Some(class) if class.admits(client) => Ok(()),
        Some(_) => Err(Refused::NotHere),
    }
}

/// The answer to a refused event: for that connection alone, and ignored by
/// the page, which never sends a kind it is not shown (B52).
pub(crate) fn refusal(kind: &str, reason: Refused) -> Message {
    Message::Text(
        json!({"type": "refused", "payload": {"kind": kind, "reason": reason.name()}})
            .to_string()
            .into(),
    )
}

/// Logs a refusal with the kind and the connection, never the payload.
pub(crate) fn log_refusal(kind: &str, reason: Refused, client: Client, connection: u64) {
    hide_core::diagnostic!(json!({
        "component": "hided", "kind": "event.refused",
        "event": kind.chars().take(64).collect::<String>(), "reason": reason.name(),
        "client": format!("{client:?}"), "connection": connection,
    }));
}

use Class::{Daemon, Desktop, Local, OwnWindow, Screen, Window};

/// Every kind a screen sends, sorted, with its class: the core's kinds and
/// the ones hided answers itself.
pub(crate) const CLASSES: &[(&str, Class)] = &[
    ("access_approve", OwnWindow),
    ("access_call", OwnWindow),
    ("access_invite_cancel", OwnWindow),
    ("access_mode", OwnWindow),
    ("access_refuse", OwnWindow),
    ("access_revoke", OwnWindow),
    ("add_project_folder_set", Screen),
    ("agent_definitions_read", Screen),
    ("agent_group", Screen),
    ("agent_layout", Screen),
    ("agent_logo_mode_set", Screen),
    ("agent_onboarding_apply", Screen),
    ("agent_permission_set", Screen),
    ("agent_profile_change", Screen),
    ("agent_sleep", Screen),
    ("agent_sleep_set", Screen),
    ("agent_start_in_checkout", Screen),
    ("agent_tree_toggle", Screen),
    ("agent_wake", Screen),
    ("ai_settings", Screen),
    ("archive_check", Screen),
    ("archive_delete", Screen),
    ("archive_open", Screen),
    ("archive_resume", Screen),
    ("archive_session", Screen),
    ("attachment_cancel", Screen),
    ("attachment_commit", Screen),
    ("attachment_stage", Screen),
    ("automation_add", Screen),
    ("automation_call", Screen),
    ("automation_edit", Screen),
    ("automation_pause", Screen),
    ("automation_remove", Screen),
    ("automation_resume", Screen),
    ("automation_run", Screen),
    ("automation_run_open", Screen),
    ("automation_run_stop", Screen),
    ("automation_run_viewing", Screen),
    ("automation_seen", Screen),
    ("backlog_add", Screen),
    ("backlog_dismiss", Screen),
    ("backlog_origin", Screen),
    ("backlog_origin_close", Screen),
    ("backlog_retry", Screen),
    ("backlog_undo", Screen),
    ("browser_comment", OwnWindow),
    ("browser_open", Screen),
    ("browser_state", Screen),
    ("cancel_repository_clone", Screen),
    ("card_refresh", Screen),
    ("changes_select", Screen),
    ("check_close_status", Screen),
    ("check_sleeping_session", Screen),
    ("checkout_agents_toggle", Screen),
    ("checkout_catch_up", Screen),
    ("cleanup_confirm", Screen),
    ("cleanup_dismiss", Screen),
    ("cleanup_review", Screen),
    ("click", Screen),
    ("clone_repository", Screen),
    ("clone_target", Screen),
    ("close_pane", Screen),
    ("close_tab", Screen),
    ("close_tree", Screen),
    ("close_workspace", Screen),
    ("codex_daemon_disable", Screen),
    ("color_theme_set", Screen),
    ("conductor_settings_set", Screen),
    ("create_pane", Screen),
    ("create_tab", Screen),
    ("create_workspace", Screen),
    ("create_worktree", Screen),
    ("deliver_letters_now", Screen),
    ("desk_approval", Screen),
    ("desk_install", Screen),
    ("desk_job_cancel", Screen),
    ("desk_job_read", Screen),
    ("desk_job_requeue", Screen),
    ("desk_lane", Screen),
    ("desk_new_conversation", Screen),
    ("desk_output_seen", Screen),
    ("desk_restart", Screen),
    ("desk_select", Screen),
    ("desk_send", Screen),
    ("desk_settings_set", Screen),
    ("desk_stop", Screen),
    ("desktop_activity", Desktop),
    ("desktop_notices", Desktop),
    ("desktop_notifications_done_set", OwnWindow),
    ("desktop_notifications_set", OwnWindow),
    ("detail_open", Screen),
    ("device_host_consent", Local),
    ("device_host_retry", Local),
    ("device_sign_in", Local),
    ("device_switch", Daemon),
    ("dir_create", Screen),
    ("disk_watch", Screen),
    ("dismiss_agent_close", Screen),
    ("editor_text_scale", Screen),
    ("external_grants", Local),
    ("external_open", Local),
    ("fetch_look", Screen),
    ("file_bytes", Screen),
    ("file_close", Screen),
    ("file_conflict", Screen),
    ("file_create", Screen),
    ("file_draft", Screen),
    ("file_focus", Screen),
    ("file_index", Screen),
    ("file_keep_open", Screen),
    ("file_list", Screen),
    ("file_open", Screen),
    ("file_save", Screen),
    ("file_view", Screen),
    ("first_run_advance", Screen),
    ("first_run_back", Screen),
    ("first_run_draft", Screen),
    ("first_run_sign_in_withdraw", Screen),
    ("focus_checkout", Screen),
    ("focus_device", Screen),
    ("focus_pane", Screen),
    ("focus_tab", Screen),
    ("fork_pane", Screen),
    ("git_worktree_open", Screen),
    ("git_worktree_set_base", Screen),
    ("github_request", Screen),
    ("github_search", Screen),
    // A guest's pane here is ended by this machine's operator (D-74).
    ("guest_pane_close", OwnWindow),
    ("guest_panes", Daemon),
    ("herdr_import", Screen),
    ("herdr_profiles_list", Screen),
    ("hide_ai_switch", Screen),
    ("hide_ai_wake", Daemon),
    ("home_folder_list", Screen),
    ("human_turn_done", Screen),
    ("importable_projects", Screen),
    ("inactive_checkouts_toggle", Screen),
    ("inactive_projects_toggle", Screen),
    ("interface_language_set", Screen),
    ("issue_detail_request", Screen),
    ("issue_set_open", Screen),
    ("issue_settings_set", Screen),
    ("issue_source_set", Screen),
    ("key", Screen),
    ("kit_agent_set", Screen),
    ("kit_check", Screen),
    ("kit_reinstall", Screen),
    ("kit_store", Screen),
    ("kit_store_copy", Local),
    ("links_close", Screen),
    ("links_open", Screen),
    ("local_issue_update", Screen),
    ("machine_profile_connect", Local),
    ("machine_profiles_list", Local),
    ("memory_action", Screen),
    ("memory_evictions_seen", Screen),
    ("memory_expand", Screen),
    ("memory_open", Screen),
    ("memory_search", Screen),
    ("migrate_main_branch", Screen),
    ("mission_answer", Screen),
    ("mission_attention", Screen),
    ("mission_brief_read", Screen),
    ("mission_end", Screen),
    ("mission_handover", Screen),
    ("mission_material", Screen),
    ("mission_put_away", Screen),
    ("mission_reopen", Screen),
    ("mission_shown", Screen),
    ("mission_view_reopen", Screen),
    ("mobile_observe", Window),
    ("mobile_push_mode", Window),
    ("mobile_retry", Window),
    ("mobile_revoke", Window),
    ("open_external", Local),
    ("overview_graph_change", Screen),
    ("overview_open_result", Screen),
    ("overview_open_section", Screen),
    ("overview_refresh", Screen),
    ("pair_view", Screen),
    ("pair_view_desk", Screen),
    ("pane_find", Screen),
    ("pane_find_open", Screen),
    ("pane_input_sent", Daemon),
    ("pane_input_submitted", Daemon),
    ("pane_move", Screen),
    ("pane_move_check", Screen),
    ("pane_move_preview", Screen),
    ("pane_move_preview_cancel", Screen),
    ("pane_reopen", Screen),
    ("pane_text_scale", Screen),
    ("pane_turn_row_mode_set", Screen),
    ("pane_visit", Screen),
    ("path_copy", Screen),
    ("path_import", Screen),
    ("path_move", Screen),
    ("path_rename", Screen),
    ("path_trash", Screen),
    ("peer_machines", Daemon),
    // What another machine told of its own work (D-50).
    ("peer_work", Daemon),
    ("phone_viewing", Daemon),
    ("pr_card_read", Screen),
    ("pr_delegate", Screen),
    ("pr_feedback_read", Screen),
    ("pr_link_issue", Screen),
    ("pr_merge", Screen),
    ("pr_open", Screen),
    ("pr_update_branch", Screen),
    ("practice_invite_close", Screen),
    ("practice_progress_set", Screen),
    ("product_analytics_connected", OwnWindow),
    ("product_analytics_set", Screen),
    ("project_checkouts_fold", Screen),
    ("project_memory_navigate", Screen),
    ("project_settings_set", Screen),
    ("project_target", Screen),
    ("reconnect_pane", Screen),
    ("refresh_status", Screen),
    ("register_device", Local),
    ("remote_control", Local),
    ("remote_file_list", Screen),
    ("remove_device", Local),
    ("rename_device", Local),
    ("remove_workspace", Screen),
    ("remove_worktree", Screen),
    ("rename_tab", Screen),
    ("reopen_closed", Screen),
    ("reorder_tab", Screen),
    ("request_view", Screen),
    ("resize_pane", Screen),
    ("resolve_session", Screen),
    ("retry_agent_close", Screen),
    ("retry_connect", Screen),
    ("reveal_path", Local),
    ("revisit_done", Screen),
    ("revisit_star", Screen),
    ("screen_intro_seen", Screen),
    ("session_fold_toggle", Screen),
    ("session_missed_status", Screen),
    ("session_record_close", Screen),
    ("session_record_open", Screen),
    ("session_snapshot", Screen),
    ("session_thread_close", Screen),
    ("session_thread_input", Screen),
    ("session_thread_older", Screen),
    ("session_thread_open", Screen),
    ("session_tree_toggle", Screen),
    ("session_work_search", Screen),
    ("sessions_refresh", Screen),
    ("sessions_set_filter", Screen),
    ("set_checkout_issue", Screen),
    ("set_checkout_purpose", Screen),
    ("set_primary_checkout", Screen),
    ("sidebar_lens_set", Screen),
    ("sidebar_project_set", Screen),
    ("ssh_hosts_list", Local),
    ("store_ask_answer", Screen),
    ("stray_dismiss", Screen),
    ("stray_hand_off", Screen),
    ("stray_restore", Screen),
    ("suggestion_action", Screen),
    ("suggestion_branch_file", Screen),
    ("suggestion_evidence", Screen),
    ("suggestion_settings", Screen),
    ("suggestions_intro_seen", Screen),
    ("task_agent_retry", Screen),
    ("task_operation_ack", Screen),
    ("task_reveal", Local),
    ("task_view_set", Screen),
    ("terminal_attachment", Screen),
    ("terminal_attachment_action", Screen),
    ("terminal_attachment_ready", Screen),
    ("terminal_mouse", Screen),
    ("terminal_resize", Screen),
    ("terminal_scroll", Screen),
    ("terminal_viewport", Screen),
    ("test_device", Local),
    ("theme_set", Screen),
    ("thread_input", Screen),
    ("toggle_conversation", Screen),
    ("toggle_zoom", Screen),
    ("turn_answer_read", Screen),
    ("ui_attached", Daemon),
    ("ui_state_update", Screen),
    ("usage_hidden_set", Screen),
    ("usage_pin_set", Screen),
    ("view_comment", Window),
    ("view_feedback_send", Window),
    ("view_layout", Screen),
    ("wake_sleeping_session", Screen),
    ("window_focus", OwnWindow),
    ("workspace_pin_set", Screen),
    ("workspace_view", Screen),
    ("worktree_name_suggest", Screen),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// The kinds the core decodes, read from its decoder: the arms of
    /// `validate_event`'s match and the kinds of the families it hands on.
    fn core_kinds() -> BTreeSet<String> {
        let read = |file: &str| {
            let path = format!(
                "{}/../hide-core/src/runtime/{file}",
                env!("CARGO_MANIFEST_DIR")
            );
            std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"))
        };
        let literals = |text: &str| -> Vec<String> {
            text.split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect()
        };
        let events = read("events.rs");
        let start = events
            .find("fn validate_event")
            .and_then(|at| {
                events[at..]
                    .find("match kind.as_str() {")
                    .map(|found| at + found)
            })
            .expect("validate_event matches on the kind");
        let end = start
            + events[start..]
                .find("_ => Err(EventValidationError")
                .expect("validate_event ends with the unknown kind");
        let mut kinds = BTreeSet::new();
        for line in events[start..end].lines() {
            let arm = line.trim_start();
            if arm.starts_with('"')
                && let Some((head, _)) = arm.split_once("=>")
            {
                kinds.extend(literals(head));
            }
        }
        for (file, list) in [
            ("automations.rs", "EVENT_KINDS: [&str;"),
            ("access.rs", "KINDS: [&'static str;"),
            ("desk.rs", "KINDS: [&'static str;"),
        ] {
            let text = read(file);
            let at = text
                .find(list)
                .unwrap_or_else(|| panic!("{file} lists its kinds"));
            let body = &text[at..];
            let body = &body[body.find("= [").unwrap()..body.find("];").unwrap()];
            kinds.extend(literals(body));
        }
        assert!(kinds.len() > 200, "the decoder was not read: {kinds:?}");
        kinds
    }

    /// The kinds the daemon answers before the core sees them: each one an
    /// arm of `handle_client_text` names (`Some("…")`), its device file
    /// events and the automation screen's kind.
    fn daemon_kinds() -> BTreeSet<String> {
        let path = format!("{}/src/server.rs", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        let start = text
            .find("fn handle_client_text(")
            .expect("the daemon reads a screen's events in handle_client_text");
        let end = start
            + text[start..]
                .find("\n}\n")
                .expect("handle_client_text ends");
        let mut kinds = BTreeSet::new();
        for arm in text[start..end].split("Some(").skip(1) {
            let Some(close) = arm.find(')') else {
                continue;
            };
            kinds.extend(
                arm[..close]
                    .split('"')
                    .skip(1)
                    .step_by(2)
                    .map(str::to_owned),
            );
        }
        kinds.extend(
            crate::server::DEVICE_FILE_EVENTS
                .iter()
                .map(|kind| (*kind).to_owned()),
        );
        kinds.insert(crate::automation_screen::KIND.to_owned());
        assert!(
            kinds.len() > 20,
            "handle_client_text was not read: {kinds:?}"
        );
        kinds
    }

    #[test]
    fn every_kind_the_core_or_the_daemon_reads_has_a_class() {
        let unclassified: Vec<_> = core_kinds()
            .into_iter()
            .chain(daemon_kinds())
            .filter(|kind| class_of(kind).is_none())
            .collect();
        assert!(
            unclassified.is_empty(),
            "add each to CLASSES with the class that may send it (PRD D-67): {unclassified:?}"
        );
    }

    #[test]
    fn the_table_is_sorted_and_names_each_kind_once() {
        for pair in CLASSES.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{} then {}", pair[0].0, pair[1].0);
        }
    }

    /// Another machine's view of this one shows the work and refuses what
    /// lets a machine or a tool in, what moves past Hide, and what counts as
    /// someone in front of this machine (D-53, D-67, B52).
    #[test]
    fn a_remote_screen_works_but_lets_nothing_in() {
        for kind in [
            "key",
            "terminal_viewport",
            "rename_tab",
            "file_bytes",
            "ui_state_update",
        ] {
            assert_eq!(admit(kind, Client::Remote), Ok(()), "{kind}");
        }
        for kind in [
            "pairing_share",
            "pairing_answer",
            "pairing_pause",
            "pairing_remove",
            "pairing_request",
            "access_approve",
            "access_call",
            "access_invite_cancel",
            "mobile_show_code",
            "mobile_enable",
            "register_device",
            "remove_device",
            "rename_device",
            "pairing_rename",
            "remote_control",
            "open_external",
            "reveal_path",
            "window_focus",
            "product_analytics_connected",
            "desktop_activity",
        ] {
            assert_eq!(admit(kind, Client::Remote), Err(Refused::NotHere), "{kind}");
            assert_eq!(
                admit(kind, Client::Window { desktop: true }),
                Ok(()),
                "{kind}"
            );
        }
        // A phone is removed from any window (D-70).
        assert_eq!(admit("mobile_revoke", Client::Remote), Ok(()));
        assert_eq!(admit("mobile_retry", Client::Remote), Ok(()));
        assert_eq!(admit("mobile_push_mode", Client::Remote), Ok(()));
        assert_eq!(
            admit("s3_unclassified_probe", Client::Remote),
            Err(Refused::Unclassified)
        );
        assert_eq!(
            admit("s3_unclassified_probe", Client::Window { desktop: true }),
            Err(Refused::Unclassified)
        );
    }

    #[test]
    fn what_the_daemon_sends_no_client_may() {
        for client in [
            Client::Window { desktop: true },
            Client::Remote,
            Client::Tool,
        ] {
            for kind in [
                "ui_attached",
                "peer_machines",
                "device_switch",
                "guest_panes",
                "peer_work",
            ] {
                assert_eq!(admit(kind, client), Err(Refused::NotHere), "{kind}");
            }
        }
        assert_eq!(
            admit("desktop_notices", Client::Window { desktop: false }),
            Err(Refused::NotHere)
        );
        assert_eq!(admit("window_focus", Client::Tool), Err(Refused::NotHere));
    }
}
