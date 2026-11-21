//! What a machine tells the machines that may open it about its own work,
//! and what this machine holds of what it was told (PRD device-connect-iroh
//! D-50, B49).
//!
//! A machine that may open another sees that machine's Needs You and Done
//! beside its own. The machine that answers for the work is the only source:
//! [`told`] reads this core's list and keeps what the one table gives the
//! machines that may open it (`whose_work::Reach::peers`), hided sends it on
//! the connection, and the receiving hided hands it to its core as one
//! `peer_work` event per machine, replaced whole. What was received is never
//! sent on, rings nothing here (`whose_work::Whose::TheirsAway`) and goes
//! when the machine is no longer connected.

use serde::{Deserialize, Serialize};

use crate::agent_state::phone::{GroupRow, PhoneAgent, Projection};

/// The most rows one machine tells; the counts say how many there are.
pub const MAX_ROWS: usize = 64;

const MAX_ID_BYTES: usize = 256;
const MAX_NAME_CHARS: usize = 160;
const MAX_DETAIL_CHARS: usize = 400;

/// The groups a machine tells of: the ones a person acts on.
pub const GROUPS: [&str; 2] = ["needs_you", "done"];

/// One piece of a machine's own work that waits on a person or is done.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct WorkRow {
    /// The work's id on its own machine: the pane its core knows it by. The
    /// machine that opens the row hands it back to say which work to show.
    pub id: String,
    /// `needs_you` or `done`.
    pub group: String,
    pub title: String,
    pub agent_kind: String,
    /// `project · branch`, or the project alone for a plain folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// The row's mark, as its own machine draws it.
    pub mark: String,
    pub status_code: String,
    /// The sentence its own core chose for the row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// An ask its operator read and has not answered.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_at_unix_ms: Option<u64>,
}

/// What one machine tells: how many of its own top rows stand in Needs You
/// and in Done, and the first [`MAX_ROWS`] of them.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Told {
    pub needs_you: u32,
    pub done: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<WorkRow>,
}

/// What this machine holds of one machine it opens.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PeerWork {
    /// The machine's product device id, the id `peers` names it by.
    pub device_id: String,
    /// Its connection id.
    pub key: String,
    #[serde(flatten)]
    pub told: Told,
}

