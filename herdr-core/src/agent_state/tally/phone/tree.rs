//! The phone list's tree, order and folds. The phone only draws what this
//! decides (PRD mobile-companion-ui D-01 to D-04): children stand under their
//! parent as the sidebar's tree stands them (`lineage::TREE_LEVELS`,
//! `SIBLINGS_SHOWN`, `sibling_order`), rows of this Mac and of every connected
//! device are mixed in one most-recent-first order (`recent_first`, the order
//! `sort_agents` gives every surface), and a group lists only its first rows
//! before its "N more".

use std::collections::HashMap;

use super::{Descendants, Group, GroupRow, PhoneAgent, PhoneSleepingSession};
use crate::agent_state::recent_first;
use crate::agent_state::tally::lineage::{SIBLINGS_SHOWN, TREE_LEVELS, sibling_order};
use crate::agent_state::tally::scope::RAISED_SHOWN;

/// How many rows 작업 중 and 확인함 list before their "N more" (PRD
/// mobile-companion-ui D-04); 내 차례 and 읽을 것 stop where the sidebar's
/// raised sections do (`RAISED_SHOWN`). 다시 볼 것 is the operator's own
/// starred list and never folds.
const WORKING_SHOWN: usize = 5;
const SEEN_SHOWN: usize = 3;

/// The rows a group lists before its "N more", or all of them.
fn shown_in(group: &str) -> usize {
    match group {
        "working" => WORKING_SHOWN,
        "seen" => SEEN_SHOWN,
        group => RAISED_SHOWN
            .iter()
            .find(|(raised, _)| *raised == group)
            .map_or(usize::MAX, |(_, shown)| *shown),
    }
}

/// The groups in `order` first, then any other group a row names, each with
/// its top rows mixed and folded and every row's children attached.
pub(super) fn groups(
    agents: Vec<PhoneAgent>,
    sleeping: Vec<PhoneSleepingSession>,
    order: &[&str],
) -> Vec<Group> {
    let mut slots: Vec<Option<PhoneAgent>> = agents.into_iter().map(Some).collect();
    let index: HashMap<String, usize> = slots
        .iter()
        .enumerate()
        .filter_map(|(at, agent)| Some((agent.as_ref()?.pane_id.clone(), at)))
        .collect();
    // A row is a child only while its parent is on the phone; an orphan whose
    // parent is not listed is a root again, as on the desktop (D-52).
    let mut roots = Vec::new();
    let mut children: HashMap<usize, Vec<usize>> = HashMap::new();
    for (at, slot) in slots.iter().enumerate() {
        let agent = slot.as_ref().expect("every slot is filled before the tree");
        match agent
            .delegated
            .then(|| agent.parent_pane_id.as_deref().and_then(|id| index.get(id)))
            .flatten()
        {
            Some(&parent) if parent != at => children.entry(parent).or_default().push(at),
            _ => roots.push(at),
        }
    }
    let mut top: Vec<(String, Vec<GroupRow>)> = order
        .iter()
        .map(|group| ((*group).to_owned(), Vec::new()))
        .collect();
    for at in roots {
        let agent = attach(&mut slots, &children, at, 0);
        let rows = match top.iter_mut().find(|(group, _)| *group == agent.group) {
            Some((_, rows)) => rows,
            None => {
                top.push((agent.group.clone(), Vec::new()));
                &mut top.last_mut().expect("a group was just added").1
            }
        };
        rows.push(GroupRow::Agent(Box::new(agent)));
    }
    for session in sleeping {
        match top.iter_mut().find(|(group, _)| *group == session.group) {
            Some((_, rows)) => rows.push(GroupRow::Sleeping(session)),
            None => crate::diagnostic!(serde_json::json!({
                "component": "mobile_phone", "kind": "sleep.group_invalid", "sleep_id": session.sleep_id,
            })),
        }
    }
    top.into_iter()
        .filter(|(_, rows)| !rows.is_empty())
        .map(|(group, mut rows)| {
            // Stable: rows with one key keep the order they were listed in,
            // this Mac's before a device's.
            rows.sort_by(|left, right| recent_first(&left.activity(), &right.activity()));
            let count = rows.len();
            let more = rows.split_off(shown_in(&group).min(count));
            Group {
                more_asleep: more.iter().filter(|row| row.asleep()).count(),
                group,
                count,
                rows,
                more,
            }
        })
        .collect()
}

