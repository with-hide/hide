//! The agent kind and model a start names, the words they become on the
//! agent's command line, and the choice every start surface preselects next
//! (PRD home-device-rail D-18, D-20).
//!
//! Every start (⌘N, New worktree, an issue's Start, 맡기기, the phone) goes
//! through here, so a pick on any of them is the next default on all of
//! them, and a model reaches the CLI the same way wherever it was chosen.
//!
//! The model and effort a pane's agent was started with are kept here too
//! (`ui_state.pane_launches`), for the conversation it was started for, so
//! every restart Hide makes of it (a wake, a fork, a reopen) starts it with
//! the same choice instead of the CLI's configured default.

use super::*;

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
    launch_arguments(kind, &choice).map_err(|reason| format!("{model}: {reason}"))?;
    Ok(Some(model.to_owned()))
}

/// The agent CLI's own arguments for a start: the model a `chosen_model`
/// admitted, in the form the kind's adapter declares, and each folder a Home
/// agent may write through its links (D-08). Extra roots are passed only to
/// a launch dialect that accepts them, independently of the checkout cwd.
pub(crate) fn agent_arguments(
    kind: Option<&str>,
    model: Option<&str>,
    add_dirs: &[String],
) -> Vec<String> {
    let dialect = kind
        .and_then(hide_agent_adapter::adapter)
        .and_then(|row| row.start);
    let mut args = Vec::new();
    if let (Some(model), Some(dialect)) = (model, dialect) {
        args.extend(
            dialect
                .options()
                .model
                .iter()
                .map(|part| part.replace("{}", model)),
        );
    }
    if dialect.is_some_and(|dialect| dialect.accepts_extra_directories()) {
        for dir in add_dirs {
            args.push("--add-dir".to_owned());
            args.push(dir.clone());
        }
    }
    args
}

/// The arguments a restart of a `kind` agent adds for the choice it was
/// started with, after its resume arguments; none for the CLI's defaults.
/// A saved choice the agent does not take (a store edited by hand) is
/// refused with the reason rather than dropped, so the restart says so.
pub(crate) fn launch_arguments(
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
/// holds at most one per such pane; a start past this many is recorded
/// nowhere and logged (`pane_launch.limit`), and its restarts run on the
/// CLI's defaults.
pub(crate) const PANE_LAUNCH_LIMIT: usize = 1024;

/// Whether `launch` describes `agent`: the same kind, and the name Hide
/// started it under, or, for an agent that came back from a Herdr restart
/// without a name, the conversation that named agent last reported.
fn describes(launch: &crate::model::PaneLaunch, agent: &SidebarAgentSnapshot) -> bool {
    hide_agent_adapter::canonical_kind(&agent.agent_kind) == launch.kind
        && match agent.started_name.as_deref() {
            Some(name) => name == launch.name,
            None => launch.session.is_some() && launch.session == agent.session_id,
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
    /// Records the choice Hide started a `kind` agent named `name` with in
    /// `pane_id` (the core's id for the pane), replacing what the pane held:
    /// a start on the CLI's defaults leaves none. `session` is the
    /// conversation a restart resumed there, which the record then already
    /// describes.
    pub(crate) fn record_pane_launch(
        &mut self,
        pane_id: &str,
        kind: &str,
        name: &str,
        choice: &hide_agent_adapter::LaunchChoice,
        session: Option<&str>,
    ) {
        if self.put_pane_launch(pane_id, kind, name, choice, session) {
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
        session: Option<&str>,
    ) -> bool {
        let launches = &mut self.snapshot.ui_state.pane_launches;
        if choice.is_default() {
            launches.remove(pane_id).is_some()
        } else if !launches.contains_key(pane_id) && launches.len() >= PANE_LAUNCH_LIMIT {
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
            };
            launches.insert(pane_id.to_owned(), launch.clone()) != Some(launch)
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
        let prefix = remote_pane_id_prefix(device);
        let launches = &mut self.snapshot.ui_state.pane_launches;
        let count = launches.len();
        launches.retain(|pane_id, _| !pane_id.starts_with(&prefix));
        if launches.len() != count {
            self.persist_ui_state();
        }
    }

    /// Settles the launch records against one pass's agent rows, and saves
    /// them when one moved.
    pub(super) fn apply_pane_launches(
        &mut self,
        agents: &[SidebarAgentSnapshot],
        scope: crate::agent_state::ReadRecordScope<'_>,
        live_pane_ids: Option<&HashSet<String>>,
    ) {
        if settle_pane_launches(
            &mut self.snapshot.ui_state.pane_launches,
            agents,
            live_pane_ids,
            scope,
        ) {
            self.persist_ui_state();
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
