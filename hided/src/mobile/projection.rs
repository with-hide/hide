//! What a phone sees of the agents (PRD D-03, D-08, D-17): every agent this
//! Mac lists and every connected SSH device's, grouped the way the desktop
//! sidebar groups them, each row keyed by device id and pane id. It is built
//! from the core's snapshot `rest` section and carries no files, no paths and
//! nothing else of the snapshot.

pub use herdr_core::agent_state::phone::{AgentKey, PhoneAgent, Projection, project};
use serde_json::Value;

/// Materialize only the phone/start projection's changed inputs. Immutable
/// typed sections arrive directly from the owner; transport bytes and the
/// unrelated editor, layouts and terminal state never enter this follower.
pub fn update_inputs(
    kept: &mut Value,
    rest: &herdr_core::RestSections,
    have: u64,
) -> Result<bool, serde_json::Error> {
    if have == 0 || !kept.is_object() {
        *kept = serde_json::json!({"navigator": {}, "status": {}, "ui_state": {}});
    }
    let mut changed = false;
    macro_rules! field {
        ($group:literal, $key:literal, $stamp:expr) => {
            if $stamp.revision > have {
                kept[$group][$key] = serde_json::to_value($stamp.value.as_ref())?;
                changed = true;
            }
        };
    }
    field!("navigator", "agents", rest.navigator.agents);
    field!("navigator", "workspaces", rest.navigator.workspaces);
    field!("navigator", "devices", rest.navigator.devices);
    field!(
        "navigator",
        "sleeping_sessions",
        rest.navigator.sleeping_sessions
    );
    field!("status", "remote", rest.status.remote);
    field!("status", "background_ai", rest.status.background_ai);
    field!("status", "last_error", rest.status.last_error);
    if rest.task_operation.revision > have {
        kept["task_operation"] = serde_json::to_value(rest.task_operation.value.as_ref())?;
        changed = true;
    }
    if rest.ui_state.revision > have {
        kept["ui_state"]["agent_start"] = serde_json::to_value(&rest.ui_state.agent_start)?;
        kept["ui_state"]["interface_language"] =
            serde_json::to_value(&rest.ui_state.interface_language)?;
        kept["ui_state"]["color_theme"] = serde_json::to_value(rest.ui_state.color_theme)?;
        changed = true;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use herdr_core::agent_state::phone::Line;
    use serde_json::json;

    #[test]
    fn typed_projection_retains_omissions_and_reset_removes_old_phone_state() {
        let options: herdr_core::CoreOptions = serde_json::from_value(json!({
            "schema_version": 2, "node_id": "test-node", "app_state_path": ""
        }))
        .unwrap();
        let mut snapshot = herdr_core::Snapshot::initial(&options);
        let first = herdr_core::RestSections::capture(&snapshot, None, 1);
        let mut fast = Value::Null;
        let mut slow = json!({"navigator": {"agents": [agent("old", "working", json!({}))]}});
        assert!(update_inputs(&mut fast, &first, 0).unwrap());
        assert!(update_inputs(&mut slow, &first, 0).unwrap());
        assert_eq!(fast, slow);
        assert_eq!(
            project(&fast, "local"),
            project(&serde_json::to_value(&first).unwrap(), "local")
        );
        snapshot.zoomed = Some("fixture:p1".to_owned());
        let selected = herdr_core::RestSections::capture(&snapshot, Some(&first), 2);
        let retained = fast.clone();
        assert!(!update_inputs(&mut fast, &selected, 1).unwrap());
        assert_eq!(fast, retained);
        assert!(update_inputs(&mut slow, &selected, 0).unwrap());
        assert_eq!(
            fast, slow,
            "a reader reset independently reconstructs the same phone inputs"
        );
        fast["status"]["remote"] = json!([{"target_id":"retired", "session":{"agents":[]}}]);
        assert!(update_inputs(&mut fast, &selected, 0).unwrap());
        assert_eq!(fast["status"]["remote"], json!([]));
    }

    fn agent(pane: &str, group: &str, extra: Value) -> Value {
        let mut value = json!({
            "pane_id": pane, "group": group, "mark": "going", "identity_label": format!("task {pane}"),
            "agent_kind": "claude", "changed_at_unix_ms": 1_790_000_000_000_u64, "status_code": "idle", "demand": "none",
        });
        for (key, field) in extra.as_object().unwrap() {
            value[key] = field.clone();
        }
        value
    }

    fn line(text: &str, mode: &str, tone: &str) -> Value {
        json!({"text": text, "mode": mode, "tone": {"kind": tone, "read": false}})
    }

    fn rest() -> Value {
        json!({
            "navigator": {
                "agents": [
                    agent("w1:p1", "working", json!({"activity": "working"})),
                    agent("w1:p2", "needs_you", json!({"demand": "approval", "mark": "turn", "state": {"line": line("Bash(cargo test) 실행을 허용할까요?", "request", "warning")}})),
                    agent("w1:p3", "working", json!({"lineage_parent_pane_id": "w1:p1", "delegated": true, "demand": "question"})),
                    agent("w1:p4", "seen", json!({"mark": "quiet", "state": {"line": line("quiet sentence", "quiet", "subtle")}})),
                ],
                "workspaces": [
                    {"label": "hide", "is_git": true, "checkouts": [
                        {"label": "main", "branch": "prd/mobile-companion", "tabs": [{"panes": [{"id": "w1:p1"}, {"id": "w1:p2"}]}]},
                    ]},
                    {"label": "notes", "is_git": false, "checkouts": [
                        {"label": "notes", "branch": null, "tabs": [{"panes": [{"id": "w1:p4"}]}]},
                    ]},
                ],
                "devices": [{"id": "local", "label": null}, {"id": "mini", "label": "mini"}],
            },
            "status": {"remote": [
                {"target_id": "mini", "state": "connected", "session": {
                    "agents": [agent("remote:mini:pane:w2:p1", "done", json!({"activity": "stopped", "mark": "done", "unread": true, "state": {"line": line("PR #186", "news", "news")}}))],
                    "workspaces": [{"label": "contong", "is_git": true, "checkouts": [{"label": "main", "branch": "main", "tabs": [{"panes": [{"id": "remote:mini:pane:w2:p1"}]}]}]}],
                }},
                {"target_id": "gone", "state": "unreachable", "session": {"agents": [agent("remote:gone:pane:w1:p1", "needs_you", json!({}))]}},
            ]},
        })
    }

    #[test]
    fn sleeping_conversations_have_no_pane_or_native_reference_on_the_phone() {
        let input = json!({"navigator": {"agents": [], "sleeping_sessions": [{
            "sleep_id": "sleep-a-1", "node_id": "local", "kind": "pi",
            "identity_label": "Review parser", "group": "seen", "phase": "sleeping",
            "since_unix_ms": 12, "wake_available": true, "checking": false, "reason": null,
            "checkout_path": "/private/never-send", "native_session_id": "native-secret",
            "old_pane_id": "retired-pane", "label_owner": "private-proof",
        }, {
            "sleep_id": "sleep-a-2", "node_id": "another-device", "kind": "pi",
            "identity_label": "Other device", "group": "seen", "phase": "sleeping",
            "since_unix_ms": 12, "wake_available": true, "checking": false, "reason": null,
        }]}});
        let projection = project(&input, "local");
        assert_eq!(projection.groups.len(), 1);
        assert_eq!(projection.groups[0].group, "seen");
        assert_eq!(projection.groups[0].count, 1);
        assert_eq!(projection.agents().count(), 0);
        assert_eq!(
            projection.groups[0].sleeping_sessions[0].title,
            "Review parser"
        );
        let sent = serde_json::to_string(&projection).unwrap();
        for private in [
            "path",
            "native",
            "retired-pane",
            "private-proof",
            "pane_id",
            "another-device",
        ] {
            assert!(
                !sent.contains(private),
                "private field reached phone: {private}"
            );
        }
    }

    #[test]
    fn groups_come_in_the_sidebar_order_with_devices_included() {
        let projection = project(&rest(), "local");
        let order: Vec<_> = projection
            .groups
            .iter()
            .map(|group| (group.group.as_str(), group.count))
            .collect();
        assert_eq!(
            order,
            [("needs_you", 1), ("done", 1), ("working", 2), ("seen", 1)]
        );
        let done = &projection.groups[1].agents[0];
        assert_eq!(done.device_id, "mini");
        assert_eq!(done.device_label.as_deref(), Some("mini"));
        assert_eq!(done.place.as_deref(), Some("contong · main"));
        assert_eq!(done.mark, "done");
        assert_eq!(
            done.line,
            Some(Line {
                text: "PR #186".into(),
                mode: "news".into(),
                tone: "news",
                pull_request: None,
            })
        );
        assert_eq!(done.key().herdr_pane_id(), "w2:p1");
        // The unreachable device lists nothing.
        assert!(projection.agents().all(|agent| agent.device_id != "gone"));
    }

    /// The phone draws the desktop's mark and line as the core decided them:
    /// a waiting root's cause line, a read question's dimmed request, a
    /// delegated child's quiet mark beside its request, work left kept.
    #[test]
    fn phone_rows_take_the_core_mark_and_line() {
        let input = json!({"navigator": {"agents": [
            agent("root", "working", json!({"wait": "children", "mark": "waiting",
                "descendant_counts": {"working": 2},
                "state": {"line": line("", "waiting", "subtle")}})),
            agent("read-question", "seen", json!({"demand": "question", "mark": "turn",
                "state": {"line": line("  answer me  ", "request", "warning")}})),
            agent("child", "seen", json!({"lineage_parent_pane_id": "root", "delegated": true,
                "demand": "error", "mark": "quiet",
                "state": {"line": line("failed", "request", "warning")}})),
            agent("stopped", "done", json!({"mark": "unread", "unread": true,
                "state": {"line": line("deploy left", "news", "news")}})),
            agent("kept", "seen", json!({"mark": "quiet",
                "state": {"line": line("tests left", "kept", "subtle")}})),
            agent("unknown", "seen", json!({"mark": "unknown",
                "state": {"line": line("quiet", "quiet", "subtle")}})),
        ]}});
        let projection = project(&input, "local");
        let rows: Vec<_> = projection
            .agents()
            .map(|row| {
                (
                    row.pane_id.as_str(),
                    row.root_pane_id.as_str(),
                    row.mark.as_str(),
                    row.line
                        .as_ref()
                        .map(|line| (line.text.as_str(), line.mode.as_str(), line.tone)),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "stopped",
                    "stopped",
                    "unread",
                    Some(("deploy left", "news", "news"))
                ),
                ("root", "root", "waiting", Some(("", "waiting", "subtle"))),
                (
                    "read-question",
                    "read-question",
                    "turn",
                    Some(("answer me", "request", "warning"))
                ),
                (
                    "child",
                    "root",
                    "quiet",
                    Some(("failed", "request", "warning"))
                ),
                (
                    "kept",
                    "kept",
                    "quiet",
                    Some(("tests left", "kept", "subtle"))
                ),
                ("unknown", "unknown", "unknown", None),
            ]
        );
        let root = projection
            .agents()
            .find(|row| row.pane_id == "root")
            .unwrap();
        assert_eq!(
            (root.wait.as_deref(), root.descendants_working),
            (Some("children"), 2)
        );
    }

    /// The request view's words (the operator's request, its summary and
    /// the earlier turns', the agent's reply) stay on the desktop: the
    /// phone's rows carry none of them (PRD overview-request-view Risks,
    /// pane-last-request D-13).
    #[test]
    fn the_request_views_text_never_reaches_a_phone_row() {
        let mut rest = rest();
        rest["navigator"]["agents"][0]["request"] = json!({
            "verb": "working", "verb_since_unix_ms": 1,
            "request": {"text": "secret request words", "cut": false, "images": 0, "at_unix_ms": 1, "sender": {"kind": "operator"}},
            "request_summary": "secret summary",
            "recent_turns": [{"request": "secret earlier summary", "sender": {"kind": "operator"}, "end": "done", "line": "secret earlier line", "at_unix_ms": 0}],
            "reply": {"text": "secret reply words", "cut": false, "at_unix_ms": 2},
            "later_by": null, "pull_requests": [],
        });
        let projection = project(&rest, "local");
        let sent = serde_json::to_string(&projection.groups).unwrap();
        assert!(!sent.contains("secret"), "{sent}");
    }

    #[test]
    fn rows_carry_place_line_and_lineage_root() {
        let projection = project(&rest(), "local");
        let asking = &projection.groups[0].agents[0];
        assert_eq!(asking.place.as_deref(), Some("hide · prd/mobile-companion"));
        assert_eq!(asking.mark, "turn");
        assert_eq!(asking.line.as_ref().unwrap().tone, "warning");
        assert_eq!(asking.key().herdr_pane_id(), "w1:p2");
        let child = projection
            .agents()
            .find(|agent| agent.pane_id == "w1:p3")
            .unwrap();
        assert_eq!(child.root_pane_id, "w1:p1");
        let seen = &projection.groups[3].agents[0];
        assert_eq!(seen.place.as_deref(), Some("notes"));
        assert_eq!(seen.line, None, "a quiet sentence is not drawn");
        assert!(
            serde_json::to_string(&projection)
                .unwrap()
                .find("path")
                .is_none()
        );
    }

    #[test]
    fn the_interface_language_is_read_as_stored_and_a_change_makes_the_projection_differ() {
        let mut rest = rest();
        assert_eq!(project(&rest, "local").interface_language, None, "absent");
        rest["ui_state"] = json!({"interface_language": null});
        let unset = project(&rest, "local");
        assert_eq!(unset.interface_language, None, "null");
        rest["ui_state"] = json!({"interface_language": 7});
        assert_eq!(
            project(&rest, "local").interface_language,
            None,
            "not a string"
        );
        rest["ui_state"] = json!({"interface_language": "ko"});
        let korean = project(&rest, "local");
        assert_eq!(korean.interface_language.as_deref(), Some("ko"));
        assert_ne!(korean, unset, "the watch channel must republish");
        rest["ui_state"] = json!({"interface_language": "zh-CN"});
        assert_eq!(
            project(&rest, "local").interface_language.as_deref(),
            Some("zh-CN"),
            "the string is not validated here"
        );
    }

    #[test]
    fn the_color_theme_reaches_the_phone_and_a_change_makes_the_projection_differ() {
        let options: herdr_core::CoreOptions = serde_json::from_value(json!({
            "schema_version": 2, "node_id": "test-node", "app_state_path": ""
        }))
        .unwrap();
        let mut snapshot = herdr_core::Snapshot::initial(&options);
        let first = herdr_core::RestSections::capture(&snapshot, None, 1);
        let mut kept = Value::Null;
        assert!(update_inputs(&mut kept, &first, 0).unwrap());
        let hide = project(&kept, "local");
        assert_eq!(hide.color_theme.as_deref(), Some("hide"), "the default");
        snapshot.ui_state.color_theme = serde_json::from_value(json!("neutral")).unwrap();
        let neutral = herdr_core::RestSections::capture(&snapshot, Some(&first), 2);
        assert!(update_inputs(&mut kept, &neutral, 1).unwrap());
        let projected = project(&kept, "local");
        assert_eq!(projected.color_theme.as_deref(), Some("neutral"));
        assert_ne!(projected, hide, "the watch channel must republish");
    }
}
