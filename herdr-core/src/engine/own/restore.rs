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
//! again. An adopted pane whose saved agent is not running, its shell the
//! terminal's foreground, is resumed the same way: the daemon that typed
//! its resume ended before the line reached the shell, or the agent ended
//! while no daemon ran.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use hide_agent_adapter::definition::Catalog;
use hide_node_link::holders::HolderRecovery;
use hide_node_link::protocol::Call;

use super::agents::{self, PaneAgent};
use super::shell_line::{self, Shell};
use super::starts::STARTS_AT_ONCE;
use super::store::{self, Read};
use super::topology::{Pane, Topology};
use super::{EngineError, Env, OwnEngine, holder_error};
use crate::agent_start::{ShellStart, StartError};
use crate::model::PaneLaunch;
use crate::node_access::{NodeLink, call_as};
use crate::runtime::agent_choice::{choice_arguments, launch_describes};

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

/// What a restarted pane's agent gets: its resume, typed once its shell is
/// ready, or the line its shell opens with saying why not.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Plan {
    Resume(Resume),
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
/// engine-bundle-4-herdr-removal D-24); an agent with no such record resumes
/// on its CLI's defaults.
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
                        Ok(()) => Plan::Resume(Resume {
                            pane: restart.pane.clone(),
                            kind: definition.id.clone(),
                            shown,
                            name: agent.name.clone(),
                            session: agent.session.clone(),
                            argv,
                            cut: false,
                            new_tab: None,
                        }),
                    },
                }
            }
        };
        plans.insert(restart.pane.clone(), plan);
    }
    plans
}

