//! A daemon's engine opening its saved session again (PRD
//! engine-bundle-3-core-owns D-7): the saved session says where every pane
//! goes, and the node's pane records say which panes still run. The engine
//! starts with the saved layout, so a screen draws its tabs at once, and a
//! thread of its own then waits for the node to say which holders it found
//! ([`PaneHolders::recovered`]): a pane whose holder runs is taken as it is,
//! the same pane and the same process; a saved pane whose program exited or
//! that a node ended closes, as a pane whose program exits closes; a saved
//! pane whose holder ended under it (a reboot) or never ran gets a new shell
//! under its own id in its saved folder, or in the home folder with a line
//! saying why when that folder is gone; a holder the saved session does not place (the daemon died before
//! it saved, or the save could not be read) is gathered into a new tab of
//! the workspace its id names, so no running shell is lost or ended. Every
//! id counter is then taken past every id the records hold, so a new pane
//! never meets an old record under its own id.
//!
//! A restarted pane that ran an agent opens its shell and then resumes the
//! agent's conversation in it (D-7, B13): by its `[resume] session` when the
//! save knows the session, else by its `[resume] last`. The resumes are typed
//! one pane at a time once the restore's gate has let other calls through,
//! each once its shell is ready as any start into a shell waits for it
//! ([`crate::agent_start::wait_for_shell`]), and no conversation is resumed in
//! two panes: a second pane with the same session (or, for `last`, the same
//! agent in the same folder) or one an adopted pane still runs stays a shell.
//! A pane that cannot be resumed opens with one line saying why, and a resume
//! whose agent ends with a failure says so in the pane, which is a shell
//! again. The saved agent stays the pane's saved agent until its resume is
//! typed (`Restored::Resume`), so a daemon that ends first, with the resumes
//! of a dozen panes still waiting their turn, leaves each to the next restore;
//! a resume that could not be typed is tried once more after the others
//! ([`RESUME_ATTEMPTS`]). An agent that is not resumed for good, in a pane
//! that was restarted or adopted at its prompt, is named by the session
//! (`ProjectionState::not_resumed`), and the core ends its registration, so
//! its row reads ended rather than running with nothing behind it. That is
//! the agent whose resume could not be typed and the one whose typed resume
//! ended without the agent running: its start's deadline passed with no
//! program shown, or its program left the foreground before the start settled
//! ([`PaneAgent::resume_missed`]), the same deadline every start has. An
//! adopted pane whose saved agent is not running, its shell the
//! terminal's foreground, is resumed the same way: the daemon that typed
//! its resume ended before the line reached the shell, or the agent ended
//! while no daemon ran.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hide_agent_adapter::definition::Catalog;
use hide_node_link::holders::HolderRecovery;
use hide_node_link::protocol::Call;

use super::agents::{self, PaneAgent};
use super::shell_line;
use super::starts::STARTS_AT_ONCE;
use super::store::{self, Read};
use super::topology::{Pane, Restored, Topology};
use super::{EngineError, Env, OwnEngine, holder_error};
use crate::agent_start::{ShellStart, StartError};
use crate::model::PaneLaunch;
use crate::node_access::{NodeLink, call_as};
use crate::runtime::agent_choice::{choice_arguments, launch_describes};
use crate::session_sync::NotResumed;
use crate::start_prompt::ByName;

/// How long the restore waits for the node to finish judging its records.
/// A node judges each record in a few rounds of a bounded budget, so this
/// only runs out when the node is stuck; the saved panes are then marked
/// as not restored rather than started over holders that may still run.
const RECOVERY_WAIT: Duration = Duration::from_secs(120);

/// How long a call that makes a pane waits for the restore: the node's
/// recovery and the restarts after it, [`STARTS_AT_ONCE`] at a time.
pub(super) const BOUND: Duration = Duration::from_secs(180);

/// How long the restore waits for the node to say whether a saved pane's
/// folder is still there.
const FOLDER_WAIT: Duration = Duration::from_secs(10);

/// How many times a resume that could not be typed is tried: once in its turn
/// and once more after every other resume has had its turn. A machine that
/// restarted dozens of panes at once holds shells past their start wait
/// (2026-11-06, 42 resumes at load 324), and by the second pass the load
/// the others made has settled. Two tries are the cap, since each can wait a
/// start's full shell wait; a resume that fails both is left as an agent that
/// was not resumed, whether its shell would not take it or the load had not
/// settled yet.
const RESUME_ATTEMPTS: usize = 2;

/// The name of the tab running holders the saved session does not place
/// are gathered in.
const GATHERED_LABEL: &str = "restored";

/// The session saved at `path`, or an empty one when there is none or it
/// cannot be read or does not hold together; a file that cannot be used is
/// moved into `backups` and the reason logged, never deleted (D-6).
pub(super) fn read(path: &Path, backups: &Path) -> Topology {
    match store::read(path, backups) {
        Read::Missing | Read::Unreadable => Topology::default(),
        Read::Saved(saved) => match Topology::from_saved(saved) {
            Ok(topology) => topology,
            Err(why) => {
                store::set_aside_unusable(path, backups, &why);
                Topology::default()
            }
        },
    }
}

/// Starts the restore of the panes `saved` names on a thread of its own.
pub(super) fn start(engine: OwnEngine<'static>, saved: BTreeSet<String>) -> Result<(), String> {
    std::thread::Builder::new()
        .name("hide-own-engine-restore".into())
        .spawn(move || {
            let gate = engine.restore_gate();
            let resumes = engine.restore(&saved);
            // Release the restore's ownership before opening the gate, so
            // an engine its caller drops is dropped whole.
            // Resumes hold it only while each one runs; the next stops when
            // its owner has let the engine go (a device's, with its link).
            let shared = std::sync::Arc::downgrade(&engine.shared);
            drop(engine);
            match resumes {
                Ok(resumes) => {
                    gate.finish(Ok(()));
                    OwnEngine::resume_agents(&shared, &resumes);
                }
                Err(error) => gate.finish(Err(error)),
            }
        })
        .map(|_| ())
        .map_err(|error| format!("the session restore thread could not start: {error}"))
}

