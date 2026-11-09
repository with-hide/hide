//! Which profiles are being started and which are being removed (PRD
//! agent-profiles D-11, B27; docs/ARCHITECTURE.md, Changing a machine's
//! profiles).
//!
//! A start with a profile takes a [`Hold`] under the runtime lock when it is
//! planned, before its machine is asked for the profile, and a removal is
//! planned under the same lock: each sees the other there and refuses. The
//! table has a small lock of its own, taken after the runtime's and never
//! around I/O, a node call or the runtime's lock, so a start's worker can
//! end a hold without the runtime.
//!
//! The worker ends a hold with its attempt's result, never by reading a
//! projected agent row. A definite non-start releases it immediately. A
//! start that ran or may have run hands its launch record to the runtime
//! through this same bounded table. The runtime writes the record before
//! releasing the reserved slot, under its own lock. Completion wakes the
//! existing session coordinator; every admission also takes pending results.
//! A guard hands off an unknown result on unwind after submitting a start.
//!
//! A removal marks its profile `removing` with the list version its change
//! names, and a start of a marked profile is refused. The mark goes when the
//! node definitely refused the change, or when a list newer than the version
//! the change named shows the profile gone or not being removed: a change is
//! applied only against the version it names, so a list newer than that
//! settles whether it ever will be, and an older list, read before the
//! profile existed or before a late apply, settles nothing. A change made
//! again on a moved list is checked again by the runtime before it is marked
//! anew (`Runtime::recheck_removal`).

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use hide_agent_adapter::ProfileId;

/// The starts and removals of every machine's profiles, which the runtime
/// owns and shares with the starts it plans.
#[derive(Default)]
pub(crate) struct ProfileHolds {
    table: Mutex<Table>,
    wake: Mutex<Option<crate::session_sync::RepublishWaker>>,
}

impl std::fmt::Debug for ProfileHolds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProfileHolds")
            .field("table", &self.lock())
            .finish()
    }
}

#[derive(Debug, Default)]
struct Table {
    next: u64,
    starts: BTreeMap<u64, Start>,
    /// The profiles being removed, with the list version the removal
    /// names.
    removing: BTreeMap<(String, ProfileId), u64>,
}

/// One start of an agent with a profile.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Start {
    pub(crate) machine: String,
    pub(crate) profile: ProfileId,
    /// The agent, as the start names it.
    pub(crate) kind: String,
    /// The pane the start was typed into, once it was.
    pub(crate) pane: Option<String>,
    choice: hide_agent_adapter::LaunchChoice,
    /// Submitted to the engine, and handed off once the attempt ends.
    pub(crate) record: Option<crate::model::PaneLaunch>,
    finished: bool,
}

/// One start's hold: its clones are the same start, and the last one going
/// ends a start never typed.
#[derive(Clone, Debug)]
pub(crate) struct Hold(Arc<Held>);

#[derive(Debug)]
struct Held {
    holds: Arc<ProfileHolds>,
    id: u64,
}

impl PartialEq for Hold {
    fn eq(&self, other: &Self) -> bool {
        self.0.id == other.0.id && Arc::ptr_eq(&self.0.holds, &other.0.holds)
    }
}

impl Eq for Hold {}

impl Drop for Held {
    fn drop(&mut self) {
        let mut table = self.holds.lock();
        if table
            .starts
            .get(&self.id)
            .is_some_and(|start| start.record.is_none())
        {
            table.starts.remove(&self.id);
        }
    }
}

/// Owns the handoff on every return and unwind of one start call.
pub(crate) struct Finish {
    hold: Option<Hold>,
}

impl Finish {
    pub(crate) fn new(hold: Option<Hold>) -> Self {
        Self { hold }
    }

