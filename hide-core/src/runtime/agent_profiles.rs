//! The runtime's half of a change to a machine's agent profiles
//! (`crate::agent_profiles`): the plan made under the lock, from what the
//! machine last listed and the agents that run with each profile, and the
//! outcome taken back. The node's call runs on the caller's thread for
//! `hide agent profile`, and on a thread of its own for Settings › Agents.

use super::*;
use crate::agent_profiles::{
    self as profiles, Change, EnvEdit, InUse, Outcome, Prepared, ProfileAnswer, ProfileCommand,
    Refusal, ScreenChange,
};
use hide_agent_adapter::ProfileId;
use serde::Deserialize;

/// `agent_profile_change`: Settings › Agents asks for one change of the
/// profiles of the machine it shows.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProfileChangePayload {
    device_id: String,
    request_id: String,
    change: ScreenChange,
}

impl Runtime {
    /// Refresh only when a project's identity, device or root changed. A
    /// catalog's pane updates do not run the profile picker again.
    pub(super) fn refresh_start_profile_catalog(&mut self) {
        let kept = &self.snapshot.status.start_profiles;
        if self.catalog_workspaces().count() != kept.len()
            || self.catalog_workspaces().any(|project| {
                kept.get(&project.id).is_none_or(|row| {
                    row.device_id != project.device_id || row.root != project.path
                })
            })
        {
            self.refresh_start_profiles();
        }
    }

    /// The final automatic choice, never a second interpretation of its
    /// priority. Called on profile lists, project settings and catalog changes.
    pub(super) fn refresh_start_profiles(&mut self) {
        self.snapshot.status.start_profiles = self
            .catalog_workspaces()
            .map(|project| {
                let profiles = hide_agent_adapter::ADAPTERS
                    .iter()
                    .map(|adapter| {
                        let selected = self.new_start_profile(
                            &project.device_id,
                            Some(adapter.id),
                            Some(&project.path),
                            None,
                        );
                        let selected = selected
                            .expect("an automatic choice has no explicit profile to refuse");
                        (
                            adapter.id.to_owned(),
                            selected.map_or_else(|| "default".to_owned(), |(id, _)| id.0),
                        )
                    })
                    .collect();
                (
                    project.id.clone(),
                    crate::model::StartProfilesSnapshot {
                        device_id: project.device_id.clone(),
                        root: project.path.clone(),
                        profiles,
                    },
                )
            })
            .collect();
    }

    /// `hide agent profile` for a caller on `device`: the plan, or why
    /// there is none. A listing whose machine was read answers without a
    /// call too, but asks the node all the same, so it shows what the
    /// machine holds now.
    pub(crate) fn prepare_agent_profile_command(
        &mut self,
        device: &str,
        command: ProfileCommand,
    ) -> Result<Prepared, String> {
        self.finish_profile_starts();
        let machine = match command.machine() {
            Some(name) => self.machine_named(name)?,
            None => device.to_owned(),
        };
        let kind = match &command {
            ProfileCommand::List {
                kind: Some(kind), ..
            } => Some(agent_of(kind).map_err(|refusal| refusal.line())?),
            _ => None,
        };
        let change = self
            .plan_profile_command(&machine, command)
            .map_err(|refusal| refusal.line())?;
        let mut prepared = self
            .prepare_agent_profiles(&machine, change, None)
            .map_err(|refusal| refusal.line())?;
        prepared.kind = kind;
        Ok(prepared)
    }

