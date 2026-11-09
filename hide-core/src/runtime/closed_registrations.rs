//! The delivery registrations of agents closed through Hide while Reopen
//! closed can bring them back (docs/ARCHITECTURE.md, Reopening locally
//! closed work).
//!
//! A close kept for reopen holds the registration of each agent its panes
//! ran from the moment it starts (`ClosedAgent::registration`, recorded by
//! `closed_panes`) for as long as its item can be reopened: while the close
//! settles, while a tree close records its entry, and while the item is on
//! `recent_closed`. Its pane's absence then ends neither the registration
//! nor the watches on it. A reopen moves each registration whose
//! conversation resumed to its new pane through the delivery store's rebind,
//! the wake's own continuation (`take_over_registration`), so its letters,
//! the watch on it, its parent and its spawn receipt follow it. A close that
//! settled without an item, an item pushed off the list, and an agent whose
//! conversation did not resume let theirs go, to end with the closed pane as
//! any other.
//!
//! While held the agent reads closed (`coordination::life`): not running, no
//! watch reading and so no quiet warning to its parent, and no place under a
//! mission's or a plan's cap.

use super::*;
use crate::agent_sleep::DormantRegistration;

/// A reopened agent's registration waiting for its new pane.
#[derive(Clone, Debug)]
pub(super) struct Continuation {
    registration: DormantRegistration,
    pane: String,
    since_unix_ms: u64,
    /// The session has listed the pane; a pane that leaves after this is
    /// gone, one not listed yet may still be on its way.
    seen: bool,
}

impl PendingClose {
    /// The registrations this close holds: its closing panes' agents', when
    /// the close is kept for reopen.
    pub(super) fn held_registrations(&self) -> impl Iterator<Item = &DormantRegistration> {
        self.request
            .panes
            .iter()
            .filter(move |pane| {
                self.request.retain_for_reopen && self.pane_ids.contains(&pane.pane_id)
            })
            .filter_map(crate::recent_closed::ClosedPane::registration)
    }
}

impl Runtime {
    /// The registrations of agents that are closed and can come back by
    /// Reopen closed: a kept close still settling, a node of a tree close
    /// still recording its entry, an item on the list.
    fn closed_registrations(&self) -> impl Iterator<Item = &DormantRegistration> {
        self.close_operations
            .values()
            .flat_map(PendingClose::held_registrations)
            .chain(self.tree_capture_registrations())
            .chain(
                self.recent_closed
                    .iter()
                    .flat_map(ClosedItem::held_registrations),
            )
    }

    /// Every registration a closed pane's absence must not end: the closed
    /// ones, and those a reopen is moving to their new pane.
    pub(super) fn reopen_held_registrations(&self) -> impl Iterator<Item = &DormantRegistration> {
        self.closed_registrations().chain(
            self.reopen_continuations
                .iter()
                .map(|continuation| &continuation.registration),
        )
    }

    /// Whether Reopen closed holds `actor`'s registration, so a watch on it
    /// is neither read nor ended while it is away.
    pub(super) fn reopen_holds_actor(&self, actor: &crate::delivery::Actor) -> bool {
        self.reopen_held_registrations()
            .any(|registration| registration.actor.same_identity(actor))
    }

    /// The registered agent a letter to `target` reaches while its pane is
    /// closed: named by its registration's id or name, or by the pane it was
    /// closed from. The letter waits with no bell, as one to a sleeper does,
    /// and the reopen's continuation carries it to the new pane; its hour
    /// runs meanwhile.
    pub(super) fn closed_target(
        &self,
        ledger: &crate::delivery::ledger::Ledger,
        registered: Option<&crate::delivery::Actor>,
        pane: &str,
    ) -> Option<super::delivery::Observation> {
        let actor = registered
            .filter(|actor| self.reopen_holds_actor(actor))
            .or_else(|| {
                ledger
                    .agents
                    .iter()
                    .filter(|record| !record.ended && record.actor.pane_id == pane)
                    .map(|record| &record.actor)
                    .find(|actor| self.reopen_holds_actor(actor))
            })?;
        Some(super::delivery::Observation {
            status: "closed".into(),
            ..super::agent_sleep::letters::asleep_observation(actor.clone())
        })
    }

