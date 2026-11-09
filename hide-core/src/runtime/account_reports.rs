//! The account a pane's agent runs in, as its hook reported it at the
//! session's start (PRD agent-profiles D-03, D-35, B24, B25;
//! docs/ARCHITECTURE.md, Agent profiles and accounts).
//!
//! An agent the operator starts by hand with an alias that sets a profile's
//! account variable runs in that account; so does one whose program (a
//! wrapper) sets another account than its profile's. The hook in that
//! account reports the variables it runs with, and the machine's node lists
//! the variables each of its profiles sets, both compared as the same
//! function spells them (`hide_agent_hooks::account::compared_folder`). The
//! core only compares the two: it never looks at a path, which may be a
//! connected device's.
//!
//! The answer is kept per pane and worked out again only when a report
//! moves, a machine's list comes in, or the saved facts are installed, so a
//! projection reads one map entry a row and nothing more.

use std::collections::{BTreeMap, BTreeSet};

use hide_agent_adapter::{AccountKey, ProfileId};

use super::Runtime;

/// The account one pane's agent runs in, as its hook reported it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReportedAccount {
    /// The agent's adapter id.
    pub kind: String,
    pub session: Option<String>,
    pub key: AccountKey,
    /// The one profile of the machine in that account, which the row names
    /// (B24); none for the default account and for one several profiles
    /// share, whose row shows the agent's name alone.
    pub named: Option<(ProfileId, String)>,
    /// The profile a start in that account takes again (a wake, a reopen):
    /// the agent's default among several, then the list's order; none for
    /// the default account.
    pub start: Option<(ProfileId, String)>,
}

/// The adapter id of `kind`, in any of its spellings.
fn agent_id(kind: &str) -> Option<&'static str> {
    hide_agent_adapter::adapter(kind).map(|adapter| adapter.id)
}

impl Runtime {
    /// The account the `kind` agent in `pane` runs in, when its hook
    /// reported one the machine keeps.
    pub(crate) fn reported_account(&self, pane: &str, kind: &str) -> Option<&ReportedAccount> {
        self.reported_accounts
            .get(pane)
            .filter(|reported| Some(reported.kind.as_str()) == agent_id(kind))
    }

    /// Works out every pane's reported account again from the hook facts and
    /// the machines' lists, and which profiles' last start ran in another
    /// account (D-35); lays the warnings on the lists. Whether anything a
    /// row or a list shows moved.
    pub(crate) fn refresh_reported_accounts(&mut self) -> bool {
        let reported: Vec<(String, ReportedAccount)> = {
            let facts = self
                .hook_facts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            facts
                .reported_accounts()
                .filter_map(|report| {
                    let kind = agent_id(report.runtime)?;
                    let device =
                        crate::address::pane_device(report.pane).unwrap_or(self.node.as_str());
                    let account = self.account_reported(device, kind, report.variables)?;
                    Some((
                        report.pane.to_owned(),
                        ReportedAccount {
                            kind: kind.to_owned(),
                            session: report.session.map(str::to_owned),
                            ..account
                        },
                    ))
                })
                .collect()
        };
        let reported: BTreeMap<String, ReportedAccount> = reported.into_iter().collect();
        // A start Hide made with a profile whose hook reports another
        // account: its program set a folder of its own. A start that reports
        // its own account again lifts the warning.
        let mut wrapper = self.wrapper_folders.clone();
        for (pane, account) in &reported {
            let Some(profile) = self
                .snapshot
                .ui_state
                .pane_launches
                .get(pane)
                .filter(|launch| agent_id(&launch.kind) == Some(account.kind.as_str()))
                .filter(|launch| {
                    launch.session.is_none()
                        || account.session.is_none()
                        || launch.session == account.session
                })
                .and_then(|launch| launch.profile.as_ref())
            else {
                continue;
            };
            let device = crate::address::pane_device(pane).unwrap_or(self.node.as_str());
            let at = (device.to_owned(), profile.id.0.clone());
            if profile.account == account.key {
                wrapper.remove(&at);
            } else {
                wrapper.insert(at);
            }
        }
        let rows_moved = reported != self.reported_accounts;
        if rows_moved {
            // The rows of each machine whose panes moved are laid again.
            let machines: BTreeSet<Option<String>> = reported
                .iter()
                .filter(|(pane, account)| self.reported_accounts.get(*pane) != Some(account))
                .map(|(pane, _)| pane)
                .chain(
                    self.reported_accounts
                        .keys()
                        .filter(|pane| !reported.contains_key(*pane)),
                )
                .map(|pane| crate::address::pane_device(pane).map(str::to_owned))
                .collect();
            for machine in machines {
                match machine {
                    None => self.request_republish(),
                    Some(device) => {
                        if let Some(sync) = self
                            .remote_connections
                            .get(&device)
                            .and_then(|connection| connection.sync.as_ref())
                        {
                            sync.republish_waker().wake();
                        }
                    }
                }
            }
        }
        self.reported_accounts = reported;
        let warned_moved = wrapper != self.wrapper_folders;
        self.wrapper_folders = wrapper;
        // A list that came in since has none of the marks yet.
        let mut lists_moved = false;
        let devices: Vec<String> = self.kit_states.keys().cloned().collect();
        for device in devices {
            lists_moved |= self.lay_wrapper_folders(&device);
        }
        rows_moved || warned_moved || lists_moved
    }