/// Whether `resume`'s agent was cut off mid-turn: the last record its hook
/// wrote in the pane's file under `home` for that agent, and for its
/// conversation when both name one, says its turn was running or waiting on
/// the operator. A pane no hook wrote, or an agent that rested, was not.
fn was_cut(catalog: &Catalog, home: &Path, resume: &Resume) -> bool {
    hide_agent_hooks::counters::records(home, &resume.pane)
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
/// choice the agent does not take is refused with why, as a restart Hide
/// makes of it on Herdr refuses it, rather than resumed on another model.
fn with_launch_choice(
    mut argv: Vec<String>,
    launches: &BTreeMap<String, PaneLaunch>,
    restart: &Restart,
    agent: &PaneAgent,
) -> Result<Vec<String>, String> {
    let Some(launch) = launches.get(&restart.pane).filter(|launch| {
        launch_describes(
            launch,
            &agent.kind,
            agent.name.as_deref(),
            agent.session.as_deref(),
        )
    }) else {
        return Ok(argv);
    };
    argv.extend(
        choice_arguments(&agent.kind, &launch.choice)
            .map_err(|why| format!("its saved model and effort do not fit it ({why})"))?,
    );
    Ok(argv)
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
            // shell until its resume is typed.
            let restarts: Vec<Restart> = restarted
                .into_iter()
                .filter_map(|id| {
                    let pane = topology.pane_mut(&id).ok()?;
                    Some(Restart {
                        cwd: pane.cwd.clone(),
                        agent: pane.agent.take(),
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
                    Some(Restart {
                        cwd: pane.cwd.clone(),
                        agent: pane.agent.take(),
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
        let (failures, mut resumes) = self.start_restarts(&restarts, &plans, home.as_deref());
        resumes.extend(given_back);
        for restart in &idle {
            match plans.get(&restart.pane) {
                Some(Plan::Resume(resume)) => resumes.push(resume.clone()),
                Some(Plan::Shell(why)) => crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "engine.restore_not_resumed",
                    "pane_id": restart.pane,
                    "message": why,
                })),
                None => {}
            }
        }
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
            for (pane, why) in &failures {
                if let Ok(found) = topology.pane_mut(pane) {
                    found.failed = Some(why.clone());
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
                    ..resume.clone()
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
    /// line its plan says when its agent is not resumed: the panes that
    /// could not start, with why, and the agents to resume, in `restarts`
    /// order.
    fn start_restarts(
        &self,
        restarts: &[Restart],
        plans: &BTreeMap<String, Plan>,
        home: Option<&str>,
    ) -> (Vec<(String, String)>, Vec<Resume>) {
        let next = Mutex::new(restarts.iter());
        let failures = Mutex::new(Vec::new());
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
                Some(Plan::Resume(resume)) => Some(resume.clone()),
                _ => None,
            })
            .collect();
        (
            failures
                .into_inner()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            resumes,
        )
    }

    /// Marks each resume whose agent's hook last said its turn was running or
    /// waiting on the operator in that conversation: the move or the
    /// machine's restart cut that turn off (D-22, B9). Read from the hook
    /// files on this machine; a device's engine reads none (`hooks_home`).
    fn mark_cut(&self, plans: &mut BTreeMap<String, Plan>) {
        let Some(home) = self
            .shared
            .place
            .as_ref()
            .and_then(|place| place.hooks_home.as_deref())
        else {
            return;
        };
        let catalog = self.shared.definitions.get();
        for plan in plans.values_mut() {
            if let Plan::Resume(resume) = plan {
                resume.cut = was_cut(&catalog, home, resume);
            }
        }
    }

    /// Types each of `resumes` into its pane, one pane after another, each
    /// once its shell is ready; one that cannot be typed leaves its pane a
    /// shell and logs why. A cut resume is owed its letter from before its
    /// line is typed, so the agent's first hook report never comes first.
    pub(super) fn resume_agents(engine: &std::sync::Weak<super::Shared>, resumes: &[Resume]) {
        let mut shell = None;
        for resume in resumes {
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
            // A restarted pane runs the shell a pane with no program runs: it
            // marks its first prompt when that is the integration's, and the
            // resume waits for the mark as a start into a tab the core just
            // opened with it does.
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
                Ok(()) => crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "agent.resumed",
                    "pane_id": resume.pane,
                    "agent": resume.kind,
                    "by_session": resume.session.is_some(),
                    "cut": resume.cut,
                })),
                Err(why) => {
                    // An agent that never resumed is owed nothing.
                    cut().remove(&resume.pane);
                    crate::diagnostic!(serde_json::json!({
                        "component": "own_engine",
                        "kind": "agent.resume_not_typed",
                        "pane_id": resume.pane,
                        "agent": resume.kind,
                        "message": why,
                    }));
                }
            }
        }
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
    /// agent as started and types its resume.
    fn resume_agent(&self, resume: &Resume, shell: ShellStart) -> Result<(), String> {
        // The longest of the shells' lines, as a start is bounded.
        let longest = shell_line::longest(|shell| {
            shell_line::resume_command(&resume.argv, &resume.shown, shell)
        })
        .ok_or_else(|| "the resume names no program".to_owned())?;
        let node = self.node().ok().map(|node| &**node as &dyn NodeLink);
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
            StartError::ShellStarting(line) => {
                crate::agent_start::shell_starting_message(line.as_deref())
            }
            StartError::Herdr(error) => error.to_string(),
        })?;
        let group = self
            .processes(&resume.pane)
            .map_err(|error| error.to_string())?;
        let shell = Shell::of(group.shell_name(), self.shared.windows)
            .ok_or_else(|| "the pane's shell is cmd.exe, which takes no resume line".to_owned())?;
        let line = shell_line::resume_command(&resume.argv, &resume.shown, shell)
            .ok_or_else(|| "the resume names no program".to_owned())?;
        // A restart's resume is a wake of the agent.
        let awaits_report = super::agents::definition(&self.shared.definitions.get(), &resume.kind)
            .is_some_and(|definition| {
                super::agents::awaits_session_report(definition, &resume.argv, true)
            });
        self.type_agent_start(
            &resume.pane,
            PaneAgent::started(
                &resume.kind,
                resume.name.as_deref(),
                resume.session.clone(),
                awaits_report,
            ),
            &line,
        )
        .map_err(|error| error.to_string())
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