    /// The ids of the registrations whose agent is closed, which read closed.
    pub(crate) fn closed_registration_ids(&self) -> std::collections::BTreeSet<String> {
        self.closed_registrations()
            .map(|registration| registration.id.clone())
            .collect()
    }

    /// A close left the queue without an item going onto the list or into
    /// its tree's entry: what it held ends with its pane, if that is gone.
    pub(super) fn release_close_registrations(&mut self, operation: &PendingClose) {
        let held = operation
            .held_registrations()
            .map(|registration| registration.id.clone())
            .collect::<Vec<_>>();
        for id in held {
            self.release_dormant_hold(&id);
        }
    }

    /// An item left the list unreopened: what it held ends with its panes.
    pub(super) fn release_item_registrations(&mut self, item: &ClosedItem) {
        let held = item
            .held_registrations()
            .into_iter()
            .map(|registration| registration.id.clone())
            .collect::<Vec<_>>();
        for id in held {
            crate::diagnostic!(serde_json::json!({
                "component": "recent_closed",
                "kind": "recent_closed.hold_released",
                "key": item.key(),
                "agent_id": id,
            }));
            self.release_dormant_hold(&id);
        }
    }

    /// A reopen consumed `item` and started `agents`: each registration the
    /// item held waits for the pane its agent's conversation resumed in, the
    /// reopen worker naming which (`ReopenedAgent::registration`), and one
    /// whose conversation did not resume has no pane to continue it and is
    /// let go. The pane takes it over only once its own session proves the
    /// same conversation (`continue_dormant_registration`).
    pub(super) fn continue_reopened_registrations(
        &mut self,
        item: &ClosedItem,
        agents: &[crate::recent_closed::reopen::ReopenedAgent],
    ) {
        let now = unix_milliseconds();
        let held = item
            .held_registrations()
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        for registration in held {
            let resumed = agents
                .iter()
                .find(|agent| agent.registration.as_ref() == Some(&registration));
            match resumed {
                Some(agent) => self.reopen_continuations.push(Continuation {
                    registration,
                    pane: agent.pane_id.clone(),
                    since_unix_ms: now,
                    seen: false,
                }),
                None => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "recent_closed",
                        "kind": "recent_closed.registration_released",
                        "key": item.key(),
                        "agent_id": registration.id,
                        "reason": "not_resumed",
                    }));
                    self.release_dormant_hold(&registration.id);
                }
            }
        }
    }

    /// Pushes a finished tree's entry held back by a close that left the
    /// queue without settling through it, and moves each reopened agent's
    /// registration to its new pane once the delivery store sees it there.
    /// Runs on the operation tick and after each session ingest; free while
    /// nothing waits.
    pub(super) fn advance_reopens(&mut self) -> bool {
        let mut changed = false;
        if self.push_ready_tree_entries() {
            self.sync_recent_closed_snapshot();
            changed = true;
        }
        if self.reopen_continuations.is_empty() {
            return changed;
        }
        let now = unix_milliseconds();
        for mut continuation in std::mem::take(&mut self.reopen_continuations) {
            let listed = self.pane_listed(&continuation.pane);
            continuation.seen |= listed;
            let step = self.take_over_registration(
                &continuation.registration,
                &continuation.pane,
                continuation.seen && !listed,
                continuation.since_unix_ms,
                now,
            );
            match step {
                super::agent_sleep::Takeover::Waiting => {
                    self.reopen_continuations.push(continuation);
                }
                super::agent_sleep::Takeover::Continued => changed = true,
                super::agent_sleep::Takeover::Released(reason) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "recent_closed",
                        "kind": "recent_closed.registration_released",
                        "agent_id": continuation.registration.id,
                        "pane_id": continuation.pane,
                        "reason": reason,
                    }));
                    changed = true;
                }
            }
        }
        changed
    }
}