    /// Marks each profile of `device`'s list whose last start ran in another
    /// account. Whether the list moved.
    pub(crate) fn lay_wrapper_folders(&mut self, device: &str) -> bool {
        let warned: BTreeSet<&str> = self
            .wrapper_folders
            .iter()
            .filter(|(machine, _)| machine == device)
            .map(|(_, id)| id.as_str())
            .collect();
        let Some(mut state) = self.kit_states.get(device).cloned() else {
            return false;
        };
        let Some(profiles) = state.profiles.as_mut() else {
            return false;
        };
        let mut moved = false;
        for profile in &mut profiles.profiles {
            let warn = warned.contains(profile.id.as_str());
            moved |= profile.wrapper_folder != warn;
            profile.wrapper_folder = warn;
        }
        moved && self.set_kit_state(device, state)
    }

    /// The account on `device` whose variables are `variables`, as the
    /// machine lists its profiles' (`ProfileSnapshot::account_variables`):
    /// the default account for none; `None` for variables no profile of the
    /// machine sets, an account Hide keeps nothing of (an unregistered
    /// folder, or the node's own default the variable already names).
    fn account_reported(
        &self,
        device: &str,
        kind: &str,
        variables: &[(String, String)],
    ) -> Option<ReportedAccount> {
        let blank = |key| ReportedAccount {
            kind: kind.to_owned(),
            session: None,
            key,
            named: None,
            start: None,
        };
        if variables.is_empty() {
            return Some(blank(AccountKey::Default));
        }
        let profiles = self.machine_profiles(device)?;
        let mut wanted = variables.to_vec();
        wanted.sort();
        let default = profiles.defaults.get(kind);
        let mut matches: Vec<&crate::model::ProfileSnapshot> = profiles
            .profiles
            .iter()
            .filter(|profile| {
                agent_id(&profile.kind) == Some(kind)
                    && !matches!(
                        profile.install,
                        Some(
                            crate::model::ProfileInstall::Removing
                                | crate::model::ProfileInstall::RemoveFailed
                        )
                    )
                    && !profile.account_variables.is_empty()
                    && {
                        let mut listed = profile.account_variables.clone();
                        listed.sort();
                        listed == wanted
                    }
            })
            .collect();
        matches.sort_by_key(|profile| Some(&profile.id) != default);
        let first = matches.first()?;
        let pair = |profile: &crate::model::ProfileSnapshot| {
            (ProfileId(profile.id.clone()), profile.name.clone())
        };
        Some(ReportedAccount {
            named: (matches.len() == 1).then(|| pair(first)),
            start: Some(pair(first)),
            ..blank(first.account_key.clone())
        })
    }
}
