//! The phone list's tree, order and folds. The phone only draws what this
//! decides (PRD mobile-companion-ui D-01 to D-04): children stand under their
//! parent as the sidebar's tree stands them (`lineage::TREE_LEVELS`,
//! `SIBLINGS_SHOWN`, `children_in_order`), rows of this Mac and of every connected
//! device are mixed in one most-recent-first order (`recent_first`, the order
//! `sort_agents` gives every surface), and a group lists only its first rows
//! before its "N more".

use std::collections::HashMap;

use super::{Descendants, Group, GroupRow, PhoneAgent, PhoneSleepingSession};
use crate::agent_state::recent_first;
use crate::agent_state::tally::lineage::{SIBLINGS_SHOWN, TREE_LEVELS, children_in_order};
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
    // A delegated row hangs under the parent that lists it among its
    // children, in the sidebar's child order; one whose parent is not on the
    // phone is a root again, as on the desktop (D-52).
    let mut claimed = vec![false; slots.len()];
    for (at, slot) in slots.iter().enumerate() {
        let agent = slot.as_ref().expect("every slot is filled before the tree");
        for id in &agent.child_pane_ids {
            if let Some(kid) = kid_at(&slots, &index, at, id) {
                claimed[kid] = true;
            }
        }
    }
    let mut top: Vec<(String, Vec<GroupRow>)> = order
        .iter()
        .map(|group| ((*group).to_owned(), Vec::new()))
        .collect();
    // The roots first, then every row again: a lineage that loops claims all
    // of its rows, so none is a root, and placing what is left keeps it listed.
    // A row already placed answers `None`.
    for at in (0..slots.len())
        .filter(|&at| !claimed[at])
        .chain(0..slots.len())
    {
        let Some(agent) = attach(&mut slots, &index, at, 0) else {
            continue;
        };
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
/// `more_children`. A grandchild's own descendants, which the sidebar opens
/// in its popover, wait flat behind that row's "더 보기", so every agent stays
/// on the phone to open or answer. `None` when another parent placed it.
fn attach(
    slots: &mut [Option<PhoneAgent>],
    index: &HashMap<String, usize>,
    at: usize,
    level: usize,
) -> Option<PhoneAgent> {
    let mut row = slots[at].take()?;
    row.children_open = row.descendants.asking > 0;
    let kids = kids_of(slots, index, at, &row);
    if level >= TREE_LEVELS {
        for kid in kids {
            flatten(slots, index, kid, &mut row.more_children);
        }
        return Some(row);
    }
    let mut kids = kids.into_iter();
    for kid in kids.by_ref().take(SIBLINGS_SHOWN) {
        row.children.extend(attach(slots, index, kid, level + 1));
    }
    for kid in kids {
        row.more_children
            .extend(attach(slots, index, kid, level + 1));
    }
    Some(row)
}

/// The row at `at` and, after it, everything below it, parent first.
fn flatten(
    slots: &mut [Option<PhoneAgent>],
    index: &HashMap<String, usize>,
    at: usize,
    out: &mut Vec<PhoneAgent>,
) {
    let Some(mut row) = slots[at].take() else {
        return;
    };
    row.children_open = row.descendants.asking > 0;
    let kids = kids_of(slots, index, at, &row);
    out.push(row);
    for kid in kids {
        flatten(slots, index, kid, out);
    }
}

/// The delegated rows still unplaced under `row` (the row at `at`), in the
/// order every tree stands them.
fn kids_of(
    slots: &[Option<PhoneAgent>],
    index: &HashMap<String, usize>,
    at: usize,
    row: &PhoneAgent,
) -> Vec<usize> {
    children_in_order(
        &row.child_pane_ids,
        |id| kid_at(slots, index, at, id),
        |kid| slots[kid].as_ref().map_or(u8::MAX, |agent| agent.tree_rank),
    )
}

/// The slot of the delegated row `id` that the row at `at` lists as a child,
/// while that row is on the phone and not yet placed.
fn kid_at(
    slots: &[Option<PhoneAgent>],
    index: &HashMap<String, usize>,
    at: usize,
    id: &str,
) -> Option<usize> {
    let kid = *index.get(id)?;
    (kid != at && slots[kid].as_ref().is_some_and(|agent| agent.delegated)).then_some(kid)
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

    /// The rows as the core lists them: a parent names its children in the
    /// order they were listed, unless the row already says (`lineage_child_pane_ids`).
    fn projected(mut agents: Vec<Value>) -> Projection {
        let named: Vec<(String, String)> = agents
            .iter()
            .filter_map(|row| {
                Some((
                    row.get("lineage_parent_pane_id")?.as_str()?.to_owned(),
                    row.get("pane_id")?.as_str()?.to_owned(),
                ))
            })
            .collect();
        for row in &mut agents {
            if row.get("lineage_child_pane_ids").is_some() {
                continue;
            }
            let pane = row["pane_id"].as_str().unwrap().to_owned();
            let kids: Vec<_> = named
                .iter()
                .filter(|(parent, _)| *parent == pane)
                .map(|(_, kid)| kid.clone())
                .collect();
            row["lineage_child_pane_ids"] = json!(kids);
        }
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

    /// B4: two levels open under a root; what lies deeper stays on the phone,
    /// flat and parent first, behind the grandchild's "더 보기", so the phone
    /// can still open or answer it.
    #[test]
    fn what_lies_deeper_than_two_levels_waits_behind_the_grandchild() {
        let projection = projected(vec![
            agent("p", "working", 10, json!({})),
            child("c", "p", 9, json!({})),
            child("g", "c", 8, json!({})),
            child("deep", "g", 7, json!({})),
            child("deeper", "deep", 6, json!({})),
            child("deep-too", "g", 5, json!({})),
        ]);
        let grandchild = &top(&projection, "working", "p").children[0].children[0];
        assert_eq!(grandchild.pane_id, "g");
        assert!(grandchild.children.is_empty());
        assert_eq!(
            ids(&grandchild.more_children),
            ["deep", "deeper", "deep-too"]
        );
        assert!(
            grandchild
                .more_children
                .iter()
                .all(|row| row.children.is_empty() && row.more_children.is_empty())
        );
        let found = |pane: &str| {
            projection
                .agents()
                .find(|row| row.pane_id == pane)
                .is_some_and(|row| projection.find(&row.key()).is_some())
        };
        assert!(
            ["p", "c", "g", "deep", "deeper", "deep-too"]
                .into_iter()
                .all(found)
        );
    }

    /// A lineage that loops names every row as another's child; none of them
    /// is lost for it.
    #[test]
    fn a_looping_lineage_keeps_every_row_listed() {
        let projection = projected(vec![
            child("a", "b", 9, json!({})),
            child("b", "a", 8, json!({})),
        ]);
        let listed: Vec<_> = projection
            .agents()
            .map(|row| row.pane_id.as_str())
            .collect();
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&"a") && listed.contains(&"b"));
    }

    /// B4, D-02: siblings stand as the sidebar stands them, most urgent
    /// first and then in the parent's own child order whatever their
    /// activity, five to a parent, and the rest wait for "더 보기".
    #[test]
    fn siblings_stand_by_urgency_then_the_parents_order_and_five_are_listed() {
        let mut rows = vec![agent("p", "working", 100, json!({}))];
        for (at, name) in ["a", "b", "c", "d", "e", "f", "g"].iter().enumerate() {
            rows.push(child(name, "p", 10 + at as u64, json!({})));
        }
        // The oldest child asks the operator: most urgent.
        rows.push(child("ask", "p", 1, json!({"state": {"tree_rank": 0}})));
        let projection = projected(rows);
        let parent = top(&projection, "working", "p");
        assert_eq!(ids(&parent.children), ["ask", "a", "b", "c", "d"]);
        assert_eq!(ids(&parent.more_children), ["e", "f", "g"]);
    }

    /// D-02: the order the parent names its children in is the sidebar's, so a
    /// more recently active child does not move above one listed before it.
    #[test]
    fn a_more_recent_child_does_not_pass_one_the_parent_lists_first() {
        let mut parent = agent("p", "working", 100, json!({}));
        parent["lineage_child_pane_ids"] = json!(["old", "new"]);
        let projection = projected(vec![
            parent,
            child("new", "p", 90, json!({})),
            child("old", "p", 10, json!({})),
        ]);
        assert_eq!(
            ids(&top(&projection, "working", "p").children),
            ["old", "new"]
        );
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
}