/// The row at `at` with its children and grandchildren hung under it, the
/// first `SIBLINGS_SHOWN` of each parent's in `children` and the rest in
/// `more_children`.
fn attach(
    slots: &mut [Option<PhoneAgent>],
    children: &HashMap<usize, Vec<usize>>,
    at: usize,
    level: usize,
) -> PhoneAgent {
    let mut row = slots[at].take().expect("a row hangs under one parent");
    row.children_open = row.descendants.asking > 0;
    if level >= TREE_LEVELS {
        return row;
    }
    let mut kids: Vec<(usize, u8, String)> = children
        .get(&at)
        .into_iter()
        .flatten()
        .map(|&kid| {
            let agent = slots[kid].as_ref().expect("a child is not yet attached");
            (kid, agent.tree_rank, agent.activity.clone())
        })
        .collect();
    kids.sort_by(|left, right| sibling_order((left.1, &left.2), (right.1, &right.2)));
    let mut kids = kids.into_iter().map(|(kid, ..)| kid);
    for kid in kids.by_ref().take(SIBLINGS_SHOWN) {
        row.children.push(attach(slots, children, kid, level + 1));
    }
    for kid in kids {
        row.more_children
            .push(attach(slots, children, kid, level + 1));
    }
    row
}

impl Descendants {
    /// The core's counts of a row's live descendants as the parent row's
    /// three pills draw them: asking the operator, working, finished.
    pub(super) fn of(counts: Option<&serde_json::Value>) -> Self {
        let count = |field: &str| {
            counts
                .and_then(|counts| counts.get(field))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
        };
        Self {
            asking: count("error") + count("approval") + count("question"),
            working: count("working"),
            done: count("done"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Group, GroupRow, PhoneAgent, Projection, project};
    use serde_json::{Value, json};

    fn agent(pane: &str, group: &str, activity: u64, extra: Value) -> Value {
        let mut value = json!({
            "pane_id": pane, "group": group, "mark": "going", "identity_label": pane,
            "agent_kind": "claude", "status_code": "idle", "demand": "none",
            "last_activity": format!("{activity:013}"),
            "changed_at_unix_ms": activity, "state": {"tree_rank": 2},
        });
        for (key, field) in extra.as_object().unwrap() {
            value[key] = field.clone();
        }
        value
    }

    fn child(pane: &str, parent: &str, activity: u64, extra: Value) -> Value {
        let mut value = agent(
            pane,
            "working",
            activity,
            json!({"delegated": true, "lineage_parent_pane_id": parent}),
        );
        for (key, field) in extra.as_object().unwrap() {
            value[key] = field.clone();
        }
        value
    }

    fn projected(agents: Vec<Value>) -> Projection {
        project(&json!({"navigator": {"agents": agents}}), "local")
    }

    fn panes(rows: &[GroupRow]) -> Vec<&str> {
        rows.iter()
            .map(|row| match row {
                GroupRow::Agent(agent) => agent.pane_id.as_str(),
                GroupRow::Sleeping(session) => session.sleep_id.as_str(),
            })
            .collect()
    }

    fn group<'a>(projection: &'a Projection, name: &str) -> &'a Group {
        projection
            .groups
            .iter()
            .find(|group| group.group == name)
            .unwrap_or_else(|| panic!("no group {name}"))
    }