    /// Settings › Agents' change: answered at once when nothing is asked of
    /// the node, else run on a thread of its own while the answer reads
    /// pending.
    pub(super) fn change_agent_profile(&mut self, payload: ProfileChangePayload) -> bool {
        self.finish_profile_starts();
        let ProfileChangePayload {
            device_id,
            request_id,
            change,
        } = payload;
        if request_id.is_empty()
            || request_id.len() > 64
            || !request_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            self.push_diagnostic(
                "agent_profiles.invalid_request",
                "A profile request id is missing or malformed",
            );
            return false;
        }
        let answer = ProfileAnswer::pending(&request_id, &device_id);
        let change = match self.plan_screen(&device_id, change) {
            Ok(Planned::Answered(answer_with)) => {
                self.snapshot.status.agent_profile_answer = Some(answer_with(answer));
                return true;
            }
            Ok(Planned::Change(change)) => change,
            Err(refusal) => {
                self.snapshot.status.agent_profile_answer = Some(answer.refused(&refusal));
                return true;
            }
        };
        let prepared = match self.prepare_agent_profiles(&device_id, change, Some(&request_id)) {
            Ok(prepared) => prepared,
            Err(refusal) => {
                self.snapshot.status.agent_profile_answer = Some(answer.refused(&refusal));
                return true;
            }
        };
        self.snapshot.status.agent_profile_answer = Some(answer);
        let spawned = std::thread::Builder::new()
            .name("hide-core-agent-profile".to_owned())
            .spawn(move || {
                // What it answers reaches the screen through the runtime.
                let _ = prepared.run();
            });
        if let Err(error) = spawned {
            // The job dropped with the closure: its slot is given back and
            // the answer refused (`abandon_agent_profiles`).
            crate::diagnostic!(serde_json::json!({
                "component": "agent_profiles",
                "kind": "profiles.not_started",
                "request": request_id,
                "reason": error.to_string(),
            }));
        }
        true
    }

    /// The plan's link and version, taking the one slot changes share.
    fn prepare_agent_profiles(
        &mut self,
        machine: &str,
        change: Change,
        request: Option<&str>,
    ) -> Result<Prepared, Refusal> {
        if self.agent_profile_busy {
            return Err(Refusal::code(profiles::BUSY));
        }
        let link = if machine == self.node.as_str() {
            self.own_node()
        } else {
            self.ready_device_channels()
                .into_iter()
                .find_map(|(ready, link)| (ready == machine).then_some(link))
                .ok_or_else(|| {
                    Refusal::code(if self.device_registration(machine).is_some() {
                        "machine_unavailable"
                    } else {
                        "machine_unknown"
                    })
                })?
        };
        let context = self
            .worker_context
            .as_ref()
            .ok_or_else(|| Refusal::code(profiles::UNANSWERED))?;
        let version = self
            .machine_profiles(machine)
            .map_or(0, |listed| listed.version);
        // A removal that will be made reads as being removed from here, in
        // the same hold of the lock as its check (`launch::holds`).
        if let Change::Remove { id, .. } = &change {
            self.profile_holds
                .remove(machine, id, version)
                .map_err(|starting| {
                    Refusal::in_use(starting.iter().map(InUse::starting).collect())
                })?;
        }
        self.agent_profile_busy = true;
        Ok(Prepared {
            machine: machine.to_owned(),
            link,
            change,
            version,
            kind: None,
            request: request.map(str::to_owned),
            runtime: context.runtime.clone(),
            notifier: context.notifier.clone(),
            settled: false,
        })
    }

    /// A CLI command as a change of `machine`'s profiles.
    fn plan_profile_command(
        &self,
        machine: &str,
        command: ProfileCommand,
    ) -> Result<Change, Refusal> {
        match command {
            ProfileCommand::List { .. } => Ok(Change::List),
            ProfileCommand::Add {
                kind,
                name,
                account,
                program,
                env,
                default,
                ..
            } => {
                let kind = agent_of(&kind)?;
                let env = env
                    .iter()
                    .map(|pair| profiles::pair(pair))
                    .collect::<Result<_, _>>()?;
                Ok(Change::Add {
                    account: profiles::account(&kind, account)?,
                    kind,
                    name,
                    program,
                    env,
                    default,
                })
            }
            ProfileCommand::Edit {
                kind,
                name,
                rename,
                program,
                no_program,
                env,
                unset_env,
                ..
            } => {
                let id = self.own_profile(machine, &kind, &name)?;
                if program.is_some() && no_program {
                    return Err(Refusal::detailed(
                        profiles::INVALID,
                        "--program and --no-program cannot be given together",
                    ));
                }
                let set = env
                    .iter()
                    .map(|pair| profiles::pair(pair))
                    .collect::<Result<_, _>>()?;
                Ok(Change::Edit {
                    id,
                    name: rename,
                    program: if no_program {
                        Some(None)
                    } else {
                        program.map(Some)
                    },
                    env: EnvEdit::Patch {
                        set,
                        unset: unset_env,
                    },
                })
            }
            ProfileCommand::Default { kind, name, .. } => {
                let agent = agent_of(&kind)?;
                let id = match self.own_profile(machine, &kind, &name) {
                    Err(refusal) if refusal.code == profiles::BUILT_IN => {
                        ProfileId::default_profile()
                    }
                    other => other?,
                };
                Ok(Change::Default { kind: agent, id })
            }
            ProfileCommand::Remove { kind, name, .. } => {
                let id = self.own_profile(machine, &kind, &name)?;
                self.plan_remove(machine, id)
            }
            ProfileCommand::Reinstall { kind, name, .. } => Ok(Change::Reinstall {
                id: self.own_profile(machine, &kind, &name)?,
            }),
        }
    }

    /// A screen change as a change of `machine`'s profiles, or the answer a
    /// removal's check gives without one.
    fn plan_screen(&self, machine: &str, change: ScreenChange) -> Result<Planned, Refusal> {
        let listed = |id: &str| self.listed_profile(machine, id);
        Ok(Planned::Change(match change {
            ScreenChange::Add {
                kind,
                name,
                account,
                program,
                env,
                default,
            } => {
                let kind = agent_of(&kind)?;
                Change::Add {
                    account: profiles::account(&kind, account)?,
                    kind,
                    name,
                    program,
                    env,
                    default,
                }
            }
            ScreenChange::Edit {
                id,
                name,
                program,
                env,
            } => Change::Edit {
                id: listed(&id)?,
                name: Some(name),
                program: Some(program.filter(|program| !program.trim().is_empty())),
                env: EnvEdit::Replace(env),
            },
            ScreenChange::Default { kind, id } => {
                let kind = agent_of(&kind)?;
                let id = if ProfileId(id.clone()).is_default() {
                    ProfileId::default_profile()
                } else {
                    listed(&id)?
                };
                Change::Default { kind, id }
            }
            ScreenChange::RemoveCheck { id } => {
                let id = listed(&id)?;
                let in_use = self.profile_in_use(machine, &id);
                let projects = self.profile_projects(machine, &id);
                let hide_ai = self.profile_in_hide_ai(machine, &id);
                return Ok(Planned::Answered(Box::new(move |mut answer| {
                    answer.state = profiles::AnswerState::Done;
                    answer.in_use = in_use;
                    answer.projects = projects;
                    answer.hide_ai = hide_ai;
                    answer
                })));
            }
            ScreenChange::Remove { id } => self.plan_remove(machine, listed(&id)?)?,
            ScreenChange::Reinstall { id } => Change::Reinstall { id: listed(&id)? },
            ScreenChange::Values { id } => Change::Values { id: listed(&id)? },
        }))
    }

    /// A removal, refused while an agent runs with the profile, sleeping
    /// ones included, or starts with it (D-11, B27); once its job is
    /// prepared the profile reads as being removed, so no start takes it
    /// meanwhile (`launch::holds`).
    fn plan_remove(&self, machine: &str, id: ProfileId) -> Result<Change, Refusal> {
        let in_use = self.profile_in_use(machine, &id);
        if !in_use.is_empty() {
            return Err(Refusal::in_use(in_use));
        }
        Ok(Change::Remove {
            projects: self.profile_projects(machine, &id),
            id,
        })
    }

    /// A removal of `id` on `machine` made again on a list at `version`
    /// that moved since its plan: refused as its plan would be while an
    /// agent runs, sleeps or starts with the profile, otherwise marked being
    /// removed at that version (`launch::holds`).
    pub(crate) fn recheck_removal(
        &mut self,
        machine: &str,
        id: &ProfileId,
        version: u64,
    ) -> Result<(), Refusal> {
        self.finish_profile_starts();
        let in_use = self.profile_in_use(machine, id);
        if !in_use.is_empty() {
            return Err(Refusal::in_use(in_use));
        }
        self.profile_holds
            .remove(machine, id, version)
            .map_err(|starting| Refusal::in_use(starting.iter().map(InUse::starting).collect()))
    }

    /// `request`, holding its profile on `machine` while it starts
    /// (`launch::holds`): refused while the profile is being removed, or
    /// when its launch record would have no room. The built-in default
    /// holds nothing.
    pub(crate) fn hold_profile_start(
        &mut self,
        machine: &str,
        request: crate::launch::Request,
    ) -> Result<crate::launch::Request, crate::launch::Refusal> {
        self.finish_profile_starts();
        use crate::launch::holds::Refused;
        let Some((id, name)) = request.profile() else {
            return Ok(request);
        };
        let kind = request.kind().to_owned();
        let records = self.snapshot.ui_state.pane_launches.len();
        match self.profile_holds.hold(
            machine,
            &id,
            &kind,
            records,
            agent_choice::PANE_LAUNCH_LIMIT,
            request.choice(),
        ) {
            Ok(hold) => Ok(request.held(Some(hold))),
            Err(Refused::Removing) => Err(crate::launch::Refusal {
                code: crate::launch::PROFILE_GONE,
                detail: format!("{kind}'s profile {name} is being removed, so it was not started"),
            }),
            Err(Refused::RecordsFull) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "agent_profiles",
                    "kind": "profile_start.records_full",
                    "device_id": machine,
                    "profile": id.0,
                    "limit": agent_choice::PANE_LAUNCH_LIMIT,
                }));
                Err(crate::launch::Refusal {
                    code: crate::launch::LAUNCH_RECORDS_FULL,
                    detail: format!(
                        "{kind} with the profile {name} was not started: Hide keeps no more records of what its agents started with"
                    ),
                })
            }
        }
    }

    /// Transfers completed attempts while admission remains locked: the
    /// record takes the reserved slot before the hold is released.
    pub(crate) fn finish_profile_starts(&mut self) {
        let mut moved = false;
        for (id, start) in self.profile_holds.completed() {
            if let (Some(pane), Some(record)) = (&start.pane, &start.record) {
                let pane = crate::address::pane_id_on(self.node.as_str(), &start.machine, pane);
                if self
                    .snapshot
                    .ui_state
                    .pane_launches
                    .get(&pane)
                    .is_none_or(|kept| kept.start < id)
                {
                    let mut record = record.clone();
                    // A publish may already have observed this session while
                    // its worker was still returning. Preserve that knowledge.
                    if let Some(agent) = self.agent_rows().find(|agent| agent.pane_id == pane) {
                        agent_choice::observe_launch(&mut record, agent);
                    }
                    self.snapshot.ui_state.pane_launches.insert(pane, record);
                    moved = true;
                }
            }
            self.profile_holds.release(id);
        }
        if moved {
            self.missions.launches_edit = self.missions.launches_edit.wrapping_add(1);
            self.persist_ui_state();
        }
    }

    /// The profile of `kind` named `name` on `machine`, one of the
    /// operator's own: the built-in profile is refused by its own code.
    fn own_profile(&self, machine: &str, kind: &str, name: &str) -> Result<ProfileId, Refusal> {
        let agent = agent_of(kind)?;
        let builtin = hide_agent_adapter::Profile::builtin_default(&agent);
        if name.eq_ignore_ascii_case(&builtin.name) {
            return Err(Refusal::code(profiles::BUILT_IN));
        }
        let listed = self.listed_profiles(machine)?;
        listed
            .profiles
            .iter()
            .find(|profile| profile.kind == agent && profile.name.eq_ignore_ascii_case(name))
            .map(|profile| ProfileId(profile.id.clone()))
            .ok_or_else(|| Refusal::detailed(profiles::GONE, format!("{agent} {name}")))
    }

    /// The profile `id` as `machine` lists it.
    fn listed_profile(&self, machine: &str, id: &str) -> Result<ProfileId, Refusal> {
        if ProfileId(id.to_owned()).is_default() {
            return Err(Refusal::code(profiles::BUILT_IN));
        }
        self.listed_profiles(machine)?
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .map(|profile| ProfileId(profile.id.clone()))
            .ok_or_else(|| Refusal::detailed(profiles::GONE, id.to_owned()))
    }

    /// What `machine` last listed, refused while it keeps none or was never
    /// read.
    fn listed_profiles(&self, machine: &str) -> Result<&crate::model::ProfilesSnapshot, Refusal> {
        let listed = self
            .machine_profiles(machine)
            .ok_or_else(|| Refusal::code(profiles::UNREAD))?;
        if listed.state == crate::model::ProfilesState::Unsupported {
            return Err(Refusal::code(profiles::UNSUPPORTED));
        }
        Ok(listed)
    }

    /// The agents on `machine` that run with the profile `id`: running
    /// ones whose pane's record names it, sleeping ones whose record does,
    /// here or closed (D-11), and starts that hold it (`launch::holds`).
    pub(super) fn profile_in_use(&self, machine: &str, id: &ProfileId) -> Vec<InUse> {
        let node = self.node.as_str();
        let on = |pane: &str| crate::address::split_pane(node, pane).0 == machine;
        let sleep = &self.snapshot.ui_state.agent_sleep;
        let mut found: Vec<InUse> = self
            .agent_rows()
            .filter(|row| on(&row.pane_id))
            .filter_map(|row| {
                let record = sleep.records.get(&row.pane_id);
                let profile = self
                    .launch_profile_of(row)
                    .or_else(|| record.and_then(|record| record.profile.clone()))?;
                (profile.id == *id).then(|| InUse {
                    pane_id: row.pane_id.clone(),
                    title: row.identity_label.clone(),
                    place: row.workspace_label.clone(),
                    asleep: record.is_some(),
                    starting: false,
                })
            })
            .collect();
        // An unknown result has a real launch record even before any row.
        // Once a row is observed the normal agent/account rule owns its use.
        for (pane, launch) in &self.snapshot.ui_state.pane_launches {
            if on(pane)
                && launch.pending
                && self
                    .run_in(pane, &launch.kind, Some(launch))
                    .profile
                    .is_some_and(|profile| profile.id == *id)
                && !found.iter().any(|agent| agent.pane_id == *pane)
            {
                found.push(InUse {
                    pane_id: pane.clone(),
                    title: launch.name.clone(),
                    place: String::new(),
                    asleep: false,
                    starting: false,
                });
            }
        }
        for record in sleep.dormant.values() {
            if record.node_id == machine
                && record
                    .profile
                    .as_ref()
                    .is_some_and(|profile| profile.id == *id)
                && !found
                    .iter()
                    .any(|agent| agent.pane_id == record.old_pane_id)
            {
                found.push(InUse {
                    pane_id: record.old_pane_id.clone(),
                    title: record.identity_label.clone(),
                    place: record
                        .owner
                        .as_ref()
                        .map_or_else(String::new, |owner| owner.label.clone()),
                    asleep: true,
                    starting: false,
                });
            }
        }
        // Starts under way hold it too, before they have a pane to show.
        found.extend(
            self.profile_holds
                .starting(machine, id)
                .iter()
                .map(InUse::starting),
        );
        found
    }

    /// The names of the projects on `machine` whose settings start an agent
    /// with the profile `id` (D-31).
    fn profile_projects(&self, machine: &str, id: &ProfileId) -> Vec<String> {
        let mut names: Vec<String> = self
            .snapshot
            .ui_state
            .project_settings
            .values()
            .filter(|record| {
                record.device_id == machine
                    && record.agent_profiles.values().any(|chosen| *chosen == id.0)
            })
            .map(|record| {
                record.name.clone().unwrap_or_else(|| {
                    std::path::Path::new(&record.root).file_name().map_or_else(
                        || record.root.clone(),
                        |name| name.to_string_lossy().into_owned(),
                    )
                })
            })
            .collect();
        names.sort();
        names
    }

    /// Whether Hide AI's order names the profile `id` of `machine`: an item
    /// of it placed there, which a delete leaves as it is, naming a profile
    /// that is gone (PRD agent-profiles-phase2 B21, B22).
    fn profile_in_hide_ai(&self, machine: &str, id: &ProfileId) -> bool {
        let Some(settings) = self.ai_settings.as_ref() else {
            return false;
        };
        settings.order().iter().any(|item| {
            item.profile.as_ref() == Some(id)
                && settings.machine(item).unwrap_or(self.node.as_str()) == machine
        })
    }

    /// A removed profile leaves the projects that started with it, which go
    /// back to the agent's default (D-31).
    fn forget_project_profile(&mut self, machine: &str, id: &ProfileId) -> bool {
        let mut changed = false;
        let mut forget = |record: &mut crate::project_settings::ProjectSettings| {
            if record.device_id == machine {
                let before = record.agent_profiles.len();
                record.agent_profiles.retain(|_, chosen| *chosen != id.0);
                changed |= before != record.agent_profiles.len();
            }
        };
        for record in self.snapshot.ui_state.project_settings.values_mut() {
            forget(record);
        }
        for pending in self.project_settings_pending.values_mut() {
            forget(&mut pending.record);
        }
        if changed {
            self.persist_ui_state();
        }
        changed
    }

    /// What a run of `job` came to: the list its machine answered, a
    /// removed profile's projects, and the screen's answer. Gives the slot
    /// back.
    pub(crate) fn settle_agent_profiles(&mut self, job: &Prepared, outcome: &Outcome) -> bool {
        self.agent_profile_busy = false;
        let mut changed = false;
        // A removal the node definitely did not make leaves the profile to
        // its starts; one it may have made reads as being removed until a
        // list settles it (`launch::holds`).
        if let (Change::Remove { id, .. }, Err(refusal)) = (&job.change, &outcome.result)
            && refusal.code != profiles::UNANSWERED
        {
            self.profile_holds.not_removed(&job.machine, id);
        }
        if let Some(list) = &outcome.list {
            changed |= self.ingest_profiles(&job.machine, kit::ProfilesRead::Listed(list.clone()));
        }
        if let (Change::Remove { id, .. }, Ok(_)) = (&job.change, &outcome.result) {
            changed |= self.forget_project_profile(&job.machine, id);
        }
        if let Some(request) = &job.request {
            let answer = ProfileAnswer::pending(request, &job.machine);
            let answer = match &outcome.result {
                Ok(done) => ProfileAnswer {
                    state: profiles::AnswerState::Done,
                    values: done.values.clone(),
                    added: done.added.as_ref().map(|id| id.0.clone()),
                    ..answer
                },
                Err(refusal) => answer.refused(refusal),
            };
            // A newer request owns the slot: its answer stays.
            if self
                .snapshot
                .status
                .agent_profile_answer
                .as_ref()
                .is_none_or(|current| current.request_id == *request)
            {
                self.snapshot.status.agent_profile_answer = Some(answer);
                changed = true;
            }
        }
        changed
    }

    /// A job that ended without an outcome gives the slot back, and its
    /// screen request reads refused.
    pub(crate) fn abandon_agent_profiles(&mut self, request: Option<&str>) {
        // A removal that ended without an outcome may still be made: its
        // profile reads as being removed until a list settles it.
        self.agent_profile_busy = false;
        if let Some(answer) = self
            .snapshot
            .status
            .agent_profile_answer
            .as_mut()
            .filter(|answer| Some(answer.request_id.as_str()) == request)
        {
            answer.state = profiles::AnswerState::Refused;
            answer.code = Some(profiles::UNANSWERED.to_owned());
        }
    }
}

/// A plan, or the answer given without one.
enum Planned {
    Change(Change),
    Answered(Box<dyn FnOnce(ProfileAnswer) -> ProfileAnswer>),
}

/// The agent `kind` names, by its id, refused for an agent that keeps no
/// profiles (D-25).
fn agent_of(kind: &str) -> Result<String, Refusal> {
    hide_agent_adapter::adapter(kind)
        .map(|adapter| adapter.id)
        .filter(|id| profiles::agents().any(|agent| agent == *id))
        .map(str::to_owned)
        .ok_or_else(|| {
            Refusal::detailed(
                profiles::INVALID,
                format!("Hide keeps no profiles for an agent named {kind}"),
            )
        })
}
