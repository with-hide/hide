use super::scope::*;
use crate::agent_state::{RequestVerb, turn::row_state};
use crate::model::*;
use crate::sidebar::{SessionSnapshotPayload, project_agents};
use serde_json::json;

fn rows() -> Vec<SidebarAgentSnapshot> {
    let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":
        (["root", "child", "read", "done", "unknown"].map(|id| json!({"pane_id": id, "agent": "claude", "agent_status": "idle", "state_change_seq": 1})))
    })).unwrap();
    let mut rows = project_agents(payload).agents;
    for row in &mut rows {
        row.group = match row.pane_id.as_str() {
            "root" => "working",
            "done" => "done",
            _ => "seen",
        }
        .into();
        row.delegated = row.pane_id == "child";
        row.wait = (row.pane_id == "root").then_some(crate::model::AgentWait::Children);
        row.demand = if matches!(row.pane_id.as_str(), "child" | "read") {
            "question"
        } else {
            "none"
        }
        .into();
        row.activity = if row.pane_id == "unknown" {
            "unknown"
        } else {
            "stopped"
        }
        .into();
        row.unread = row.pane_id == "done";
        row.emphasized = row.unread;
        row.state = row_state(row);
        row.state.verb = match row.pane_id.as_str() {
            "root" => RequestVerb::Waiting,
            "child" | "read" => RequestVerb::Answer,
            "done" => RequestVerb::Result,
            _ => RequestVerb::Idle,
        };
        if row.pane_id == "root" {
            row.lineage_child_pane_ids = vec!["child".into()];
            row.close_descendant_pane_ids = vec!["child".into()];
        }
        if row.pane_id == "child" {
            row.lineage_parent_pane_id = Some("root".into());
        }
    }
    rows
}

fn refs(rows: &[SidebarAgentSnapshot]) -> Vec<&SidebarAgentSnapshot> {
    rows.iter().collect()
}
fn membership(rows: &[&SidebarAgentSnapshot], project: &str) -> Vec<Member> {
    rows.iter()
        .map(|row| Member {
            pane_id: row.pane_id.clone(),
            project_id: project.into(),
            checkout_id: format!("{project}-checkout"),
        })
        .collect()
}

#[test]
fn physical_groups_root_headings_and_requests_keep_the_frozen_screen_values() {
    let rows = rows();
    let rows = refs(&rows);
    let value = scope(
        &rows,
        membership(&rows, "all"),
        &rows,
        MarkCountsSnapshot::default(),
    );
    assert_eq!(
        value.groups,
        GroupCounts {
            needs_you: 0,
            done: 1,
            working: 1,
            seen: 3
        }
    );
    assert_eq!(
        value
            .sections
            .iter()
            .map(|section| (section.group.as_str(), section.count))
            .collect::<Vec<_>>(),
        vec![("done", 1), ("working", 2), ("seen", 2)]
    );
    assert_eq!(value.overview_total, 5);
    assert_eq!(
        value.buckets,
        Buckets {
            turn: 1,
            working: 0,
            delegating: 1,
            resting: 3
        }
    );
    assert_eq!((value.requests.todo, value.requests.answer), (2, 1));
    assert_eq!(
        value
            .requests
            .groups
            .iter()
            .map(|g| (g.verb, g.rows.len()))
            .collect::<Vec<_>>(),
        vec![
            (RequestVerb::Answer, 1),
            (RequestVerb::Result, 1),
            (RequestVerb::Waiting, 1),
            (RequestVerb::Idle, 1)
        ]
    );
    assert_eq!(
        value
            .requests
            .rows
            .iter()
            .map(|r| value.members[r.member].pane_id.as_str())
            .collect::<Vec<_>>(),
        vec!["root", "read", "done", "unknown"]
    );
    let child = rows
        .iter()
        .copied()
        .filter(|r| r.pane_id == "child")
        .collect::<Vec<_>>();
    let child_scope = scope(
        &child,
        membership(&child, "child-project"),
        &rows,
        MarkCountsSnapshot::default(),
    );
    assert_eq!(
        (child_scope.requests.todo, child_scope.requests.answer),
        (1, 1)
    );
    assert_eq!(child_scope.requests.rows.len(), 1);
}

#[test]
fn defaults_publish_known_zero_request_counts_and_unknown_groups_keep_their_heading() {
    assert_eq!(Scope::default().requests.counts.len(), 8);
    let mut rows = rows();
    rows.retain(|r| r.pane_id == "unknown");
    rows[0].group = "future_group".into();
    let rows = refs(&rows);
    let value = scope(
        &rows,
        membership(&rows, "one"),
        &rows,
        MarkCountsSnapshot::default(),
    );
    assert_eq!(value.total, 1);
    assert_eq!(value.groups, GroupCounts::default());
    assert_eq!(value.sections[0].group, "future_group");
    assert_eq!(value.sections[0].count, 1);
}