/// One saved pane to start again: its id, the folder it was saved in, and
/// the agent it ran.
#[derive(Clone)]
struct Restart {
    pane: String,
    cwd: String,
    agent: Option<PaneAgent>,
    previous: Option<u64>,
}

/// What starting every pane of a restore left: the panes that could not
/// start, with why, the agents to resume, in `restarts` order, and the panes
/// whose agent opened a shell that is not resumed, with the line that says
/// why.
struct Restarted {
    failures: Vec<(String, String)>,
    resumes: Vec<Resume>,
    left: Vec<(String, String)>,
}

/// What a restarted pane's agent gets: its resume, typed once its shell is
/// ready, or the line its shell opens with saying why not.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Plan {
    Resume(Box<Resume>),
    Shell(String),
}

/// One agent to resume in a restarted pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Resume {
    pane: String,
    /// The definition's id.
    kind: String,
    /// Its name for the operator, in the line a failed resume prints.
    shown: String,
    /// The name it was started under.
    name: Option<String>,
    session: Option<String>,
    argv: Vec<String>,
    /// Its hook last said its turn was running or waiting on the operator:
    /// the turn was cut off, and the core tells the agent once it is back
    /// (PRD engine-bundle-4-herdr-removal D-22).
    cut: bool,
    /// Set when its own pane runs another command: it resumes in a new tab
    /// opened for it (PRD engine-bundle-4-herdr-removal D-8, B15).
    new_tab: Option<NewTab>,
    /// The profile its pane's launch record keeps, which the resume runs it
    /// with (PRD agent-profiles D-27); `None` for the built-in default.
    profile: Option<crate::launch::LaunchProfile>,
}

#[cfg(test)]
impl Resume {
    /// A restore's resume of the `kind` agent of `pane` into `session`, with
    /// a profile.
    pub(super) fn of_profile(pane: &str, kind: &str, session: &str) -> Self {
        Self {
            pane: pane.to_owned(),
            kind: kind.to_owned(),
            shown: kind.to_owned(),
            name: None,
            session: Some(session.to_owned()),
            argv: vec![kind.to_owned()],
            cut: false,
            new_tab: None,
            profile: Some(crate::launch::LaunchProfile {
                id: hide_agent_adapter::ProfileId("p1".to_owned()),
                name: "gw".to_owned(),
                account: hide_agent_adapter::AccountKey::Default,
            }),
        }
    }
}

/// Where a conversation an import gave back resumes when its pane runs
/// another command, or its system does not say what runs there (Windows): a
/// new tab of the pane's workspace, in the conversation's folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct NewTab {
    workspace: String,
    cwd: String,
}

/// What each restarted pane's saved agent gets, in `restarts` order: a
/// conversation is resumed in one pane only, and never in a restarted pane
/// while an adopted pane runs it (`running`, by [`conversation`]). A resume
/// carries the model and effort Hide started the agent with when the pane's
/// launch record in `launches` describes the saved agent (PRD
/// engine-bundle-4-herdr-removal D-24), and the profile it ran with (PRD
/// agent-profiles D-27); an agent with no such record resumes on its CLI's
/// defaults and the built-in default profile.
fn plans(
    catalog: &Catalog,
    launches: &BTreeMap<String, PaneLaunch>,
    restarts: &[Restart],
    mut running: BTreeSet<(String, String)>,
    waiting: &BTreeSet<String>,
) -> BTreeMap<String, Plan> {
    let mut plans = BTreeMap::new();
    for restart in restarts {
        let Some(agent) = &restart.agent else {
            // An imported pane whose conversation the Herdr it came from
            // still has open (PRD engine-bundle-4-herdr-removal D-8, B15).
            if waiting.contains(&restart.pane) {
                plans.insert(
                    restart.pane.clone(),
                    Plan::Shell(
                        "hide: this pane's conversation is open in Herdr, so it was not resumed here; this pane is a shell now.".to_owned(),
                    ),
                );
            }
            continue;
        };
        let plan = match agents::definition(catalog, &agent.kind) {
            None => Plan::Shell(format!(
                "hide: no agent definition names {} any more, so its conversation was not resumed; this pane is a shell now.",
                agent.kind
            )),
            Some(definition) => {
                let shown = definition.name.clone();
                match agents::resume_argv(definition, agent.session.as_deref())
                    .map(|argv| with_launch_choice(argv, launches, restart, agent))
                {
                    None => Plan::Shell(format!(
                        "hide: {shown} says no way to resume a conversation, so this pane is a shell now."
                    )),
                    Some(Err(why)) => Plan::Shell(format!(
                        "hide: {shown} was not resumed because {why}; this pane is a shell now."
                    )),
                    Some(Ok(argv)) => match shell_line::check_resume(&argv) {
                        Err(why) => Plan::Shell(format!(
                            "hide: {shown} was not resumed because {why}; this pane is a shell now."
                        )),
                        Ok(()) if !running.insert(conversation(agent, &restart.cwd)) => {
                            Plan::Shell(format!(
                                "hide: {shown}'s conversation is open in another pane, so this pane is a shell now."
                            ))
                        }
                        Ok(()) => Plan::Resume(Box::new(Resume {
                            pane: restart.pane.clone(),
                            kind: definition.id.clone(),
                            shown,
                            name: agent.name.clone(),
                            session: agent.session.clone(),
                            argv,
                            cut: false,
                            new_tab: None,
                            profile: described(launches, restart, agent)
                                .and_then(|launch| launch.profile.clone()),
                        })),
                    },
                }
            }
        };
        plans.insert(restart.pane.clone(), plan);
    }
    plans
}

