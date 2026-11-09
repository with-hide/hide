//! The agent kind and model a start names, the words they become on the
//! agent's command line, and the choice every start surface preselects next
//! (PRD home-device-rail D-18, D-20).
//!
//! Every start (⌘N, New worktree, an issue's Start, 맡기기, the phone) goes
//! through here, so a pick on any of them is the next default on all of
//! them, and a model reaches the CLI the same way wherever it was chosen.
//!
//! The model and effort a pane's agent was started with, and the profile it
//! runs with (PRD agent-profiles D-27), are kept here too
//! (`ui_state.pane_launches`), for the conversation it was started for, so
//! every restart Hide makes of it (a wake, a fork, a reopen, a resume) starts
//! it with the same choice and profile instead of the CLI's configured
//! default.

use super::*;
use hide_agent_adapter::AccountKey;

/// The model a start names, trimmed, with an empty one meaning the CLI's own
/// default. It is checked by the adapter's one rule: a kind that starts no
/// agent (`terminal`) or whose CLI declares no model takes none, and a name
/// must be one `hide_agent_adapter::valid_model` admits; the CLI itself
/// judges whether it knows it (D-18: a model the CLI refuses fails on its
/// pane, never silently swapped).
pub(crate) fn chosen_model(
    agent_kind: Option<&str>,
    model: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(model) = model.map(str::trim).filter(|model| !model.is_empty()) else {
        return Ok(None);
    };
    let Some(kind) = agent_kind else {
        return Err("A terminal starts no agent, so it takes no model".into());
    };
    let choice = hide_agent_adapter::LaunchChoice {
        model: Some(model.to_owned()),
        effort: None,
    };
    choice_arguments(kind, &choice).map_err(|reason| format!("{model}: {reason}"))?;
    Ok(Some(model.to_owned()))
}

/// The agent CLI's own arguments for a new start: the permission choice of
/// the machine it starts on (`agent_permission`), then the model a
/// `chosen_model` admitted, in the form the kind's adapter declares. None for
/// a kind that starts no agent.
pub(crate) fn agent_arguments(
    kind: Option<&str>,
    model: Option<&str>,
    permission: &crate::agent_permission::AgentPermission,
) -> Vec<String> {
    let Some(kind) = kind else {
        return Vec::new();
    };
    let Some(dialect) = hide_agent_adapter::adapter(kind).and_then(|row| row.start) else {
        return Vec::new();
    };
    let mut args = crate::agent_permission::arguments(kind, permission);
    if let Some(model) = model {
        args.extend(
            dialect
                .options()
                .model
                .iter()
                .map(|part| part.replace("{}", model)),
        );
    }
    args
}

/// The arguments a restart of a `kind` agent adds after its resume or fork
/// arguments: the machine's permission choice as it is now, then the model
/// and effort it was started with (none for the CLI's defaults). A saved
/// choice the agent does not take (a store edited by hand) is refused with
/// the reason rather than dropped, so the restart says so.
pub(crate) fn launch_arguments(
    kind: &str,
    choice: &hide_agent_adapter::LaunchChoice,
    permission: &crate::agent_permission::AgentPermission,
) -> Result<Vec<String>, String> {
    let mut args = crate::agent_permission::arguments(kind, permission);
    args.extend(choice_arguments(kind, choice)?);
    Ok(args)
}

/// The model and effort part of [`launch_arguments`], for a start that
/// reads the machine's permission choice later (`hide agent spawn`, whose
/// device is known only once its context is).
pub(crate) fn choice_arguments(
    kind: &str,
    choice: &hide_agent_adapter::LaunchChoice,
) -> Result<Vec<String>, String> {
    if choice.is_default() {
        return Ok(Vec::new());
    }
    hide_agent_adapter::adapter(kind)
        .and_then(|row| row.start)
        .ok_or_else(|| format!("{kind} takes no model or effort"))?
        .choice_arguments(choice)
}

