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
//! each when its shell holds the terminal, and no conversation is resumed in
//! two panes: a second pane with the same session (or, for `last`, the same
//! agent in the same folder) or one an adopted pane still runs stays a shell.
//! A pane that cannot be resumed opens with one line saying why, and a resume
//! whose agent ends with a failure says so in the pane, which is a shell
//! again.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hide_agent_adapter::definition::Catalog;
use hide_node_link::holders::HolderRecovery;
use hide_node_link::protocol::Call;

use super::agents::{self, PaneAgent};
use super::shell_line::{self, Shell};
use super::store::{self, Read};
use super::topology::{Pane, Topology};
use super::{EngineError, Env, OwnEngine};
use crate::node_access::call_as;

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

/// How many holders a restore starts at once, so a large session coming
/// back after a reboot does not start every shell together.
const STARTS_AT_ONCE: usize = 8;

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

/// How often a resume reads whether its pane's shell holds the terminal.
const SHELL_POLL: Duration = Duration::from_millis(100);

/// Starts the restore of the panes `saved` names on a thread of its own.
pub(super) fn start(engine: OwnEngine<'static>, saved: BTreeSet<String>) -> Result<(), String> {
    std::thread::Builder::new()
        .name("hide-own-engine-restore".into())
        .spawn(move || {
            let gate = engine.restore_gate();
            let resumes = engine.restore(&saved);
            gate.finish();
            // Calls that make panes no longer wait for these: each agent
            // waits for its own shell, one pane after another.
            engine.resume_agents(&resumes);
        })
        .map(|_| ())
        .map_err(|error| format!("the session restore thread could not start: {error}"))
}

/// One saved pane to start again: its id, the folder it was saved in, and
/// the agent it ran.
struct Restart {
    pane: String,
    cwd: String,
    agent: Option<PaneAgent>,
}

/// What a restarted pane's agent gets: its resume, typed once its shell
/// holds the terminal, or the line its shell opens with saying why not.
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
}