/// Whether `resume`'s agent was cut off mid-turn: the last record its hook
/// wrote in the pane's file under `records` (the daemon's hook records) for
/// that agent, and for its conversation when both name one, says its turn was
/// running or waiting on the operator. A pane no hook wrote, or an agent that
/// rested, was not.
fn was_cut(catalog: &Catalog, records: &Path, resume: &Resume) -> bool {
    hide_agent_hooks::counters::records_in(records, &resume.pane)
        .iter()
        .rev()
        .find(|record| {
            agents::definition(catalog, &record.agent)
                .is_some_and(|definition| definition.id == resume.kind)
                && match (resume.session.as_deref(), record.session.as_deref()) {
                    (Some(resumed), Some(reported)) => resumed == reported,
                    _ => true,
                }
        })
        .is_some_and(|record| {
            matches!(
                record.turn,
                Some(hide_agent_hooks::report::HookTurn::Working)
                    | Some(hide_agent_hooks::report::HookTurn::Waiting)
            )
        })
}

/// `argv`, a resume of `agent` in `restart`'s pane, followed by the model
/// and effort its pane's launch record keeps when the record describes it
/// (`launch_describes`, the rule a pane's row reads the record by). A saved
/// choice the agent does not take is refused with why, rather than resumed
/// on another model.
fn with_launch_choice(
    mut argv: Vec<String>,
    launches: &BTreeMap<String, PaneLaunch>,
    restart: &Restart,
    agent: &PaneAgent,
) -> Result<Vec<String>, String> {
    let Some(launch) = described(launches, restart, agent) else {
        return Ok(argv);
    };
    argv.extend(
        choice_arguments(&agent.kind, &launch.choice)
            .map_err(|why| format!("its saved model and effort do not fit it ({why})"))?,
    );
    Ok(argv)
}

/// The launch record of `restart`'s pane when it describes `agent`
/// (`launch_describes`, the rule a pane's row reads the record by).
fn described<'a>(
    launches: &'a BTreeMap<String, PaneLaunch>,
    restart: &Restart,
    agent: &PaneAgent,
) -> Option<&'a PaneLaunch> {
    launches.get(&restart.pane).filter(|launch| {
        launch_describes(
            launch,
            &agent.kind,
            agent.name.as_deref(),
            agent.session.as_deref(),
        )
    })
}

/// Which conversation an agent holds, as far as a resume can tell: its
/// session when known, else its agent's last one in its folder.
fn conversation(agent: &PaneAgent, cwd: &str) -> (String, String) {
    match &agent.session {
        Some(session) => (agent.kind.clone(), format!("session {session}")),
        None => (agent.kind.clone(), format!("last in {cwd}")),
    }
}

