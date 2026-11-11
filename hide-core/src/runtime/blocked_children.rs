//! A delegated child held at a blocked prompt (an approval, a selection, a
//! question its runtime asks in its own pane): its parent hears at once, and
//! its lineage root is raised only once the block has lasted the grace
//! (`escalation::CHILD_BLOCKED_GRACE_MS`), so the parent handles what it can
//! and the operator gets what it cannot (docs/delivery.md, A blocked child).
//!
//! The block's start is the core's own reading: the session projection pass
//! that first reads the row blocked, whatever made it blocked (the hook's
//! wait, the session's question, the engine's prompt). The block ends when the
//! core reads the row settled and not blocked, another known session in its
//! pane, or the pane gone. A row still being restored (sequence 0, an unknown
//! activity, no session named yet) and a disconnected device's last row keep
//! it, so a device that reconnects keeps the same block, deadline and note.
//! It is memory only: a restart of the core starts the grace again and may
//! send the parent one more note, a known limit.
//!
//! "Once" is the note's intent, `child-blocked:<pane>:<start>`: the ledger
//! takes one letter per sender and intent, so a send repeated converges on
//! the first. Nothing is retried: a parent that cannot be addressed, or a
//! note that cannot go, leaves the grace running, and the operator gets the
//! child when it ends.

use std::collections::{BTreeMap, HashSet};

use serde_json::json;

use super::Runtime;
use super::delivery::Observation;
use super::hide_notes::{Note, NoteKind};
use crate::agent_state::escalation::CHILD_BLOCKED_GRACE_MS;
use crate::model::SidebarAgentSnapshot;

/// The Hide role that tells a parent its child is blocked.
pub(crate) const BLOCKED_ROLE: &str = "blocked";

#[derive(Default)]
pub(in crate::runtime) struct BlockedChildren {
    /// Each delegated child's block, by pane, while it lasts. Bounded by the
    /// session rows: a pane that leaves them takes its block with it.
    starts: BTreeMap<String, Start>,
    /// The notes the last pass found owed, by pane, until the tick takes them.
    owed: BTreeMap<String, Owed>,
    /// The block each pane's parent was told of, by its note's key, while
    /// that block lasts.
    told: BTreeMap<String, String>,
    /// A thread is sending notes; it is the only one.
    sending: bool,
}

struct Start {
    /// The session the block was read in; `None` until the row names one.
    session: Option<String>,
    since: u64,
}

struct Owed {
    key: String,
    parent: String,
    /// The parent session the child's lineage was written for.
    parent_session: Option<String>,
    /// The row's title, for a child Hide does not observe by a registration.
    label: String,
}

impl BlockedChildren {
    /// Follows each row's block at `at`, this machine's and every device's
    /// rows in one pass, and lays a blocked delegated child's start on its
    /// row; then finds the notes owed. Memory only, under the runtime lock.
    pub(in crate::runtime) fn observe(&mut self, rows: &mut [SidebarAgentSnapshot], at: u64) {
        for row in rows.iter_mut() {
            row.blocked_since_unix_ms = None;
            // Another session the pane names is another agent; its block, if
            // any, starts now. A session not named yet is no other session.
            if let Some(start) = self.starts.get(&row.pane_id)
                && let (Some(was), Some(now)) = (&start.session, &row.lineage_session)
                && was != now
            {
                self.starts.remove(&row.pane_id);
            }
            if row.blocked {
                // A child whose lineage a restore has not given back yet
                // keeps its block and lays nothing until it has.
                if row.delegated && row.lineage_parent_pane_id.is_some() {
                    let start = self.starts.entry(row.pane_id.clone()).or_insert(Start {
                        session: row.lineage_session.clone(),
                        since: at,
                    });
                    if start.session.is_none() {
                        start.session = row.lineage_session.clone();
                    }
                    row.blocked_since_unix_ms = Some(start.since);
                }
            } else if crate::agent_state::settled(row) {
                self.starts.remove(&row.pane_id);
            }
        }
        let panes: HashSet<&str> = rows.iter().map(|row| row.pane_id.as_str()).collect();
        self.starts.retain(|pane, _| panes.contains(pane.as_str()));
        let starts = &self.starts;
        self.told.retain(|pane, _| starts.contains_key(pane));
        let told = &self.told;
        self.owed = rows
            .iter()
            // A person's turn skips the parent, which cannot sign in for it.
            .filter(|row| row.declared_human_turn.is_none())
            .filter_map(|row| {
                let since = row.blocked_since_unix_ms?;
                let key = format!("{}:{since}", row.pane_id);
                (told.get(&row.pane_id) != Some(&key)).then(|| {
                    Some((
                        row.pane_id.clone(),
                        Owed {
                            key,
                            parent: row.lineage_parent_pane_id.clone()?,
                            parent_session: row.declared_parent_session.clone(),
                            label: row.identity_label.clone(),
                        },
                    ))
                })?
            })
            .collect();
    }
}