fn clipped(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn row(agent: &PhoneAgent, group: &str) -> WorkRow {
    WorkRow {
        id: agent.pane_id.clone(),
        group: group.to_owned(),
        title: clipped(&agent.title, MAX_NAME_CHARS),
        agent_kind: clipped(&agent.agent_kind, MAX_NAME_CHARS),
        place: agent
            .place
            .as_deref()
            .map(|place| clipped(place, MAX_NAME_CHARS)),
        mark: clipped(&agent.mark, MAX_NAME_CHARS),
        status_code: clipped(&agent.status_code, MAX_NAME_CHARS),
        detail: agent
            .detail
            .as_deref()
            .map(|detail| clipped(detail, MAX_DETAIL_CHARS)),
        read: agent.read,
        changed_at_unix_ms: agent.changed_at_unix_ms,
    }
}

/// What this core tells the machines that may open it, read from its own
/// list. Only what the table sends to them: this core's work that runs here.
/// Work it placed on another machine is not passed on, and an external
/// tool's work is the tool's, in no one's shared list (D-50).
pub fn told(projection: &Projection) -> Told {
    let mut told = Told::default();
    for name in GROUPS {
        let Some(group) = projection.groups.iter().find(|group| group.group == name) else {
            continue;
        };
        for top in group.rows.iter().chain(&group.more) {
            let GroupRow::Agent(agent) = top else {
                continue;
            };
            if !agent.whose.reach().peers
                || agent.tool.is_some()
                || agent.pane_id.len() > MAX_ID_BYTES
            {
                continue;
            }
            match name {
                "needs_you" => told.needs_you += 1,
                _ => told.done += 1,
            }
            if told.rows.len() < MAX_ROWS {
                told.rows.push(row(agent, name));
            }
        }
    }
    told
}

/// Whether `told` is what a machine may tell: bounded, each row in a group
/// a machine tells of, and no work twice. hided hands on what arrived; this
/// keeps what is not such a list out of the snapshot whoever sent it.
pub fn valid(told: &Told) -> bool {
    let mut ids = std::collections::HashSet::new();
    let name = |text: &str| text.chars().count() <= MAX_NAME_CHARS;
    told.rows.len() <= MAX_ROWS
        && told.rows.iter().all(|row| {
            !row.id.is_empty()
                && row.id.len() <= MAX_ID_BYTES
                && ids.insert(row.id.as_str())
                && GROUPS.contains(&row.group.as_str())
                && name(&row.title)
                && name(&row.agent_kind)
                && row.place.as_deref().is_none_or(name)
                && name(&row.mark)
                && name(&row.status_code)
                && row
                    .detail
                    .as_deref()
                    .is_none_or(|detail| detail.chars().count() <= MAX_DETAIL_CHARS)
        })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::agent_state::phone::project;

    fn agent(pane: &str, group: &str, extra: serde_json::Value) -> serde_json::Value {
        let mut agent = json!({
            "pane_id": pane, "group": group, "agent_kind": "claude",
            "identity_label": format!("task {pane}"), "status_code": "question",
            "detail": "Which branch?", "mark": "turn", "demand": "question",
            "changed_at_unix_ms": 7,
        });
        agent
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        agent
    }

    /// B49, D-50: a machine tells of its own work that runs on it and
    /// waits on a person or is done; work it placed on another machine and
    /// an external tool's work stay with it.
    #[test]
    fn a_machine_tells_only_its_own_work_that_runs_on_it() {
        let rest = json!({
            "navigator": { "agents": [
                agent("w1:p1", "needs_you", json!({})),
                agent("w1:p2", "done", json!({"status_code": "done"})),
                agent("w1:p3", "working", json!({})),
                agent("w1:p4", "needs_you", json!({"tool": {"name": "Conductor"}})),
            ]},
            "status": { "remote": [{
                "target_id": "windows", "state": "connected",
                "session": { "agents": [
                    agent("remote:windows:pane:w1:p9", "needs_you", json!({})),
                ]},
            }]},
        });
        let projection = project(&rest, "mac");
        let told = told(&projection);
        assert_eq!((told.needs_you, told.done), (1, 1));
        assert_eq!(
            told.rows
                .iter()
                .map(|row| (row.id.as_str(), row.group.as_str()))
                .collect::<Vec<_>>(),
            [("w1:p1", "needs_you"), ("w1:p2", "done")]
        );
        assert_eq!(told.rows[0].title, "task w1:p1");
        assert_eq!(told.rows[0].detail.as_deref(), Some("Which branch?"));
        assert!(valid(&told));
    }

    /// The counts are every row; the rows stop at the cap.
    #[test]
    fn rows_stop_at_the_cap_and_the_counts_do_not() {
        let agents: Vec<_> = (0..MAX_ROWS + 5)
            .map(|at| agent(&format!("w1:p{at}"), "needs_you", json!({})))
            .collect();
        let told = told(&project(&json!({"navigator": {"agents": agents}}), "mac"));
        assert_eq!(told.needs_you as usize, MAX_ROWS + 5);
        assert_eq!(told.rows.len(), MAX_ROWS);
        assert!(valid(&told));
    }

    #[test]
    fn a_list_no_machine_tells_is_refused() {
        let row = |id: &str, group: &str| WorkRow {
            id: id.to_owned(),
            group: group.to_owned(),
            title: "t".into(),
            agent_kind: "claude".into(),
            place: None,
            mark: "turn".into(),
            status_code: "question".into(),
            detail: None,
            read: false,
            changed_at_unix_ms: None,
        };
        let told = |rows: Vec<WorkRow>| Told {
            needs_you: 1,
            done: 0,
            rows,
        };
        assert!(valid(&told(vec![row("w1:p1", "needs_you")])));
        // A group no machine tells of, the same work twice, no id.
        assert!(!valid(&told(vec![row("w1:p1", "working")])));
        assert!(!valid(&told(vec![
            row("w1:p1", "done"),
            row("w1:p1", "done")
        ])));
        assert!(!valid(&told(vec![row("", "done")])));
        let mut long = row("w1:p1", "done");
        long.detail = Some("x".repeat(MAX_DETAIL_CHARS + 1));
        assert!(!valid(&told(vec![long])));
        assert!(!valid(&told(
            (0..=MAX_ROWS)
                .map(|at| row(&format!("p{at}"), "done"))
                .collect()
        )));
    }
}
