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
//! A start's hold lives through two stages:
//!
//! - **Resolving**, from the plan until the start is typed into its pane:
//!   the last clone of its [`Hold`] going ends it, since nothing ran. A
//!   start refused before typing, or whose attempt failed, ends so.
//! - **Typed** ([`Hold::typed`], by `agent_start::start_within` once the
//!   engine answered a start that may have run): it no longer ends with the
//!   start's values. It ends when the runtime sees the agent row the start
//!   made, described by a launch record naming the profile, which carries
//!   the use from then on, or when the pane is gone from the topology a pass
//!   owns (`Runtime::settle_profile_holds`). A start nothing confirmed keeps
//!   it, as one that may run.
//!
//! Through both it keeps a slot of the launch records: a start counts
//! against `PANE_LAUNCH_LIMIT` from its plan until its record is written, so
//! starts planned together never take more records than there are.
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
#[derive(Debug, Default)]
pub(crate) struct ProfileHolds {
    table: Mutex<Table>,
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
    /// Its launch record was written, so it holds no slot of its own.
    pub(crate) recorded: bool,
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
            .is_some_and(|start| start.pane.is_none())
        {
            table.starts.remove(&self.id);
        }
    }
}

impl Hold {
    /// The start was typed into `pane`: it may run from now.
    pub(crate) fn typed(&self, pane: &str) {
        if let Some(start) = self.0.holds.lock().starts.get_mut(&self.0.id) {
            start.pane = Some(pane.to_owned());
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
        table.next += 1;
        let id = table.next;
        table.starts.insert(
            id,
            Start {
                machine: machine.to_owned(),
                profile: profile.clone(),
                kind: kind.to_owned(),
                pane: None,
                recorded: false,
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

    /// The start typed into `pane` of `machine` with `profile` has its
    /// record now.
    pub(crate) fn recorded(&self, machine: &str, pane: &str, profile: &ProfileId) {
        let mut table = self.lock();
        for start in table.starts.values_mut() {
            if start.machine == machine
                && start.pane.as_deref() == Some(pane)
                && start.profile == *profile
            {
                start.recorded = true;
            }
        }
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

    /// Ends the typed starts `ended` says are over; called under the
    /// runtime lock with what it knows of their panes.
    pub(crate) fn settle(&self, ended: impl Fn(&Start, &str) -> bool) -> usize {
        let mut table = self.lock();
        let before = table.starts.len();
        table.starts.retain(|_, start| match start.pane.as_deref() {
            Some(pane) => !ended(start, pane),
            None => true,
        });
        before - table.starts.len()
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
        self.starts.values().filter(|start| !start.recorded).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(name: &str) -> ProfileId {
        ProfileId(name.to_owned())
    }

    #[test]
    fn a_start_never_typed_ends_with_its_last_clone_and_a_typed_one_stays() {
        let holds = Arc::new(ProfileHolds::default());
        let hold = holds.hold("m", &id("p"), "claude", 0, 8).unwrap();
        let clone = hold.clone();
        drop(hold);
        assert_eq!(holds.starting("m", &id("p")).len(), 1);
        drop(clone);
        assert!(holds.starting("m", &id("p")).is_empty());

        let typed = holds.hold("m", &id("p"), "claude", 0, 8).unwrap();
        typed.typed("m:p1");
        drop(typed);
        assert_eq!(holds.starting("m", &id("p")).len(), 1);
    }

    #[test]
    fn a_removal_and_a_start_of_one_profile_refuse_each_other() {
        let holds = Arc::new(ProfileHolds::default());
        let hold = holds.hold("m", &id("p"), "claude", 0, 8).unwrap();
        assert_eq!(holds.remove("m", &id("p"), 3).unwrap_err().len(), 1);
        // Another machine's profile of the same id is another profile.
        assert!(holds.remove("n", &id("p"), 3).is_ok());
        drop(hold);
        assert!(holds.remove("m", &id("p"), 3).is_ok());
        assert_eq!(
            holds.hold("m", &id("p"), "claude", 0, 8).unwrap_err(),
            Refused::Removing
        );
    }

    #[test]
    fn starts_planned_together_never_take_more_records_than_there_are() {
        let holds = Arc::new(ProfileHolds::default());
        // Two records kept of four: two more starts fit, a third does not.
        let first = holds.hold("m", &id("a"), "claude", 2, 4).unwrap();
        let second = holds.hold("m", &id("b"), "claude", 2, 4).unwrap();
        assert_eq!(
            holds.hold("m", &id("c"), "claude", 2, 4).unwrap_err(),
            Refused::RecordsFull
        );
        // A start whose record is written is counted among the records.
        first.typed("m:p1");
        holds.recorded("m", "m:p1", &id("a"));
        assert_eq!(holds.unrecorded(), 1);
        assert!(holds.hold("m", &id("c"), "claude", 3, 4).is_err());
        drop(second);
        assert!(holds.hold("m", &id("c"), "claude", 3, 4).is_ok());
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