/// The most launch records the core keeps. A record lives only while its pane
/// is in its machine's topology and the machine is registered, so the map
/// holds at most one per such pane. A start with a profile holds its slot
/// from its plan (`launch::holds`) and is refused when none is left; a start
/// on the built-in default past this many is recorded nowhere and logged
/// (`pane_launch.limit`): its restarts run on the CLI's defaults, and a child
/// it starts cannot know its account, so picks as a start with no parent
/// does.
pub(crate) const PANE_LAUNCH_LIMIT: usize = 1024;

/// Whether `launch` describes `agent`: the same kind, and the name Hide
/// started it under, or, for an agent that came back from an engine restart
/// without a name, the conversation that named agent last reported.
fn describes(launch: &crate::model::PaneLaunch, agent: &SidebarAgentSnapshot) -> bool {
    launch_describes(
        launch,
        &agent.agent_kind,
        agent.started_name.as_deref(),
        agent.session_id.as_deref(),
    )
}

/// Whether `launch` describes an agent of `kind` started under `name` (none
/// for one that came back without its name) whose conversation is
/// `session`: the one rule a pane's row and the own engine's resume of a
/// saved agent both read a launch record by (PRD engine-bundle-4-herdr-removal
/// D-24).
pub(crate) fn launch_describes(
    launch: &crate::model::PaneLaunch,
    kind: &str,
    name: Option<&str>,
    session: Option<&str>,
) -> bool {
    hide_agent_adapter::canonical_kind(kind) == launch.kind
        && match name {
            Some(name) => name == launch.name,
            None => launch.session.is_some() && launch.session.as_deref() == session,
        }
}

/// Follows each pane's launch record to the conversation its named agent
/// reports, and drops a record whose pane hosts another agent (another
/// kind, another name, or, unnamed, another conversation than the named one
/// had) or is gone from the authoritative topology this pass owns. A row of
/// no known kind (a restored server still detecting it) decides nothing.
/// Returns whether a record moved.
pub(crate) fn settle_pane_launches(
    launches: &mut BTreeMap<String, crate::model::PaneLaunch>,
    agents: &[SidebarAgentSnapshot],
    live_pane_ids: Option<&HashSet<String>>,
    scope: crate::agent_state::ReadRecordScope<'_>,
) -> bool {
    if launches.is_empty() {
        return false;
    }
    let count = launches.len();
    if let Some(live_pane_ids) = live_pane_ids {
        launches.retain(|pane_id, _| !scope.owns(pane_id) || live_pane_ids.contains(pane_id));
    }
    let mut changed = launches.len() != count;
    for agent in agents {
        let Some(launch) = launches.get_mut(&agent.pane_id) else {
            continue;
        };
        if hide_agent_adapter::adapter(&agent.agent_kind).is_none() {
            continue;
        }
        if describes(launch, agent) {
            if agent.session_id.is_some() && agent.session_id != launch.session {
                launch.session = agent.session_id.clone();
                changed = true;
            }
            continue;
        }
        let another = hide_agent_adapter::canonical_kind(&agent.agent_kind) != launch.kind
            || agent.started_name.is_some()
            || (launch.session.is_some() && agent.session_id.is_some());
        if another {
            launches.remove(&agent.pane_id);
            changed = true;
        }
    }
    changed
}

impl Runtime {
    /// Records the choice and profile Hide started a `kind` agent named
    /// `name` with in `pane_id` (the core's id for the pane), replacing what
    /// the pane held, the built-in default profile on the CLI's defaults
    /// included: a record with no profile is what tells a child its parent
    /// runs with the built-in default (D-32). `session` is the conversation
    /// a restart resumed there, which the record then already describes.
    pub(crate) fn record_pane_launch(
        &mut self,
        pane_id: &str,
        kind: &str,
        name: &str,
        choice: &hide_agent_adapter::LaunchChoice,
        profile: Option<&crate::launch::LaunchProfile>,
        session: Option<&str>,
    ) {
        if self.put_pane_launch(pane_id, kind, name, choice, profile, session) {
            self.persist_ui_state();
        }
    }