impl Runtime {
    /// Tells each blocked child's parent once per block. Run by the
    /// coordinator's tick; a tick with nothing owed is one test.
    pub(crate) fn drive_blocked_notes(&mut self) {
        if self.blocked_children.owed.is_empty()
            || self.blocked_children.sending
            || self.effects_held()
        {
            return;
        }
        let owed = std::mem::take(&mut self.blocked_children.owed);
        let mut notes = Vec::new();
        for (pane, owed) in owed {
            self.blocked_children
                .told
                .insert(pane.clone(), owed.key.clone());
            let Some(target) = self.blocked_parent(&owed) else {
                crate::diagnostic!(json!({
                    "component": "delivery", "kind": "child_blocked.note_unaddressed",
                    "pane_id": pane, "parent_pane_id": owed.parent,
                }));
                continue;
            };
            crate::diagnostic!(json!({
                "component": "delivery", "kind": "child_blocked.noted",
                "pane_id": pane, "parent_pane_id": target.actor.pane_id,
                "intent": format!("child-blocked:{}", owed.key),
            }));
            let child = self
                .delivery_observations
                .get(&pane)
                .map_or(owed.label, |observation| observation.actor.name.clone());
            notes.push(Note {
                body: note_body(&child, &pane),
                letter: owed.key,
                target,
            });
        }
        if notes.is_empty() {
            return;
        }
        let kind = NoteKind {
            role: BLOCKED_ROLE,
            thread: "hide-blocked-notes",
            intent: "child-blocked",
            component: "delivery",
            unsent: "child_blocked.note_unsent",
            now: true,
        };
        self.blocked_children.sending = self.send_hide_notes(kind, notes, |runtime| {
            runtime.blocked_children.sending = false;
        });
    }

    /// The parent a blocked child's note goes to: the agent the lineage names
    /// observed in its pane, else that pane's agent asleep, whose mailbox
    /// keeps the note for its next wake.
    fn blocked_parent(&self, owed: &Owed) -> Option<Observation> {
        if let Some(observed) = self.delivery_observations.get(&owed.parent) {
            let same = owed
                .parent_session
                .as_ref()
                .is_none_or(|session| observed.actor.session.as_ref() == Some(session));
            return (same && !observed.actor.code_owned()).then(|| observed.clone());
        }
        let ledger: &crate::delivery::ledger::Ledger = self.delivery_ledger.as_ref().ok()?;
        self.sleeping_target(ledger, None, &owed.parent)
    }
}

/// What a parent is told: who is blocked and the one command to start with,
/// on the first line the operator reads in a wake summary; how to answer;
/// when the operator is asked; and who may answer which screen.
fn note_body(child: &str, pane: &str) -> String {
    format!(
        "{child} is blocked at a prompt: read it with `hide pane read {pane}`.\n\
         Answer it with `hide pane send {pane} --key <name>` (esc, enter, up, down, tab, ...) or \
         with text and `--enter`, or send {child} a safer instruction; its runtime may time the \
         prompt out on its own. The operator is asked too if it is still blocked {} seconds \
         after Hide saw it stop.\n{}",
        CHILD_BLOCKED_GRACE_MS / 1_000,
        crate::delivery::typed::SCREEN_CONSENT,
    )
}

#[cfg(test)]
impl Runtime {
    /// The block each pane's parent was told of, by its note's key.
    pub(in crate::runtime) fn blocked_notes_told(&self) -> Vec<(String, String)> {
        self.blocked_children
            .told
            .iter()
            .map(|(pane, key)| (pane.clone(), key.clone()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A delegated child in `pane` that the engine reports as `status` at
    /// sequence `seq`, running `session`.
    fn child(pane: &str, status: &str, seq: u64, session: Option<&str>) -> SidebarAgentSnapshot {
        let payload: crate::sidebar::SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [{"pane_id": pane, "agent": "claude", "agent_status": status, "state_change_seq": seq}]
        }))
        .unwrap();
        let mut row = crate::sidebar::project_agents(payload).agents.remove(0);
        row.delegated = true;
        row.lineage_parent_pane_id = Some("parent".into());
        row.lineage_session = session.map(Into::into);
        row.changed_at_unix_ms = Some(1);
        row
    }