    pub(crate) fn submitted(&self, launch: &crate::engine::AgentLaunch) {
        if let Some(hold) = &self.hold {
            let mut table = hold.0.holds.lock();
            if let Some(start) = table.starts.get_mut(&hold.0.id) {
                start.pane = Some(launch.pane_id.clone());
                start.record = Some(crate::model::PaneLaunch {
                    kind: hide_agent_adapter::canonical_kind(launch.launch.kind()).into(),
                    name: launch.name.clone(),
                    choice: start.choice.clone(),
                    session: None,
                    profile: launch.launch.record(),
                    pending: true,
                    start: hold.0.id,
                });
            }
        }
    }

    /// The engine definitely admitted no execution. A retry may submit
    /// again; leaving now releases the hold even if another clone survives.
    pub(crate) fn not_started(&self) {
        if let Some(hold) = &self.hold {
            if let Some(start) = hold.0.holds.lock().starts.get_mut(&hold.0.id) {
                start.record = None;
                start.pane = None;
            }
        }
    }
}

impl Drop for Finish {
    fn drop(&mut self) {
        let Some(hold) = &self.hold else { return };
        {
            let mut table = hold.0.holds.lock();
            if let Some(start) = table.starts.get_mut(&hold.0.id) {
                if start.record.is_some() {
                    start.finished = true;
                } else {
                    table.starts.remove(&hold.0.id);
                }
            }
        }
        if let Some(wake) = &*hold
            .0
            .holds
            .wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
        {
            wake.wake();
        }
    }
}

/// Why a start's hold was refused, as `launch::Refusal` codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Refused {
    /// The profile is being removed.
    Removing,
    /// As many launch records are kept or held as the core keeps.
    RecordsFull,
}

impl ProfileHolds {
    fn lock(&self) -> MutexGuard<'_, Table> {
        // The table holds no invariant a panicking holder could break
        // halfway: every change is one insert or removal.
        self.table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A hold for a start of `kind` with `profile` on `machine`, while
    /// `records` launch records are kept and `limit` is the most kept.
    /// Called under the runtime lock.
    pub(crate) fn hold(
        self: &Arc<Self>,
        machine: &str,
        profile: &ProfileId,
        kind: &str,
        records: usize,
        limit: usize,
        choice: &hide_agent_adapter::LaunchChoice,
    ) -> Result<Hold, Refused> {
        let mut table = self.lock();
        if table
            .removing
            .contains_key(&(machine.to_owned(), profile.clone()))
        {
            return Err(Refused::Removing);
        }
        if records + table.unrecorded() >= limit {
            return Err(Refused::RecordsFull);
        }
        table.next = table.next.checked_add(1).ok_or(Refused::RecordsFull)?;
        let id = table.next;
        table.starts.insert(
            id,
            Start {
                machine: machine.to_owned(),
                profile: profile.clone(),
                kind: kind.to_owned(),
                pane: None,
                choice: choice.clone(),
                record: None,
                finished: false,
            },
        );
        Ok(Hold(Arc::new(Held {
            holds: Arc::clone(self),
            id,
        })))
    }

    /// The launch records the starts held now still need.
    pub(crate) fn unrecorded(&self) -> usize {
        self.lock().unrecorded()
    }

    pub(crate) fn set_waker(&self, wake: crate::session_sync::RepublishWaker) {
        *self
            .wake
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(wake);
    }

    /// Orders an ordinary launch record after every start already planned.
    /// The same clock orders profiled and built-in-default starts.
    pub(crate) fn record_order(&self) -> Option<u64> {
        let mut table = self.lock();
        table.next = table.next.checked_add(1)?;
        Some(table.next)
    }

    /// Reading a result does not release its slot. The runtime releases it
    /// only after writing its record, while it still owns admission.
    pub(crate) fn completed(&self) -> Vec<(u64, Start)> {
        self.lock()
            .starts
            .iter()
            .filter(|(_, start)| start.finished)
            .map(|(id, start)| (*id, start.clone()))
            .collect()
    }

    pub(crate) fn release(&self, id: u64) {
        self.lock().starts.remove(&id);
    }

    pub(crate) fn forget(&self, machine: &str) {
        let mut table = self.lock();
        table.starts.retain(|_, start| start.machine != machine);
        table.removing.retain(|(on, _), _| on != machine);
    }