    /// `record_pane_launch` for a caller that saves the state itself;
    /// returns whether the record moved.
    pub(super) fn put_pane_launch(
        &mut self,
        pane_id: &str,
        kind: &str,
        name: &str,
        choice: &hide_agent_adapter::LaunchChoice,
        profile: Option<&crate::launch::LaunchProfile>,
        session: Option<&str>,
    ) -> bool {
        let moved = self.write_pane_launch(pane_id, kind, name, choice, profile, session);
        if moved {
            self.missions.launches_edit = self.missions.launches_edit.wrapping_add(1);
        }
        moved
    }

    fn write_pane_launch(
        &mut self,
        pane_id: &str,
        kind: &str,
        name: &str,
        choice: &hide_agent_adapter::LaunchChoice,
        profile: Option<&crate::launch::LaunchProfile>,
        session: Option<&str>,
    ) -> bool {
        // A start with a profile took its slot when it was planned; one on
        // the built-in default takes only a slot no such start holds.
        let held = if profile.is_some() {
            0
        } else {
            self.profile_holds.unrecorded()
        };
        let launches = &mut self.snapshot.ui_state.pane_launches;
        if !launches.contains_key(pane_id) && launches.len() + held >= PANE_LAUNCH_LIMIT {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_choice", "kind": "pane_launch.limit",
                "pane_id": pane_id, "limit": PANE_LAUNCH_LIMIT,
            }));
            false
        } else {
            let launch = crate::model::PaneLaunch {
                kind: hide_agent_adapter::canonical_kind(kind).to_owned(),
                name: name.to_owned(),
                choice: choice.clone(),
                session: session.map(str::to_owned),
                profile: profile.cloned(),
            };
            let moved = launches.insert(pane_id.to_owned(), launch.clone()) != Some(launch);
            if let Some(profile) = profile {
                let (machine, pane) = crate::address::split_pane(self.node.as_str(), pane_id);
                self.profile_holds.recorded(machine, pane, &profile.id);
            }
            moved
        }
    }

    /// The agent profiles of the machine `device` as its node last listed
    /// them; `None` until they were read.
    pub(crate) fn machine_profiles(&self, device: &str) -> Option<&crate::model::ProfilesSnapshot> {
        self.kit_states.get(device)?.profiles.as_ref()
    }

    /// The profile a new start of `kind` on `device` with no parent runs
    /// with (D-32): the one the start panel's `▾` chose (`chosen`, by id),
    /// else the default of the project at `root` that Project settings
    /// choose, else the agent's default there; `None` for the built-in
    /// default and for a start of no agent. A chosen profile the machine
    /// does not list is refused (B16).
    pub(crate) fn new_start_profile(
        &self,
        device: &str,
        kind: Option<&str>,
        root: Option<&str>,
        chosen: Option<&str>,
    ) -> Result<Option<(hide_agent_adapter::ProfileId, String)>, String> {
        let Some(kind) = kind else {
            return Ok(None);
        };
        let profiles = self.machine_profiles(device);
        let chosen = chosen
            .map(|id| crate::launch::chosen_id(kind, id, profiles))
            .transpose()?;
        let project = root.and_then(|root| self.project_agent_profile(device, root, kind));
        Ok(crate::launch::pick(
            kind,
            chosen,
            &crate::launch::Parent::None,
            project.as_ref(),
            profiles,
        )
        .profile())
    }

    /// The profile Project settings choose for `kind` in the project that
    /// holds `path` on `device` (D-31): its root, or one of its worktrees.
    pub(crate) fn project_agent_profile(
        &self,
        device: &str,
        path: &str,
        kind: &str,
    ) -> Option<hide_agent_adapter::ProfileId> {
        let agent = hide_agent_adapter::adapter(kind)?.id;
        let root = self
            .catalog_project_of(device, path)
            .map_or(path, |project| project.root_path.as_str());
        crate::project_settings::agent_profile(
            &self.snapshot.ui_state.project_settings,
            device,
            root,
            agent,
        )
        .map(|id| hide_agent_adapter::ProfileId(id.to_owned()))
    }

    /// The profile a resume of a `kind` session on `device` runs with when
    /// the session's start recorded none (a session line or record): the
    /// built-in default for a session in the agent's default account, as for
    /// every session recorded before profiles; else the profile of that agent
    /// there whose account is `account`, the agent's default among several
    /// first, then the list's order (PRD agent-profiles B19). An account no
    /// profile names now (its profile was removed) starts nothing, so a
    /// session is never resumed in an account other than its own.
    pub(crate) fn session_profile(
        &self,
        device: &str,
        kind: &str,
        account: &hide_agent_adapter::AccountKey,
    ) -> Result<Option<(hide_agent_adapter::ProfileId, String)>, &'static str> {
        let spelled = match account {
            hide_agent_adapter::AccountKey::Default => return Ok(None),
            hide_agent_adapter::AccountKey::Folder(spelled)
            | hide_agent_adapter::AccountKey::Name(spelled) => spelled,
        };
        let agent = hide_agent_adapter::adapter(kind)
            .ok_or(crate::launch::PROFILE_GONE)?
            .id;
        let profiles = self
            .machine_profiles(device)
            .ok_or(crate::launch::PROFILE_GONE)?;
        let default = profiles.defaults.get(agent);
        profiles
            .profiles
            .iter()
            .filter(|profile| {
                profile.kind == agent
                    && profile.install != Some(crate::model::ProfileInstall::Removing)
                    && profile.account.as_deref() == Some(spelled.as_str())
            })
            .min_by_key(|profile| Some(&profile.id) != default)
            .map(|profile| {
                Some((
                    hide_agent_adapter::ProfileId(profile.id.clone()),
                    profile.name.clone(),
                ))
            })
            .ok_or(crate::launch::PROFILE_GONE)
    }

    /// The profile `agent` runs with, when its pane's record describes it;
    /// otherwise the built-in default (`None`).
    ///
    /// An agent whose hook reported another account than its record names,
    /// or that Hide did not start, runs with the profile of the account it
    /// reported (D-03, D-35): that is the account a wake or a reopen starts
    /// it in again, with the agent's default profile there among several.
    pub(crate) fn launch_profile_of(
        &self,
        agent: &SidebarAgentSnapshot,
    ) -> Option<crate::launch::LaunchProfile> {
        let recorded = self
            .snapshot
            .ui_state
            .pane_launches
            .get(&agent.pane_id)
            .filter(|launch| describes(launch, agent))
            .and_then(|launch| launch.profile.clone());
        let Some(reported) = self.reported_account(&agent.pane_id, &agent.agent_kind) else {
            return recorded;
        };
        if recorded
            .as_ref()
            .map_or(AccountKey::Default, |p| p.account.clone())
            == reported.key
        {
            return recorded;
        }
        reported
            .start
            .as_ref()
            .map(|(id, name)| crate::launch::LaunchProfile {
                id: id.clone(),
                name: name.clone(),
                account: reported.key.clone(),
            })
    }

    /// The account the `kind` agent started under `name` (none for one that
    /// came back without it) whose conversation is `session` reads its
    /// sessions in, in the pane the core calls `pane_id`: its profile's,
    /// when the pane's record describes it, else the default account (PRD
    /// agent-profiles B19). Every read of one agent's session names it, and
    /// the node reads in that account alone.
    pub(crate) fn pane_account(
        &self,
        pane_id: &str,
        kind: &str,
        name: Option<&str>,
        session: Option<&str>,
    ) -> hide_agent_adapter::AccountKey {
        // The account its hook reported is the one it runs in, whatever
        // started it (D-03, D-35).
        if let Some(reported) = self.reported_account(pane_id, kind) {
            return reported.key.clone();
        }
        self.snapshot
            .ui_state
            .pane_launches
            .get(pane_id)
            .filter(|launch| launch_describes(launch, kind, name, session))
            .and_then(|launch| launch.profile.as_ref())
            .map(|profile| profile.account.clone())
            .unwrap_or_default()
    }

    /// What the label reads of the panes on `device` (`None` for this
    /// machine) need to name each one's account, by that machine's own pane
    /// ids: the launch records whose agent runs in an account other than
    /// the default, and the account each pane's hook reported (B24, B25),
    /// with the edits they were taken at; `None` while neither moved since
    /// `edit`. The reads of one machine's coordinator name their accounts by
    /// them, and take them again only when one moved.
    pub(crate) fn account_launches(
        &self,
        device: Option<&str>,
        edit: Option<(u64, u64)>,
    ) -> Option<((u64, u64), AccountReads)> {
        let current = (self.missions.launches_edit, self.reported_edit);
        if edit == Some(current) {
            return None;
        }
        let own = |pane: &str| -> Option<String> {
            match device {
                Some(device) => crate::address::pane_of(pane, device).map(str::to_owned),
                None if crate::address::is_device_scoped(pane) => None,
                None => Some(pane.to_owned()),
            }
        };
        let launches = self
            .snapshot
            .ui_state
            .pane_launches
            .iter()
            .filter(|(_, launch)| {
                launch
                    .profile
                    .as_ref()
                    .is_some_and(|profile| !profile.account.is_default())
            })
            .filter_map(|(pane, launch)| Some((own(pane)?, launch.clone())))
            .collect();
        let reported = self
            .reported_accounts
            .iter()
            .filter_map(|(pane, account)| {
                Some((own(pane)?, (account.kind.clone(), account.key.clone())))
            })
            .collect();
        Some((current, AccountReads { launches, reported }))
    }

    /// What the agent `actor` hands a child of `kind` it starts on its own
    /// machine (D-32): the profile it runs with, the built-in default
    /// included, when it is an agent of `kind` and its pane's record
    /// describes it; nothing for another agent; and, for one of `kind`
    /// whose start Hide has no record of that describes it (started by
    /// hand, recorded before this rule, or past the record limit), that its
    /// account is unknown, which picks as no parent does.
    pub(crate) fn caller_launch_profile(
        &self,
        actor: &crate::delivery::Actor,
        kind: &str,
    ) -> crate::launch::Parent {
        if hide_agent_adapter::canonical_kind(&actor.kind)
            != hide_agent_adapter::canonical_kind(kind)
        {
            return crate::launch::Parent::None;
        }
        let record = self.snapshot.ui_state.pane_launches.get(&actor.pane_id);
        let described = record.filter(|launch| {
            launch_describes(
                launch,
                &actor.kind,
                Some(&actor.name),
                actor.session.as_deref(),
            ) || launch_describes(launch, &actor.kind, None, actor.session.as_deref())
        });
        match described {
            Some(launch) => crate::launch::Parent::recorded(launch.profile.clone()),
            None => {
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_choice",
                    "kind": "agent_profile.parent_unknown",
                    "pane_id": actor.pane_id,
                    "agent": hide_agent_adapter::canonical_kind(kind),
                    "cause": if record.is_some() { "record_mismatch" } else { "no_record" },
                }));
                crate::launch::Parent::Unknown
            }
        }
    }

    /// The choice `agent` was started with, when its pane's record describes
    /// it; otherwise the CLI's defaults.
    pub(crate) fn launch_choice_of(
        &self,
        agent: &SidebarAgentSnapshot,
    ) -> hide_agent_adapter::LaunchChoice {
        self.snapshot
            .ui_state
            .pane_launches
            .get(&agent.pane_id)
            .filter(|launch| describes(launch, agent))
            .map(|launch| launch.choice.clone())
            .unwrap_or_default()
    }

    /// Forgets the launch records of a device's panes once the device is
    /// removed: no pass of its own will settle them again.
    pub(super) fn forget_device_launches(&mut self, device: &str) {
        let launches = &mut self.snapshot.ui_state.pane_launches;
        let count = launches.len();
        launches.retain(|pane_id, _| !crate::address::is_pane_of(pane_id, device));
        if launches.len() != count {
            self.missions.launches_edit = self.missions.launches_edit.wrapping_add(1);
            self.persist_ui_state();
        }
    }

    /// Settles the launch records against one pass's agent rows, saves
    /// them when one moved, and lays each row's profile from them
    /// ([`Self::lay_profiles`]).
    pub(super) fn apply_pane_launches(
        &mut self,
        agents: &mut [SidebarAgentSnapshot],
        scope: crate::agent_state::ReadRecordScope<'_>,
        live_pane_ids: Option<&HashSet<String>>,
    ) {
        if settle_pane_launches(
            &mut self.snapshot.ui_state.pane_launches,
            agents,
            live_pane_ids,
            scope,
        ) {
            self.missions.launches_edit = self.missions.launches_edit.wrapping_add(1);
            self.persist_ui_state();
        }
        self.settle_profile_holds(agents, scope, live_pane_ids);
        // A mission worker's launch is proven once its record names the
        // worker's conversation (a settle, a wake or a resume writes it) and
        // the worker is bound to its node.
        self.record_node_launches();
        self.lay_profiles(agents);
    }

    /// The profile each of `records`' live agents runs with, by registration
    /// id, as its row names it (`SidebarAgentSnapshot::profile`): what
    /// `hide agent list` and `hide agent show` say beside the agent.
    pub(crate) fn registration_profiles<'a>(
        &self,
        records: impl IntoIterator<Item = &'a crate::coordination::AgentRecord>,
    ) -> BTreeMap<String, String> {
        let rows: HashMap<&str, &SidebarAgentSnapshot> = self
            .agent_rows()
            .filter(|row| row.profile.is_some())
            .map(|row| (row.pane_id.as_str(), row))
            .collect();
        if rows.is_empty() {
            return BTreeMap::new();
        }
        records
            .into_iter()
            .filter(|record| !record.ended)
            .filter_map(|record| {
                let row = rows.get(record.actor.pane_id.as_str())?;
                (hide_agent_adapter::canonical_kind(&row.agent_kind)
                    == hide_agent_adapter::canonical_kind(&record.actor.kind))
                .then(|| {
                    Some((
                        record.id.clone(),
                        crate::launch::shown(&row.agent_kind, row.profile.as_deref()?),
                    ))
                })?
            })
            .collect()
    }

    /// Names on each row the profile its agent runs with, when its pane's
    /// record describes it and the profile is not the built-in default (PRD
    /// agent-profiles B17); the row's title stays as it is. One map lookup a
    /// row.
    ///
    /// An agent in an account several profiles share, as its hook reported
    /// it, is named by its agent alone (B24).
    pub(crate) fn lay_profiles(&self, agents: &mut [SidebarAgentSnapshot]) {
        for agent in agents {
            let shared = self
                .reported_account(&agent.pane_id, &agent.agent_kind)
                .is_some_and(|reported| !reported.key.is_default() && reported.named.is_none());
            if shared {
                continue;
            }
            if let Some(profile) = self.launch_profile_of(agent) {
                agent.profile = Some(profile.name);
            }
        }
    }

    /// Remembers the kind and model a start named, so every start surface
    /// preselects them next (D-18, D-20). `terminal` is never remembered, and
    /// a start with no model chose the CLI default, which is remembered too.
    pub(super) fn remember_agent_choice(&mut self, agent_kind: Option<&str>, model: Option<&str>) {
        let Some(kind) = agent_kind.and_then(hide_agent_adapter::start_kind) else {
            return;
        };
        let choice = &mut self.snapshot.ui_state.agent_start;
        let before = choice.clone();
        choice.kind = Some(kind.to_owned());
        match model {
            Some(model) => choice.models.insert(kind.to_owned(), model.to_owned()),
            None => choice.models.remove(kind),
        };
        if *choice != before {
            self.persist_ui_state();
        }
    }
}

/// What one machine's label reads name each pane's account by
/// (`Runtime::account_launches`, `LabelWorker::account_of`).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AccountReads {
    /// The launch records whose agent runs in an account other than the
    /// default.
    pub launches: BTreeMap<String, crate::model::PaneLaunch>,
    /// The account each pane's hook reported, with the agent's adapter id:
    /// the one it runs in, whatever started it.
    pub reported: BTreeMap<String, (String, AccountKey)>,
}