/// What each restarted pane's saved agent gets, in `restarts` order: a
/// conversation is resumed in one pane only, and never in a restarted pane
/// while an adopted pane runs it (`running`, by [`conversation`]).
fn plans(
    catalog: &Catalog,
    restarts: &[Restart],
    mut running: BTreeSet<(String, String)>,
) -> BTreeMap<String, Plan> {
    let mut plans = BTreeMap::new();
    for restart in restarts {
        let Some(agent) = &restart.agent else {
            continue;
        };
        let plan = match agents::definition(catalog, &agent.kind) {
            None => Plan::Shell(format!(
                "hide: no agent definition names {} any more, so its conversation was not resumed; this pane is a shell now.",
                agent.kind
            )),
            Some(definition) => {
                let shown = definition.name.clone();
                match agents::resume_argv(definition, agent.session.as_deref()) {
                    None => Plan::Shell(format!(
                        "hide: {shown} says no way to resume a conversation, so this pane is a shell now."
                    )),
                    Some(argv) => match shell_line::check_resume(&argv) {
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
                        }),
                    },
                }
            }
        };
        plans.insert(restart.pane.clone(), plan);
    }
    plans
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
    fn restore(&self, saved: &BTreeSet<String>) -> Vec<Resume> {
        let recovery = match self.shared.holders.recovered(RECOVERY_WAIT) {
            Ok(recovery) => recovery,
            Err(error) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "engine.restore_unfinished",
                    "message": error.to_string(),
                }));
                self.mark_unrestored(
                    saved,
                    &format!("the node did not say which panes still run: {error}"),
                );
                return Vec::new();
            }
        };
        self.adopt(saved, &recovery)
    }

    /// Lays the panes `saved` names out around what `recovery` found: the
    /// agents to resume in the panes that started again.
    fn adopt(&self, saved: &BTreeSet<String>, recovery: &HolderRecovery) -> Vec<Resume> {
        let present: BTreeSet<&str> = recovery.present.iter().map(String::as_str).collect();
        let home = self.home().ok();
        let (finished, restarts, orphans, running) = {
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
            let placed: BTreeSet<String> = topology.panes().map(|(id, _)| id.to_owned()).collect();
            let restarted: Vec<String> = topology
                .panes()
                .filter(|(id, _)| saved.contains(*id) && !present.contains(id))
                .map(|(id, _)| id.to_owned())
                .collect();
            // A restarted pane's agent ended with its holder: the pane is a
            // shell until its resume is typed.
            let restarts: Vec<Restart> = restarted
                .into_iter()
                .filter_map(|id| {
                    let pane = topology.pane_mut(&id).ok()?;
                    Some(Restart {
                        cwd: pane.cwd.clone(),
                        agent: pane.agent.take(),
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
            let orphans: Vec<String> = recovery
                .present
                .iter()
                .filter(|id| !placed.contains(*id))
                .cloned()
                .collect();
            (finished, restarts, orphans, running)
        };
        let plans = plans(&self.shared.definitions.get(), &restarts, running);
        let (failures, resumes) = self.start_restarts(&restarts, &plans, home.as_deref());
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
                let cwd = home
                    .as_deref()
                    .and_then(|home| hide_platform::path::to_wire(home).ok())
                    .unwrap_or_default();
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
            "gathered": orphans.len(),
            "resumes": resumes.len(),
        }));
        self.changed();
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
        home: Option<&Path>,
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
                            (Some(moved), Some(Plan::Resume(resume))) => Some(format!(
                                "{moved} {} was not resumed there.",
                                resume.shown
                            )),
                            (Some(moved), Some(Plan::Shell(why))) => Some(format!("{moved} {why}")),
                            (moved @ Some(_), None) => moved,
                            (None, Some(Plan::Shell(why))) => Some(why.clone()),
                            (None, _) => None,
                        };
                        let resumable = notice.is_none()
                            && matches!(plan, Some(Plan::Resume(_)));
                        match self.start_pane(
                            &restart.pane,
                            None,
                            &cwd,
                            Env::new(),
                            notice.as_deref(),
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

    /// Types each of `resumes` into its pane, one pane after another, each
    /// once its shell holds the terminal; one that cannot be typed leaves its
    /// pane a shell and logs why.
    pub(super) fn resume_agents(&self, resumes: &[Resume]) {
        for resume in resumes {
            if self.check().is_err() {
                return;
            }
            match self.resume_agent(resume) {
                Ok(()) => crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "agent.resumed",
                    "pane_id": resume.pane,
                    "agent": resume.kind,
                    "by_session": resume.session.is_some(),
                })),
                Err(why) => crate::diagnostic!(serde_json::json!({
                    "component": "own_engine",
                    "kind": "agent.resume_not_typed",
                    "pane_id": resume.pane,
                    "agent": resume.kind,
                    "message": why,
                })),
            }
        }
    }

    /// Waits up to [`crate::agent_start::SHELL_WAIT`] for `resume`'s pane's
    /// shell to hold the terminal alone, then records the agent as started
    /// and types its resume.
    #[allow(clippy::disallowed_methods)] // a production wait, not test code
    fn resume_agent(&self, resume: &Resume) -> Result<(), String> {
        let deadline = Instant::now() + crate::agent_start::SHELL_WAIT;
        let shell = loop {
            match self.processes(&resume.pane) {
                Ok(group) if group.shell_holds_terminal() => {
                    break Shell::of(group.shell_name());
                }
                Ok(_) => {}
                Err(EngineError::Refused { code, .. }) if code == "pane_not_found" => {
                    return Err("the pane closed before its shell was ready".to_owned());
                }
                Err(_) => {}
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(format!(
                    "the shell did not hold the terminal within {} s",
                    crate::agent_start::SHELL_WAIT.as_secs()
                ));
            }
            std::thread::sleep(SHELL_POLL.min(deadline - now));
        };
        let line = shell_line::resume_command(&resume.argv, &resume.shown, shell)
            .ok_or_else(|| "the resume names no program".to_owned())?;
        {
            let catalog = self.shared.definitions.get();
            let mut topology = self.topology();
            if resume.name.as_deref().is_some_and(|name| topology.name_taken(name, &resume.pane)) {
                return Err("another pane runs an agent under its name".to_owned());
            }
            topology
                .start_agent(
                    &resume.pane,
                    PaneAgent::started(&resume.kind, resume.name.as_deref(), resume.session.clone()),
                )
                .map_err(|error| error.to_string())?;
            drop(topology);
            self.give_rules(&resume.pane, Some(&resume.kind), &catalog);
        }
        self.changed();
        self.type_bytes(&resume.pane, format!("{line}\r").into_bytes())
            .map_err(|error| error.to_string())
    }

    /// Where a saved pane whose folder was `cwd` starts again: there, or in
    /// `home` with a line saying why when the node says no folder is there.
    /// A node that cannot answer is not read as a folder that is gone: the
    /// shell starts in its folder, and a start that fails there marks the
    /// pane with why.
    fn start_folder(&self, cwd: &str, home: Option<&Path>) -> (String, Option<String>) {
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
        let home_cwd = home.and_then(|home| hide_platform::path::to_wire(home).ok());
        match (found, home_cwd) {
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