    /// Observes `row` alone at `at` and answers the start laid on it and the
    /// note owed for it.
    fn pass(
        blocks: &mut BlockedChildren,
        row: SidebarAgentSnapshot,
        at: u64,
    ) -> (Option<u64>, Option<String>) {
        let mut rows = [row];
        blocks.observe(&mut rows, at);
        let owed = blocks
            .owed
            .get(&rows[0].pane_id)
            .map(|owed| owed.key.clone());
        (rows[0].blocked_since_unix_ms, owed)
    }

    /// The block starts when the core first reads it, not when the engine
    /// last changed state, lasts while it is read blocked, and ends once the
    /// row reads settled and not blocked; the next block starts again, with
    /// the engine's sequence unchanged as Grok's wait leaves it.
    #[test]
    fn a_block_starts_when_first_read_and_a_second_block_starts_again() {
        let mut blocks = BlockedChildren::default();
        assert!(child("w1:p2", "blocked", 1, Some("s")).blocked);
        assert_eq!(
            pass(&mut blocks, child("w1:p2", "blocked", 1, Some("s")), 100),
            (Some(100), Some("w1:p2:100".into()))
        );
        assert_eq!(
            pass(&mut blocks, child("w1:p2", "blocked", 1, Some("s")), 200).0,
            Some(100)
        );
        assert_eq!(
            pass(&mut blocks, child("w1:p2", "working", 1, Some("s")), 300),
            (None, None),
            "answered: settled and not blocked"
        );
        assert_eq!(
            pass(&mut blocks, child("w1:p2", "blocked", 1, Some("s")), 400),
            (Some(400), Some("w1:p2:400".into()))
        );
    }

    /// A pane still being restored (sequence 0, an unknown state, no session
    /// named yet, its lineage not given back yet) keeps its block, on this
    /// machine and on a device alike; only another known session in the pane
    /// starts a new one.
    #[test]
    fn a_restore_keeps_the_block_and_only_another_known_session_replaces_it() {
        for pane in ["w1:p2", "remote:mini:pane:w1:p2"] {
            let mut blocks = BlockedChildren::default();
            pass(&mut blocks, child(pane, "blocked", 4, Some("s")), 100);
            pass(&mut blocks, child(pane, "idle", 0, Some("s")), 150);
            pass(&mut blocks, child(pane, "unknown", 2, None), 160);
            let mut away = child(pane, "blocked", 3, None);
            away.delegated = false;
            away.lineage_parent_pane_id = None;
            assert_eq!(pass(&mut blocks, away, 170).0, None, "{pane}");
            assert_eq!(
                pass(&mut blocks, child(pane, "blocked", 3, None), 180).0,
                Some(100),
                "{pane}"
            );
            assert_eq!(
                pass(&mut blocks, child(pane, "blocked", 3, Some("s")), 190).0,
                Some(100),
                "{pane}"
            );
            assert_eq!(
                pass(&mut blocks, child(pane, "blocked", 3, Some("t")), 200),
                (Some(200), Some(format!("{pane}:200"))),
                "{pane}"
            );
        }
    }

    /// One note per block: once taken it is no longer owed while the block
    /// lasts; a root and a child that declared a person's turn owe none; a
    /// pane that leaves the rows takes its block and its note with it.
    #[test]
    fn a_block_owes_its_parent_one_note_and_a_root_or_a_persons_turn_owes_none() {
        let mut blocks = BlockedChildren::default();
        pass(&mut blocks, child("w1:p2", "blocked", 1, Some("s")), 100);
        blocks.told.insert("w1:p2".into(), "w1:p2:100".into());
        assert_eq!(
            pass(&mut blocks, child("w1:p2", "blocked", 1, Some("s")), 110),
            (Some(100), None)
        );
        let mut root = child("w1:p3", "blocked", 1, Some("r"));
        root.delegated = false;
        root.lineage_parent_pane_id = None;
        assert_eq!(pass(&mut blocks, root, 120), (None, None));
        let mut declared = child("w1:p4", "blocked", 1, Some("d"));
        declared.declared_human_turn = Some(crate::model::DeclaredHumanTurn {
            id: "agent-4.1".into(),
            kind: crate::coordination::human_turn::HumanKind::Auth,
            place: "github.com/login".into(),
            device: None,
            since_unix_ms: 120,
        });
        assert_eq!(pass(&mut blocks, declared, 130), (Some(130), None));
        blocks.observe(&mut [], 140);
        assert!(blocks.starts.is_empty() && blocks.told.is_empty());
    }
}