    /// The starts of `profile` on `machine` held now.
    pub(crate) fn starting(&self, machine: &str, profile: &ProfileId) -> Vec<Start> {
        self.lock()
            .starts
            .values()
            .filter(|start| start.machine == machine && start.profile == *profile)
            .cloned()
            .collect()
    }

    /// Marks `profile` on `machine` as being removed by a change that names
    /// the list version `version`, refused while a start holds it.
    pub(crate) fn remove(
        &self,
        machine: &str,
        profile: &ProfileId,
        version: u64,
    ) -> Result<(), Vec<Start>> {
        let mut table = self.lock();
        let starting: Vec<Start> = table
            .starts
            .values()
            .filter(|start| start.machine == machine && start.profile == *profile)
            .cloned()
            .collect();
        if !starting.is_empty() {
            return Err(starting);
        }
        table
            .removing
            .insert((machine.to_owned(), profile.clone()), version);
        Ok(())
    }

    /// The removal of `profile` was definitely not made.
    pub(crate) fn not_removed(&self, machine: &str, profile: &ProfileId) {
        self.lock()
            .removing
            .remove(&(machine.to_owned(), profile.clone()));
    }

    /// Whether `profile` on `machine` is being removed.
    #[cfg(test)]
    pub(crate) fn is_removing(&self, machine: &str, profile: &ProfileId) -> bool {
        self.lock()
            .removing
            .contains_key(&(machine.to_owned(), profile.clone()))
    }

    /// What `machine`'s list at `version` settles of its removals: one whose
    /// profile it no longer lists, or lists as not being removed, in a
    /// version newer than the one the removal named, is over. A list no
    /// newer than that may have been read before the profile existed, or
    /// before the removal applied, so it settles nothing.
    pub(crate) fn listed(
        &self,
        machine: &str,
        version: u64,
        removing: impl Fn(&ProfileId) -> Option<bool>,
    ) {
        self.lock().removing.retain(|(on, profile), named| {
            on != machine || removing(profile) == Some(true) || version <= *named
        });
    }
}

impl Table {
    fn unrecorded(&self) -> usize {
        self.starts.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn id(name: &str) -> ProfileId {
        ProfileId(name.into())
    }
    #[test]
    fn a_removal_and_a_start_of_one_profile_refuse_each_other() {
        let holds = Arc::new(ProfileHolds::default());
        let hold = holds
            .hold("m", &id("p"), "claude", 0, 8, &Default::default())
            .unwrap();
        assert_eq!(holds.remove("m", &id("p"), 3).unwrap_err().len(), 1);
        // Another machine's profile of the same id is another profile.
        assert!(holds.remove("n", &id("p"), 3).is_ok());
        drop(hold);
        assert!(holds.remove("m", &id("p"), 3).is_ok());
        assert_eq!(
            holds
                .hold("m", &id("p"), "claude", 0, 8, &Default::default())
                .unwrap_err(),
            Refused::Removing
        );
    }

    #[test]
    fn only_a_list_newer_than_the_removal_or_without_the_profile_ends_it() {
        let holds = ProfileHolds::default();
        holds.remove("m", &id("p"), 5).unwrap();
        // A list read before the change applied, at its own version, or
        // before the profile existed.
        holds.listed("m", 5, |_| Some(false));
        holds.listed("m", 4, |_| None);
        assert!(holds.is_removing("m", &id("p")));
        // The change applied late: the profile is being removed.
        holds.listed("m", 6, |_| Some(true));
        assert!(holds.is_removing("m", &id("p")));
        // Its removal finished.
        holds.listed("m", 7, |_| None);
        assert!(!holds.is_removing("m", &id("p")));

        // A removal that never applied: a newer list settles it.
        holds.remove("m", &id("q"), 5).unwrap();
        holds.listed("m", 6, |_| Some(false));
        assert!(!holds.is_removing("m", &id("q")));
    }
}