    fn top<'a>(projection: &'a Projection, name: &str, pane: &str) -> &'a PhoneAgent {
        group(projection, name)
            .rows
            .iter()
            .find_map(|row| match row {
                GroupRow::Agent(agent) if agent.pane_id == pane => Some(agent),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no top row {pane}"))
    }

    fn ids(agents: &[PhoneAgent]) -> Vec<&str> {
        agents.iter().map(|agent| agent.pane_id.as_str()).collect()
    }

    /// B1, B5: a child hangs under its parent and is no row of any group, so
    /// a group's count is its top rows.
    #[test]
    fn a_child_hangs_under_its_parent_and_is_not_counted_in_a_group() {
        let projection = projected(vec![
            agent(
                "p",
                "working",
                10,
                json!({"descendant_counts": {"working": 2}}),
            ),
            child("c1", "p", 9, json!({})),
            child("g1", "c1", 8, json!({})),
        ]);
        let working = group(&projection, "working");
        assert_eq!(working.count, 1);
        let parent = top(&projection, "working", "p");
        assert_eq!(ids(&parent.children), ["c1"]);
        assert_eq!(ids(&parent.children[0].children), ["g1"]);
        assert_eq!(parent.descendants.working, 2);
        assert_eq!(
            projection
                .agents()
                .map(|a| a.pane_id.as_str())
                .collect::<Vec<_>>(),
            ["p", "c1", "g1"],
            "every row stays reachable for a detail or a notice"
        );
    }

    /// An orphan whose parent is not on the phone is a root again, so it is
    /// never lost (the desktop's rule for a parent that is gone).
    #[test]
    fn a_child_whose_parent_is_not_listed_is_a_root() {
        let projection = projected(vec![child("c", "gone", 9, json!({}))]);
        assert_eq!(panes(&group(&projection, "working").rows), ["c"]);
    }

    /// B4: two levels open under a root, the third is only counted.
    #[test]
    fn only_two_levels_of_children_are_listed() {
        let projection = projected(vec![
            agent("p", "working", 10, json!({})),
            child("c", "p", 9, json!({})),
            child("g", "c", 8, json!({})),
            child("deep", "g", 7, json!({})),
        ]);
        let grandchild = &top(&projection, "working", "p").children[0].children[0];
        assert_eq!(grandchild.pane_id, "g");
        assert!(grandchild.children.is_empty() && grandchild.more_children.is_empty());
        assert!(projection.agents().all(|agent| agent.pane_id != "deep"));
    }

    /// B4: siblings stand most urgent first, then most recent, five to a
    /// parent, and the rest wait for "더 보기".
    #[test]
    fn siblings_stand_by_urgency_then_recency_and_five_are_listed() {
        let mut rows = vec![agent("p", "working", 100, json!({}))];
        for (at, name) in ["a", "b", "c", "d", "e", "f", "g"].iter().enumerate() {
            rows.push(child(name, "p", 10 + at as u64, json!({})));
        }
        // The oldest child asks the operator: most urgent.
        rows.push(child("ask", "p", 1, json!({"state": {"tree_rank": 0}})));
        let projection = projected(rows);
        let parent = top(&projection, "working", "p");
        assert_eq!(ids(&parent.children), ["ask", "g", "f", "e", "d"]);
        assert_eq!(ids(&parent.more_children), ["c", "b", "a"]);
    }

    /// B3: a parent opens while a descendant waits for the operator, and
    /// stays closed otherwise.
    #[test]
    fn a_parent_opens_only_while_a_descendant_asks() {
        let projection = projected(vec![
            agent(
                "asked",
                "working",
                10,
                json!({"descendant_counts": {"question": 1, "working": 1}}),
            ),
            agent(
                "quiet",
                "working",
                9,
                json!({"descendant_counts": {"working": 2}}),
            ),
            agent(
                "failed",
                "working",
                8,
                json!({"descendant_counts": {"error": 1}}),
            ),
        ]);
        let open = |pane: &str| top(&projection, "working", pane).children_open;
        assert_eq!(
            (open("asked"), open("quiet"), open("failed")),
            (true, false, true)
        );
        assert_eq!(top(&projection, "working", "asked").descendants.asking, 1);
    }

    /// B6: rows of this Mac and of a connected device stand in one most
    /// recent first order, so a device's fresh row is above an older local one.
    #[test]
    fn rows_of_every_machine_stand_in_one_recency_order() {
        let rest = json!({
            "navigator": {"agents": [
                agent("local-new", "seen", 5_000, json!({})),
                agent("local-old", "seen", 1_000, json!({})),
            ], "devices": [{"id": "mini", "label": "Mac mini"}]},
            "status": {"remote": [{"target_id": "mini", "state": "connected", "session": {"agents": [
                agent("remote:mini:pane:w1:p1", "seen", 3_000, json!({})),
            ]}}]},
        });
        let projection = project(&rest, "local");
        assert_eq!(
            panes(&group(&projection, "seen").rows),
            ["local-new", "remote:mini:pane:w1:p1", "local-old"]
        );
    }

    /// B5, B7: each group lists its first rows (내 차례 5, 읽을 것 3, 작업 중
    /// 5, 확인함 3), the rest wait behind "더 보기", and the count is every top row.
    #[test]
    fn a_group_lists_its_first_rows_and_counts_the_folded_ones() {
        let mut rows = Vec::new();
        for (group, total) in [("needs_you", 7), ("done", 4), ("working", 6), ("seen", 9)] {
            for at in 0..total {
                rows.push(agent(
                    &format!("{group}-{at}"),
                    group,
                    1_000 + at,
                    json!({}),
                ));
            }
        }
        let projection = projected(rows);
        let shown: Vec<_> = projection
            .groups
            .iter()
            .map(|g| (g.group.as_str(), g.rows.len(), g.more.len(), g.count))
            .collect();
        assert_eq!(
            shown,
            [
                ("needs_you", 5, 2, 7),
                ("done", 3, 1, 4),
                ("working", 5, 1, 6),
                ("seen", 3, 6, 9)
            ]
        );
        assert_eq!(
            panes(&group(&projection, "seen").rows),
            ["seen-8", "seen-7", "seen-6"],
            "the most recent three are listed"
        );
    }

    /// B7: sleeping conversations stand in the same order and fold, and a
    /// fold says how many of its rows sleep.
    #[test]
    fn sleeping_conversations_follow_the_order_and_the_fold() {
        let sleeping = |id: &str, since: u64| {
            json!({
                "sleep_id": id, "node_id": "local", "kind": "pi", "identity_label": id,
                "group": "seen", "phase": "sleeping", "since_unix_ms": since,
                "wake_available": true, "checking": false, "reason": null,
            })
        };
        let rest = json!({"navigator": {
            "agents": [
                agent("a1", "seen", 9_000, json!({})),
                agent("a2", "seen", 2_000, json!({})),
            ],
            "sleeping_sessions": [sleeping("sleep-a-1", 8_000), sleeping("sleep-a-2", 1_000), sleeping("sleep-a-3", 500)],
        }});
        let projection = project(&rest, "local");
        let seen = group(&projection, "seen");
        assert_eq!(panes(&seen.rows), ["a1", "sleep-a-1", "a2"]);
        assert_eq!(panes(&seen.more), ["sleep-a-2", "sleep-a-3"]);
        assert_eq!((seen.count, seen.more_asleep), (5, 2));
    }

    /// B8, B9: the paired Mac and every device it reaches, each with its
    /// state and its counts, which add up to the groups' counts.
    #[test]
    fn the_devices_of_the_mac_carry_state_and_counts() {
        let rest = json!({
            "navigator": {"agents": [
                agent("l1", "needs_you", 3, json!({})),
                agent("l2", "working", 2, json!({})),
            ], "devices": [{"id": "mini", "label": "Mac mini"}, {"id": "old", "label": "Old box"}]},
            "status": {"remote": [
                {"target_id": "mini", "state": "connected", "session": {"agents": [
                    agent("remote:mini:pane:w1:p1", "needs_you", 5, json!({})),
                    agent("remote:mini:pane:w1:p2", "working", 4, json!({})),
                    agent("remote:mini:pane:w1:p3", "working", 3, json!({})),
                ]}},
                {"target_id": "old", "state": "unreachable", "session": {"agents": [
                    agent("remote:old:pane:w1:p1", "needs_you", 9, json!({})),
                ]}},
            ]},
        });
        let projection = project(&rest, "local");
        let summary: Vec<_> = projection
            .devices
            .iter()
            .map(|d| {
                (
                    d.device_id.as_str(),
                    d.label.as_deref(),
                    d.local,
                    d.connected,
                    d.needs_you,
                    d.working,
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("local", None, true, true, 1, 1),
                ("mini", Some("Mac mini"), false, true, 1, 2),
                ("old", Some("Old box"), false, false, 0, 0),
            ]
        );
        assert_eq!(group(&projection, "needs_you").count, 2);
        assert_eq!(group(&projection, "working").count, 3);
    }

    /// The sidebar draws its siblings through `web/src/sidebarTree.ts`; the
    /// phone's cap is the same number, held equal here so the two cannot drift.
    #[test]
    fn the_sibling_cap_is_the_sidebars() {
        let source = include_str!("../../../../../web/src/sidebarTree.ts");
        let declared = format!(
            "export const SIBLINGS_SHOWN = {};",
            crate::agent_state::tally::lineage::SIBLINGS_SHOWN
        );
        assert!(
            source.contains(&declared),
            "sidebarTree.ts must declare `{declared}`"
        );
    }
}