impl OwnEngine<'static> {
    fn restore(&self, saved: &BTreeSet<String>) -> Result<Vec<Resume>, EngineError> {
        let recovery = match self.shared.holders.recovered(RECOVERY_WAIT) {
            Ok(recovery) => recovery,
            Err(error) => {
                let discarded_rules = self.shared.rules.end();
                crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "engine.restore_unfinished",
                    "message": error.to_string(),
                    "discarded_rules": discarded_rules,
                }));
                self.mark_unrestored(
                    saved,
                    &format!("the node did not say which panes still run: {error}"),
                );
                return Err(holder_error(error));
            }
        };
        // LocalPanes may publish facts while PaneSlot is still acquiring.
        // Only this successful answer acknowledges its Ready owner; let
        // queued rules through before adopt can start or inspect panes.
        self.shared.rules.ready();
        Ok(self.adopt(saved, &recovery))
    }

    /// Lays the panes `saved` names out around what `recovery` found: the
    /// agents to resume in the panes that started again.
    fn adopt(&self, saved: &BTreeSet<String>, recovery: &HolderRecovery) -> Vec<Resume> {
        let present: BTreeSet<&str> = recovery.present.iter().map(String::as_str).collect();
        let home = self.home().ok();
        let (finished, restarts, orphans, adopted_agents) = {
            let mut topology = self.topology();
            topology.count_past(
                recovery
                    .present
                    .iter()
                    .chain(&recovery.ended)
                    .chain(&recovery.closed)
                    .map(String::as_str),
            );
            // A saved pane whose program exited, or that a node ended, while
            // no daemon ran is finished: it closes, its tab and workspace
            // with it when it was their last, and no new shell takes it.
            let finished: Vec<String> = recovery
                .closed
                .iter()
                .filter(|id| saved.contains(*id) && topology.remove_pane(id).is_ok())
                .cloned()
                .collect();
            // Existence is durable before the coalesced layout save. Gather
            // those recorded IDs before selecting the one bounded restart set.
            let missing = recovery
                .recoverable
                .iter()
                .filter_map(|record| {
                    if topology.pane(&record.pane).is_ok() {
                        return None;
                    }
                    record
                        .cwd
                        .as_ref()
                        .map(|cwd| (record.pane.clone(), Pane::new(cwd, None)))
                })
                .collect();
            topology.gather(missing, GATHERED_LABEL);
            let placed: BTreeSet<String> = topology.panes().map(|(id, _)| id.to_owned()).collect();
            let mut restarted: Vec<String> = topology
                .panes()
                .filter(|(id, _)| {
                    (saved.contains(*id)
                        || recovery.recoverable.iter().any(|record| record.pane == *id))
                        && !present.contains(id)
                })
                .map(|(id, _)| id.to_owned())
                .collect();
            // The shells of the tab the screen opens on start first, so its
            // panes draw before the rest have their turn.
            topology.front_first(&mut restarted);
            // A restarted pane's agent ended with its holder: the pane is a
            // shell until its resume is typed, and the agent is its saved
            // agent until then.
            let restarts: Vec<Restart> = restarted
                .into_iter()
                .filter_map(|id| {
                    let pane = topology.pane_mut(&id).ok()?;
                    let agent = pane.agent.take();
                    pane.restored = agent.clone().map(Restored::Resume);
                    Some(Restart {
                        cwd: pane.cwd.clone(),
                        agent,
                        previous: recovery
                            .recoverable
                            .iter()
                            .find(|record| record.pane == id)
                            .map(|record| record.incarnation),
                        pane: id,
                    })
                })
                .collect();
            let adopted_agents: Vec<String> = topology
                .panes()
                .filter(|(id, pane)| present.contains(id) && pane.agent.is_some())
                .map(|(id, _)| id.to_owned())
                .collect();
            let orphans: Vec<String> = recovery
                .present
                .iter()
                .filter(|id| !placed.contains(*id))
                .cloned()
                .collect();
            (finished, restarts, orphans, adopted_agents)
        };
        // Asked of each holder off the topology lock.
        let (idle, busy): (Vec<String>, Vec<String>) = adopted_agents
            .into_iter()
            .partition(|pane| self.at_its_prompt(pane));
        let given_back = self.settle_given_back(&busy);
        let (idle, running) = {
            let mut topology = self.topology();
            let idle: Vec<Restart> = idle
                .into_iter()
                .filter_map(|id| {
                    let pane = topology.pane_mut(&id).ok()?;
                    let agent = pane.agent.take();
                    pane.restored = agent.clone().map(Restored::Resume);
                    Some(Restart {
                        cwd: pane.cwd.clone(),
                        agent,
                        previous: recovery
                            .recoverable
                            .iter()
                            .find(|record| record.pane == id)
                            .map(|record| record.incarnation),
                        pane: id,
                    })
                })
                .collect();
            // The conversations the adopted panes' agents still hold.
            let running: BTreeSet<(String, String)> = topology
                .panes()
                .filter(|(id, _)| present.contains(id))
                .filter_map(|(_, pane)| Some(conversation(pane.agent.as_ref()?, &pane.cwd)))
                .collect();
            (idle, running)
        };
        let both: Vec<Restart> = restarts.iter().chain(&idle).cloned().collect();
        let no_launches = BTreeMap::new();
        let launches = self
            .shared
            .place
            .as_ref()
            .map_or(&no_launches, |place| &*place.launches);
        let waiting = self
            .shared
            .place
            .as_ref()
            .map(|place| crate::herdr_import::waiting(&place.engine_dir))
            .unwrap_or_default();
        let mut plans = plans(
            &self.shared.definitions.get(),
            launches,
            &both,
            running,
            &waiting,
        );
        self.mark_cut(&mut plans);
        let Restarted {
            failures,
            mut resumes,
            mut left,
        } = self.start_restarts(&restarts, &plans, home.as_deref());
        resumes.extend(given_back);
        for restart in &idle {
            match plans.get(&restart.pane) {
                Some(Plan::Resume(resume)) => resumes.push((**resume).clone()),
                Some(Plan::Shell(why)) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "own_engine",
                        "kind": "engine.restore_not_resumed",
                        "pane_id": restart.pane,
                        "message": why,
                    }));
                    if restart.agent.is_some() {
                        left.push((restart.pane.clone(), why.clone()));
                    }
                }
                None => {}
            }
        }
        // A pane whose shell could not start never ran its agent again.
        let said: BTreeSet<String> = left.iter().map(|(pane, _)| pane.clone()).collect();
        left.extend(
            failures
                .iter()
                .filter(|(pane, _)| {
                    !said.contains(pane)
                        && both.iter().any(|r| r.pane == *pane && r.agent.is_some())
                })
                .map(|(pane, why)| {
                    (
                        pane.clone(),
                        format!("its new shell could not start ({why})"),
                    )
                }),
        );
        let closed: Vec<String> = {
            let mut topology = self.topology();
            let failed: BTreeSet<&str> = failures.iter().map(|(pane, _)| pane.as_str()).collect();
            // A pane closed while its shell started has nowhere to be.
            let closed = restarts
                .iter()
                .filter(|restart| {
                    !failed.contains(restart.pane.as_str()) && topology.pane(&restart.pane).is_err()
                })
                .map(|restart| restart.pane.clone())
                .collect();
            // The agents no resume will be typed for: the session names each
            // until an agent shows in its pane, and the core ends its
            // registration. Said before a failed pane takes its agent back,
            // which a pane that runs one cannot be said of.
            for (pane, why) in &left {
                let Some(agent) = both
                    .iter()
                    .find(|restart| restart.pane == *pane)
                    .and_then(|restart| restart.agent.as_ref())
                else {
                    continue;
                };
                if topology.not_resumed(
                    pane,
                    NotResumed {
                        pane_id: pane.clone(),
                        agent: agent.kind.clone(),
                        session: agent.session.clone(),
                        why: why.clone(),
                    },
                ) {
                    crate::diagnostic!(serde_json::json!({
                        "component": "own_engine",
                        "kind": "agent.not_resumed",
                        "pane_id": pane,
                        "agent": agent.kind,
                        "by_session": agent.session.is_some(),
                        "message": why,
                    }));
                }
            }
            for (pane, why) in &failures {
                if let Ok(found) = topology.pane_mut(pane) {
                    found.failed = Some(why.clone());
                    // The agent never ran again, so it is still the pane's:
                    // the next restore resumes it as this one would have.
                    found.agent = restarts
                        .iter()
                        .find(|restart| restart.pane == *pane)
                        .and_then(|restart| restart.agent.clone());
                }
            }
            if !orphans.is_empty() {
                let cwd = home.clone().unwrap_or_default();
                let gathered = orphans
                    .iter()
                    .map(|id| (id.clone(), Pane::new(&cwd, None)))
                    .collect();
                let tabs = topology.gather(gathered, GATHERED_LABEL);
                crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "engine.restore_gathered",
                    "panes": orphans,
                    "tabs": tabs,
                }));
            }
            closed
        };
        let _ = self.end_panes(&closed);
        crate::diagnostic!(serde_json::json!({
            "component": "own_engine",
            "kind": "engine.restored",
            "adopted": present.len().saturating_sub(orphans.len()),
            "closed": finished,
            "started": restarts.len(),
            "idle": idle.len(),
            "gathered": orphans.len(),
            "resumes": resumes.len(),
            "not_resumed": left.len(),
        }));
        self.changed();
        resumes
    }

    /// Whether the pane's shell is its terminal's foreground, so no program
    /// it started runs there. A system that does not say (Windows has no
    /// foreground group) or a holder that does not answer is not read as
    /// one, so nothing is typed over a program that may run.
    fn at_its_prompt(&self, pane: &str) -> bool {
        match self
            .shared
            .holders
            .processes(pane, hide_node_link::link_panes::ASK_WAIT)
        {
            Ok(processes) => processes
                .foreground
                .is_some_and(|foreground| foreground.group == processes.shell),
            Err(error) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "engine.restore_processes_unread",
                    "pane_id": pane,
                    "message": error.to_string(),
                }));
                false
            }
        }
    }

    /// A conversation an import gave back to its pane once the Herdr it
    /// came from stopped (PRD engine-bundle-4-herdr-removal D-8, B15) is
    /// resumed there at the pane's prompt, as any adopted pane's agent is;
    /// one whose pane runs another command (`busy`: adopted panes off their
    /// prompt, every adopted pane where the system does not say) resumes in
    /// a new tab of the same workspace, opened once the restore has
    /// finished. Either way the record's give-back is settled.
    fn settle_given_back(&self, busy: &[String]) -> Vec<Resume> {
        let Some(place) = self.shared.place.as_ref() else {
            return Vec::new();
        };
        let given = crate::herdr_import::given_back(&place.engine_dir);
        if given.is_empty() {
            return Vec::new();
        }
        let moved: Vec<(Restart, String)> = {
            let mut topology = self.topology();
            busy.iter()
                .filter(|pane| given.contains(*pane))
                .filter_map(|pane| {
                    let (workspace, _) = topology.pane_place(pane).ok()?;
                    let found = topology.pane_mut(pane).ok()?;
                    let agent = found.agent.take()?;
                    Some((
                        Restart {
                            pane: pane.clone(),
                            cwd: found.cwd.clone(),
                            agent: Some(agent),
                            previous: None,
                        },
                        workspace,
                    ))
                })
                .collect()
        };
        let restarts: Vec<Restart> = moved.iter().map(|(restart, _)| restart.clone()).collect();
        let plans = plans(
            &self.shared.definitions.get(),
            &place.launches,
            &restarts,
            BTreeSet::new(),
            &BTreeSet::new(),
        );
        let mut resumes = Vec::new();
        for (restart, workspace) in moved {
            match plans.get(&restart.pane) {
                Some(Plan::Resume(resume)) => resumes.push(Resume {
                    new_tab: Some(NewTab {
                        workspace,
                        cwd: restart.cwd,
                    }),
                    ..(**resume).clone()
                }),
                Some(Plan::Shell(why)) => crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "engine.restore_not_resumed",
                    "pane_id": restart.pane,
                    "message": why,
                })),
                None => {}
            }
        }
        if let Err(reason) = crate::herdr_import::wait_again(&place.engine_dir, std::iter::empty())
        {
            crate::diagnostic!(serde_json::json!({
                "component": "own_engine",
                "kind": "engine.restore_given_back_unsaved",
                "message": reason,
            }));
        }
        resumes
    }

    /// Starts every pane of `restarts`, [`STARTS_AT_ONCE`] at a time, each
    /// in its folder or in `home` when the folder is gone, opening with the
    /// line its plan says when its agent is not resumed.
    fn start_restarts(
        &self,
        restarts: &[Restart],
        plans: &BTreeMap<String, Plan>,
        home: Option<&str>,
    ) -> Restarted {
        let next = Mutex::new(restarts.iter());
        let failures = Mutex::new(Vec::new());
        let left = Mutex::new(Vec::new());
        let started = Mutex::new(BTreeSet::new());
        std::thread::scope(|scope| {
            for _ in 0..STARTS_AT_ONCE.min(restarts.len()) {
                scope.spawn(|| {
                    loop {
                        let Some(restart) = next
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .next()
                        else {
                            break;
                        };
                        let (cwd, moved) = self.start_folder(&restart.cwd, home);
                        let plan = plans.get(&restart.pane);
                        // An agent is resumed only in its own folder: its
                        // conversation belongs to it.
                        let notice = match (moved, plan) {
                            (Some(moved), Some(Plan::Resume(resume))) => {
                                Some(format!("{moved} {} was not resumed there.", resume.shown))
                            }
                            (Some(moved), Some(Plan::Shell(why))) => Some(format!("{moved} {why}")),
                            (moved @ Some(_), None) => moved,
                            (None, Some(Plan::Shell(why))) => Some(why.clone()),
                            (None, _) => None,
                        };
                        let resumable = notice.is_none() && matches!(plan, Some(Plan::Resume(_)));
                        if let (false, true, Some(notice)) =
                            (resumable, restart.agent.is_some(), &notice)
                        {
                            left.lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push((restart.pane.clone(), notice.clone()));
                        }
                        match self.start_pane_replacing(
                            &restart.pane,
                            None,
                            &cwd,
                            Env::new(),
                            notice.as_deref(),
                            restart.previous,
                        ) {
                            Ok(()) if resumable => {
                                started
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .insert(restart.pane.clone());
                            }
                            Ok(()) => {}
                            Err(error) => {
                                crate::diagnostic!(serde_json::json!({
                                    "component": "own_engine",
                                    "kind": "engine.restore_start_failed",
                                    "pane_id": restart.pane,
                                    "message": error.to_string(),
                                }));
                                failures
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .push((restart.pane.clone(), error.to_string()));
                            }
                        }
                    }
                });
            }
        });
        let started = started
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let resumes = restarts
            .iter()
            .filter(|restart| started.contains(&restart.pane))
            .filter_map(|restart| match plans.get(&restart.pane) {
                Some(Plan::Resume(resume)) => Some((**resume).clone()),
                _ => None,
            })
            .collect();
        // In `restarts` order, whichever thread said it first.
        let mut left = left
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        left.sort_by_key(|(pane, _)| restarts.iter().position(|restart| restart.pane == *pane));
        Restarted {
            failures: failures
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            resumes,
            left,
        }
    }

    /// Marks each resume whose agent's hook last said its turn was running or
    /// waiting on the operator in that conversation: the move or the
    /// machine's restart cut that turn off (D-22, B9). Read from the hook
    /// records on this machine; a device's engine reads none
    /// (`hooks_records`).
    fn mark_cut(&self, plans: &mut BTreeMap<String, Plan>) {
        let Some(records) = self
            .shared
            .place
            .as_ref()
            .and_then(|place| place.hooks_records.as_deref())
        else {
            return;
        };
        let catalog = self.shared.definitions.get();
        for plan in plans.values_mut() {
            if let Plan::Resume(resume) = plan {
                resume.cut = was_cut(&catalog, records, resume);
            }
        }
    }

    /// Types each of `resumes` into its pane, one pane after another, each
    /// once its shell is ready; one that cannot be typed is tried again once
    /// the others have had their turn ([`RESUME_ATTEMPTS`]), and one that
    /// still cannot leaves its pane a shell, logs why and is named by the
    /// session as an agent that was not resumed. A cut resume is owed its
    /// letter from before its line is typed, so the agent's first hook report
    /// never comes first. One that was resting is not typed ([`Self::rests`]).
    /// An engine its owner lets go ends the pass: each resume it had yet to
    /// type is still its pane's saved agent, which the next restore resumes.
    pub(super) fn resume_agents(engine: &std::sync::Weak<super::Shared>, resumes: &[Resume]) {
        let mut shell = None;
        let mut turn: Vec<Resume> = resumes.to_vec();
        for attempt in 1..=RESUME_ATTEMPTS {
            let mut again = Vec::new();
            for resume in &turn {
                let Some(shared) = engine.upgrade() else {
                    return;
                };
                let engine = OwnEngine {
                    shared,
                    gates: Vec::new(),
                    saver: None,
                };
                if engine.check().is_err() {
                    return;
                }
                if engine.rests(resume) {
                    continue;
                }
                // A restarted pane runs the shell a pane with no program runs:
                // it marks its first prompt when that is the integration's, and
                // the resume waits for the mark as a start into a tab the core
                // just opened with it does.
                let shell = shell
                    .get_or_insert_with(|| {
                        ShellStart::of_tab(true, engine.shared.holders.shell_launch().as_ref())
                    })
                    .clone();
                let cut = || {
                    engine
                        .shared
                        .cut
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                };
                let opened;
                let resume = match &resume.new_tab {
                    None => resume,
                    Some(tab) => match engine.open_tab(
                        &tab.workspace,
                        None,
                        &resume.shown,
                        None,
                        false,
                        &tab.cwd,
                        Env::new(),
                        None,
                    ) {
                        Ok((_, pane)) => {
                            opened = Resume {
                                pane,
                                new_tab: None,
                                ..resume.clone()
                            };
                            &opened
                        }
                        Err(error) => {
                            engine.wait_again(resume, &error.to_string());
                            continue;
                        }
                    },
                };
                if resume.cut {
                    cut().insert(resume.pane.clone(), resume.kind.clone());
                }
                match engine.resume_agent(resume, shell) {
                    Ok(by_name) => crate::diagnostic!(serde_json::json!({
                        "component": "own_engine",
                        "kind": "agent.resumed",
                        "pane_id": resume.pane,
                        "agent": resume.kind,
                        "by_session": resume.session.is_some(),
                        "by_file": by_name.is_none(),
                        "by_name_cause": by_name.map(ByName::as_str),
                        "cut": resume.cut,
                        "attempt": attempt,
                    })),
                    Err(why) => {
                        // An agent that never resumed is owed nothing.
                        cut().remove(&resume.pane);
                        let last = attempt == RESUME_ATTEMPTS;
                        crate::diagnostic!(serde_json::json!({
                            "component": "own_engine",
                            "kind": "agent.resume_not_typed",
                            "pane_id": resume.pane,
                            "agent": resume.kind,
                            "message": why,
                            "attempt": attempt,
                            "final": last,
                        }));
                        if last {
                            engine.leave_not_resumed(resume, &why);
                        } else {
                            // Into the tab the first try opened, if it did.
                            again.push(resume.clone());
                        }
                    }
                }
            }
            if again.is_empty() {
                return;
            }
            turn = again;
        }
    }

    /// Names the agent of `resume`, which could not be typed into its pane,
    /// as one that was not resumed: the pane's shell stays as it is and the
    /// core ends the agent's registration. Only when the pane runs nothing
    /// of the agent: a resume whose typing is unconfirmed has left its start
    /// pending there, and that start is what settles it.
    fn leave_not_resumed(&self, resume: &Resume, why: &str) {
        let left = NotResumed {
            pane_id: resume.pane.clone(),
            agent: resume.kind.clone(),
            session: resume.session.clone(),
            why: format!("its resume could not be typed ({why})"),
        };
        if self.topology().not_resumed(&resume.pane, left.clone()) {
            self.say_not_resumed(&left);
            self.changed();
        }
    }

    /// Leaves the agent of `resume` resting in its pane when its turn was
    /// not cut off, and answers whether it did: started again, every agent
    /// of a machine that was running dozens would load it at once, while
    /// one that rested is waited on by no one. The core keeps it as a
    /// sleeping agent the operator's visit wakes (`Topology::rest`). Only
    /// this machine's hook files say a turn was cut (`mark_cut`), so a
    /// device's agents, a given-back conversation with no pane yet, and one
    /// with no session to wake into are started again as before.
    fn rests(&self, resume: &Resume) -> bool {
        let local = self
            .shared
            .place
            .as_ref()
            .is_some_and(|place| place.hooks_records.is_some());
        if !local || resume.cut || resume.new_tab.is_some() || resume.session.is_none() {
            return false;
        }
        // Only an agent that sleeps in its pane can wait there; one whose
        // sleep closes its pane, or that cannot sleep, starts again.
        let catalog = self.shared.definitions.get();
        let kind = agents::definition(&catalog, &resume.kind)
            .map_or_else(|| resume.kind.clone(), agents::kind_name);
        let sleeps_in_pane = hide_agent_adapter::adapter(&kind)
            .and_then(|adapter| adapter.sleep)
            .is_some_and(|dialect| !dialect.closes_pane_when_sleeping());
        if !sleeps_in_pane {
            return false;
        }
        let agent = PaneAgent {
            kind: resume.kind.clone(),
            name: resume.name.clone(),
            session: resume.session.clone(),
            ..PaneAgent::default()
        };
        if !self.topology().rest(&resume.pane, agent) {
            return false;
        }
        crate::diagnostic!(serde_json::json!({
            "component": "own_engine",
            "kind": "agent.resume_rested",
            "pane_id": resume.pane,
            "agent": resume.kind,
        }));
        self.changed();
        true
    }

    /// A given-back conversation whose tab could not open (`why`) waits
    /// for the next import again, so it is offered rather than lost.
    fn wait_again(&self, resume: &Resume, why: &str) {
        crate::diagnostic!(serde_json::json!({
            "component": "own_engine",
            "kind": "agent.resume_tab_unopened",
            "pane_id": resume.pane,
            "agent": resume.kind,
            "message": why,
        }));
        let Some(place) = self.shared.place.as_ref() else {
            return;
        };
        if let Err(reason) = crate::herdr_import::wait_again(
            &place.engine_dir,
            [(
                resume.pane.clone(),
                resume.kind.clone(),
                resume.name.clone(),
                resume.session.clone(),
            )],
        ) {
            crate::diagnostic!(serde_json::json!({
                "component": "own_engine",
                "kind": "engine.restore_given_back_unsaved",
                "message": reason,
            }));
        }
    }

    /// Waits for `resume`'s pane's shell as every start into a shell waits
    /// for it, as `shell` says it has finished starting, then records the
    /// agent as started and types its resume, its program named by the file
    /// the pane's node starts it from, as a wake's is
    /// (`start_prompt::start_program_file`, asked about the definition's
    /// first executable, the resume's first word); why it went by its name
    /// instead is what it answers, `None` when the file named it.
    fn resume_agent(&self, resume: &Resume, shell: ShellStart) -> Result<Option<ByName>, String> {
        let node = self.node().ok().map(|node| &**node as &dyn NodeLink);
        let file = match node.zip(resume.argv.first()) {
            Some((node, program)) => crate::start_prompt::start_program_file(node, program),
            None => Err(ByName::NodeUnanswered),
        };
        // A profile its machine no longer has, or refuses now, resumes
        // nothing: the pane stays a shell and says why (D-27, B20).
        let launch = crate::launch::resolve(
            node,
            &crate::launch::Request::recorded(&resume.kind, resume.profile.as_ref()),
        )
        .map_err(|refusal| refusal.detail)?;
        let argv =
            agents::named_by_file(resume.argv.clone(), launch.agent_file(file.as_ref().ok()));
        let by_name = (argv.first() == resume.argv.first())
            .then(|| file.as_ref().err().copied().unwrap_or(ByName::AsNamed));
        let line = launch.line(argv)?;
        let argv = line.words;
        // The longest of the shells' lines, as a start is bounded.
        let longest =
            shell_line::longest(|shell| shell_line::resume_command(&argv, &resume.shown, shell))
                .ok_or_else(|| "the resume names no program".to_owned())?;
        crate::agent_start::wait_for_shell(
            self,
            node,
            &format!("restore:{}", resume.pane),
            &resume.pane,
            shell,
            longest,
        )
        .map_err(|error| match error {
            StartError::NotStarted { detail, .. } => detail,
            StartError::ShellStarting { waited, screen } => {
                crate::agent_start::shell_starting_message(waited, screen.as_deref())
            }
            StartError::Engine(error) => error.to_string(),
        })?;
        let group = self
            .processes(&resume.pane)
            .map_err(|error| error.to_string())?;
        let shell = shell_line::shell_of(group.shell_name(), self.shared.windows)
            .ok_or_else(|| "the pane's shell is cmd.exe, which takes no resume line".to_owned())?;
        let typed = shell_line::resume_command(&argv, &resume.shown, shell)
            .ok_or_else(|| "the resume names no program".to_owned())?;
        // A profile's resume ends with what came of its attempt, read on a
        // thread of its own so the restore goes on to the next pane; until
        // that thread has it, whatever ends this call ends the attempt.
        let finish = line
            .attempt
            .as_ref()
            .map(|attempt| super::attempt::Finish::new(self, &resume.pane, &attempt.id));
        self.type_agent_start(
            &resume.pane,
            PaneAgent::resumed(&resume.kind, resume.name.as_deref(), resume.session.clone()),
            &typed,
            false,
            line.attempt.clone(),
        )
        .map_err(|error| error.to_string())?;
        if let Some(finish) = finish {
            self.observe_resume(resume, finish);
        }
        Ok(by_name)
    }

    /// Reads what came of the attempt of `resume`'s profile start until the
    /// start's deadline, on a thread of its own, and leaves a failure as the
    /// restore's result for the pane: a program its executor could not start
    /// is named as not resumed, with the executor's sentence. A result that
    /// stays unknown starts nothing again; the pane's facts decide, as for
    /// any typed start.
    pub(super) fn observe_resume(&self, resume: &Resume, finish: super::attempt::Finish<'_, '_>) {
        let (pane, id) = finish.hand_off();
        let shared = Arc::clone(&self.shared);
        let watched = resume.clone();
        let (thread_pane, thread_id) = (pane.clone(), id.clone());
        let spawned = std::thread::Builder::new()
            .name("hide-resume-attempt".to_owned())
            .spawn(move || {
                let engine = OwnEngine {
                    shared,
                    gates: Vec::new(),
                    saver: None,
                };
                let finish = super::attempt::Finish::new(&engine, &thread_pane, &thread_id);
                let outcome = engine.await_attempt(finish, Instant::now() + agents::START_DEADLINE);
                engine.resumed(&watched, outcome);
            });
        if let Err(error) = spawned {
            crate::diagnostic!(serde_json::json!({
                "component": "own_engine",
                "kind": "agent.resume_unobserved",
                "pane_id": pane,
                "message": error.to_string(),
            }));
            let outcome = self.finish_attempt(&pane, &id);
            self.resumed(resume, outcome);
        }
    }

    /// What a profile's resume came to, for the restore.
    fn resumed(&self, resume: &Resume, outcome: super::attempt::Outcome) {
        match outcome {
            super::attempt::Outcome::Started => {}
            super::attempt::Outcome::Failed { failure, .. } => {
                let left = NotResumed {
                    pane_id: resume.pane.clone(),
                    agent: resume.kind.clone(),
                    session: resume.session.clone(),
                    why: format!("its profile's program did not start ({failure})"),
                };
                if self.topology().not_resumed(&resume.pane, left.clone()) {
                    self.say_not_resumed(&left);
                    self.changed();
                }
            }
            super::attempt::Outcome::NotStarted { why } => {
                let left = NotResumed {
                    pane_id: resume.pane.clone(),
                    agent: resume.kind.clone(),
                    session: resume.session.clone(),
                    why: format!("its executor was not admitted ({why}); retry the resume"),
                };
                if self.topology().not_resumed(&resume.pane, left.clone()) {
                    self.say_not_resumed(&left);
                    self.changed();
                }
            }
            super::attempt::Outcome::Unconfirmed { why } => crate::diagnostic!(serde_json::json!({
                "component": "own_engine",
                "kind": "agent.resume_unconfirmed",
                "pane_id": resume.pane,
                "agent": resume.kind,
                "why": why,
            })),
        }
    }

    /// Where a saved pane whose folder was `cwd` starts again: there, or in
    /// `home` with a line saying why when the node says no folder is there.
    /// A node that cannot answer is not read as a folder that is gone: the
    /// shell starts in its folder, and a start that fails there marks the
    /// pane with why.
    fn start_folder(&self, cwd: &str, home: Option<&str>) -> (String, Option<String>) {
        let found = self
            .node()
            .map_err(|error| error.to_string())
            .and_then(|node| {
                call_as::<Option<String>>(
                    node.as_ref(),
                    Call::Directory {
                        path: cwd.to_owned(),
                    },
                    FOLDER_WAIT,
                )
                .map_err(|error| error.to_string())
            });
        match (found, home.map(str::to_owned)) {
            (Ok(None), Some(home_cwd)) => (
                home_cwd,
                Some(format!(
                    "hide: the folder {cwd} is gone, so this shell opened in your home folder."
                )),
            ),
            (Err(why), _) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "engine.restore_folder_unread",
                    "cwd": cwd,
                    "message": why,
                }));
                (cwd.to_owned(), None)
            }
            _ => (cwd.to_owned(), None),
        }
    }

    /// Every saved pane, when the node never said which run: none is
    /// started over a holder that may still run.
    fn mark_unrestored(&self, saved: &BTreeSet<String>, why: &str) {
        {
            let mut topology = self.topology();
            for pane in saved {
                if let Ok(found) = topology.pane_mut(pane) {
                    found.failed = Some(why.to_owned());
                }
            }
        }
        self.changed();
    }
}
