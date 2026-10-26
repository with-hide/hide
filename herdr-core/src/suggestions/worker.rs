//! The suggestion worker (PRD hide-ai-suggestions B4-B9, B16, B17, B20):
//! one named thread that holds the suggestion store's only connection, so
//! every record, edit and lifecycle move has one writer, and does all of it
//! outside `Mutex<Runtime>`.
//!
//! The runtime hands it commands through a bounded queue it never waits on;
//! a full queue drops the command with a diagnostic (engineering principle
//! 15). A day's analysis runs on a thread of its own and writes the notebook
//! through a pen whose every edit comes back here to be checked and saved,
//! so the worker stays free to record and answer while a run takes minutes.

use super::analysis::{
    Analysis, AnalysisBudget, AnalysisEnd, AnalysisInput, NotebookPen, PartialReason, PenRefused,
    StopFlag, TagGrouping, Written,
};
use super::apply::{self as applying, ApplyOrder, FollowedAttempt};
use super::ledger::{Observation, RecordIndex, SessionRef, Signal};
use super::lifecycle::{AttemptMove, AttemptPhase};
use super::notebook::{LineId, Notebook, NotebookEdit, Outcome, Stage, Strike, StruckBy};
use super::policy::{self, Quantity};
use super::read::{self, ReadInput};
use super::scoring;
use super::settings::ApplyMode;
use super::store::{ProjectLocation, SuggestionStore};
use super::watch::{self as watching, WatchedLine};
use crate::model::ProjectSuggestionsSnapshot;
use crate::node::NodeId;
use crate::node_access::NodeLink;
use serde_json::json;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread;
use std::time::{Duration, Instant};

/// How long the worker waits for nothing in particular before it looks at
/// the clock again.
const IDLE: Duration = Duration::from_secs(60);
/// The meta key the one-time intro line is remembered under (B3).
const INTRO_SEEN: &str = "intro_seen";
/// How long a node is given to name a folder's Project.
const IDENTITY_TIMEOUT: Duration = Duration::from_secs(5);

/// The current time in Unix milliseconds; a test passes its own.
pub(crate) type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;
/// Makes the analysis a run uses (D-49): production's investigates with
/// Hide AI when its gate is open and groups by tag otherwise.
pub(crate) type AnalysisFactory = Arc<dyn Fn(&RunPlan<'_>) -> Box<dyn Analysis> + Send + Sync>;

/// What a run's analysis is chosen and made with.
pub(crate) struct RunPlan<'a> {
    pub project_id: &'a str,
    pub gate: RunGate,
    /// The run's stop, which an analysis that waits ends its wait on.
    pub stop: &'a StopFlag,
}

/// A folder on a machine whose Project a command is about.
#[derive(Clone, Debug)]
pub(crate) struct ProjectRef {
    pub node: NodeId,
    pub path: String,
}

/// Names a folder's Project with the link to its node, when the node is
/// reachable; production asks the node.
pub(crate) type Resolver =
    Box<dyn FnMut(&ProjectRef, Option<Arc<dyn NodeLink>>) -> Result<String, String> + Send>;

/// The resolver that asks the folder's node (`hide_project`).
pub(crate) fn node_resolver() -> Resolver {
    Box::new(|project: &ProjectRef, link: Option<Arc<dyn NodeLink>>| {
        let link = link.ok_or_else(|| "node_unavailable".to_owned())?;
        crate::memory_hook::project_identity(
            link.as_ref(),
            &project.node,
            &project.path,
            IDENTITY_TIMEOUT,
        )
        .map(|identity| identity.id)
    })
}

/// A change to a Project's settings, one per event (B3, B5, D-09, D-28).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SettingsChange {
    /// The head switch went off, with whether Memory was on as it did.
    HideAiOff {
        memory_on: bool,
    },
    /// The head switch went on; Memory comes back on for `workspace_id`
    /// when the switch was what turned it off.
    HideAiOn {
        workspace_id: String,
    },
    Suggestions(bool),
    Quantity(Quantity),
    ApplyMode(ApplyMode),
    Signal(Signal, bool),
}

/// An operator's action on a line (B16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineAction {
    /// `안 함`: the line is struck and its new records never raise it.
    Dismiss,
    /// ⚙'s `되살리기`: the strike goes and the line is judged again.
    Restore,
    /// `PR 만들기` or `이슈 만들기` on a new suggestion (B11, B14).
    Apply,
    /// `다시 시도` on an attempt that failed (B12).
    Retry,
}

pub(crate) enum Command {
    Open {
        workspace_id: String,
        project: ProjectRef,
    },
    Close,
    Observe {
        project: ProjectRef,
        facts: Vec<Observation>,
    },
    /// Facts about a Project whose identity the source already holds
    /// (Memory's).
    ObserveProject {
        project_id: String,
        facts: Vec<Observation>,
    },
    Settings {
        project: ProjectRef,
        change: SettingsChange,
    },
    Action {
        project: ProjectRef,
        line: LineId,
        action: LineAction,
    },
    IntroSeen,
    /// The app opened, the device woke or the screen unlocked (B6).
    Wake,
    Pen {
        project: String,
        edit: NotebookEdit,
        reply: mpsc::Sender<Result<(Written, Notebook), PenRefused>>,
    },
    RunEnded {
        project: String,
        end: AnalysisEnd,
        edits: usize,
    },
    /// A session the label worker saw in a pane, for the watch after a
    /// merge (B15).
    SessionSeen {
        project: ProjectRef,
        session: SessionRef,
        checkout: String,
        at_unix_ms: u64,
    },
    /// The checkouts that hold a watched line's merge, as the runtime
    /// judged them from the node's worktree facts.
    Holding {
        project_id: String,
        line: LineId,
        checkouts: BTreeSet<String>,
    },
    /// What the runtime saw happen to an attempt (B11-B14).
    Attempted {
        project_id: String,
        line: LineId,
        started_at_unix_ms: u64,
        change: AttemptMove,
    },
}

/// Whether a run that asks a provider may start (D-21, D-36), and the
/// language it writes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RunGate {
    /// Hide AI is on and its agent can investigate.
    pub ai_ready: bool,
    /// The operator's usage limit is not near.
    pub usage_ok: bool,
    pub language: hide_ai::OutputLanguage,
}

/// What the worker hands back to the runtime.
pub(crate) trait Sink: Send + 'static {
    /// The opened Project's tab, read again after anything moved it.
    fn read(&self, workspace_id: String, read: ProjectSuggestionsSnapshot);
    /// Whether a provider run may start now, taken under the lock.
    fn gate(&self) -> RunGate;
    /// Turns the Project's Memory back on, as the head switch found it.
    fn restore_memory(&self, workspace_id: String);
    /// The link to `node` while it is reachable, to name a folder's Project.
    fn link(&self, node: &NodeId) -> Option<Arc<dyn NodeLink>>;
    /// Starts an attempt: an agent for a pull request, or Hide's issue.
    fn apply(&self, order: ApplyOrder);
    /// Every attempt and watch still to follow, replacing the last lists.
    fn followed(&self, attempts: Vec<FollowedAttempt>, watches: Vec<WatchedLine>);
    /// Whether an automatic attempt may start in the Project at `location`
    /// now: GitHub answers there (D-41).
    fn can_apply(&self, location: &ProjectLocation, outcome: Outcome) -> bool;
}

pub(crate) struct Options {
    pub store: PathBuf,
    pub clock: Clock,
    pub zone: jiff::tz::TimeZone,
    pub analysis: AnalysisFactory,
    pub resolver: Resolver,
}

impl Options {
    pub(crate) fn production(store: PathBuf, analysis: AnalysisFactory) -> Self {
        Self {
            store,
            clock: Arc::new(crate::runtime::unix_milliseconds),
            zone: jiff::tz::TimeZone::system(),
            analysis,
            resolver: node_resolver(),
        }
    }
}

/// The handle the runtime keeps; dropping it stops the worker, ends a run
/// in flight and joins both.
pub(crate) struct SuggestionWorker {
    client: SuggestionClient,
    stop: StopFlag,
    join: Option<thread::JoinHandle<()>>,
}

#[derive(Clone)]
pub(crate) struct SuggestionClient(SyncSender<Command>);

impl SuggestionClient {
    /// A client whose commands a test reads instead of a worker.
    #[cfg(test)]
    pub(crate) fn for_test() -> (Self, Receiver<Command>) {
        let (sender, receiver) = mpsc::sync_channel(policy::QUEUE);
        (Self(sender), receiver)
    }

    /// Hands `command` over without waiting; a full queue or a stopped
    /// worker drops it with a diagnostic naming the command only.
    pub(crate) fn send(&self, command: Command) -> bool {
        let kind = match &command {
            Command::Open { .. } => "open",
            Command::Close => "close",
            Command::Observe { .. } | Command::ObserveProject { .. } => "observe",
            Command::Settings { .. } => "settings",
            Command::Action { .. } => "action",
            Command::IntroSeen => "intro_seen",
            Command::Wake => "wake",
            Command::Pen { .. } => "pen",
            Command::RunEnded { .. } => "run_ended",
            Command::Attempted { .. } => "attempted",
            Command::SessionSeen { .. } => "session_seen",
            Command::Holding { .. } => "holding",
        };
        match self.0.try_send(command) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                crate::diagnostic!(json!({
                    "component": "suggestions", "kind": "queue.full", "command": kind,
                    "limit": policy::QUEUE,
                }));
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                crate::diagnostic!(json!({
                    "component": "suggestions", "kind": "worker.stopped", "command": kind,
                }));
                false
            }
        }
    }
}

impl SuggestionClient {
    /// Hands `command` over, waiting for room: a run's own thread delivers
    /// its edits and its end this way, so a full queue delays them and never
    /// drops them, which would leave the run held forever. Never called
    /// under the runtime lock. A worker stopping a run drains its queue, so
    /// this answers false once the worker is gone rather than waiting on it.
    fn deliver(&self, command: Command) -> bool {
        self.0.send(command).is_ok()
    }
}

impl SuggestionWorker {
    pub(crate) fn spawn(options: Options, sink: impl Sink) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(policy::QUEUE);
        let stop = StopFlag::default();
        let worker_stop = stop.clone();
        let own = SuggestionClient(sender.clone());
        let join = thread::Builder::new()
            .name("hide-suggestions".into())
            .spawn(move || Worker::new(options, own, worker_stop).run(&receiver, &sink))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            client: SuggestionClient(sender),
            stop,
            join: Some(join),
        })
    }

    pub(crate) fn client(&self) -> SuggestionClient {
        self.client.clone()
    }
}

impl Drop for SuggestionWorker {
    fn drop(&mut self) {
        self.stop.set();
        // Wakes the worker from its wait; a full queue wakes it anyway.
        let _ = self.client.0.try_send(Command::Wake);
        if let Some(join) = self.join.take()
            && join.join().is_err()
        {
            crate::diagnostic!(json!({"component": "suggestions", "kind": "worker.join_failed"}));
        }
    }
}

struct RunHandle {
    project: String,
    index: RecordIndex,
    stop: StopFlag,
    join: thread::JoinHandle<()>,
    edits: usize,
}

struct Worker {
    options: Options,
    own: SuggestionClient,
    stop: StopFlag,
    store: Result<SuggestionStore, String>,
    opened: Option<(String, Option<String>)>,
    identities: HashMap<(NodeId, String), String>,
    run: Option<RunHandle>,
    /// When the day's runs start, once a wake asked for them.
    due_at: Option<u64>,
    /// Projects whose run is still to come in this round.
    round: Vec<String>,
    /// Projects whose location was saved in this run of the worker.
    located: HashSet<String>,
    /// When the watches are next looked at for their time.
    settle_at: u64,
}

impl Worker {
    fn new(options: Options, own: SuggestionClient, stop: StopFlag) -> Self {
        let store = SuggestionStore::open(&options.store).map(|(store, opened)| {
            if opened == super::store::Opened::Rebuilt {
                crate::diagnostic!(json!({"component": "suggestions", "kind": "store.rebuilt"}));
            }
            store
        });
        if let Err(code) = &store {
            crate::diagnostic!(
                json!({"component": "suggestions", "kind": "store.unavailable", "code": code})
            );
        }
        Self {
            options,
            own,
            stop,
            store,
            opened: None,
            identities: HashMap::new(),
            run: None,
            due_at: None,
            round: Vec::new(),
            located: HashSet::new(),
            settle_at: 0,
        }
    }

    fn now(&self) -> u64 {
        (self.options.clock)()
    }

    fn today(&self) -> String {
        local_day(self.now(), &self.options.zone)
    }

    fn run(mut self, receiver: &Receiver<Command>, sink: &impl Sink) {
        self.publish_followed(sink);
        loop {
            if self.stop.is_set() {
                self.finish_run_on_stop(receiver);
                return;
            }
            let wait = self.due_at.map_or(IDLE, |due| {
                Duration::from_millis(due.saturating_sub(self.now())).min(IDLE)
            });
            match receiver.recv_timeout(wait) {
                Ok(command) => self.handle(command, sink),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.advance_runs(sink);
            if self.now() >= self.settle_at {
                self.settle_at = self.now() + policy::SETTLE_EVERY_MS;
                self.settle_all(sink);
            }
        }
    }

    /// Ends a run in flight and joins it, answering its pen meanwhile so a
    /// write it is waiting on cannot hold the join.
    fn finish_run_on_stop(&mut self, receiver: &Receiver<Command>) {
        let Some(run) = self.run.take() else {
            return;
        };
        run.stop.set();
        while !run.join.is_finished() {
            if let Ok(Command::Pen { reply, .. }) = receiver.recv_timeout(Duration::from_millis(50))
            {
                let _ = reply.send(Err(PenRefused::Stopped));
            }
        }
        if run.join.join().is_err() {
            crate::diagnostic!(json!({"component": "suggestions", "kind": "run.join_failed"}));
        }
    }

    fn resolve(&mut self, project: &ProjectRef, sink: &impl Sink) -> Option<String> {
        let key = (project.node.clone(), project.path.clone());
        if let Some(id) = self.identities.get(&key) {
            return Some(id.clone());
        }
        match (self.options.resolver)(project, sink.link(&project.node)) {
            Ok(id) => {
                if self.identities.len() >= policy::IDENTITY_CACHE {
                    self.identities.clear();
                }
                self.identities.insert(key, id.clone());
                Some(id)
            }
            Err(reason) => {
                crate::diagnostic!(json!({
                    "component": "suggestions", "kind": "project.unresolved",
                    "device": project.node.as_str(), "reason": reason,
                }));
                None
            }
        }
    }

    fn handle(&mut self, command: Command, sink: &impl Sink) {
        match command {
            Command::Open {
                workspace_id,
                project,
            } => {
                let id = self.resolve(&project, sink);
                if let Some(id) = &id {
                    self.locate(id, &project);
                }
                self.opened = Some((workspace_id, id));
                self.publish(sink);
            }
            Command::Close => self.opened = None,
            Command::Observe { project, facts } => {
                let Some(id) = self.resolve(&project, sink) else {
                    return;
                };
                if self.observe(&id, facts) {
                    self.locate(&id, &project);
                }
            }
            Command::ObserveProject { project_id, facts } => {
                self.observe(&project_id, facts);
            }
            Command::Settings { project, change } => {
                let Some(id) = self.resolve(&project, sink) else {
                    return;
                };
                self.locate(&id, &project);
                self.change_settings(&id, change, sink);
                self.raise(&id);
                self.auto_apply(&id, sink);
                self.publish_if_opened(&id, sink);
            }
            Command::Action {
                project,
                line,
                action,
            } => {
                let Some(id) = self.resolve(&project, sink) else {
                    return;
                };
                self.locate(&id, &project);
                match action {
                    LineAction::Apply | LineAction::Retry => {
                        self.apply_line(&id, line, action == LineAction::Retry, false, sink);
                    }
                    LineAction::Dismiss | LineAction::Restore => self.act(&id, line, action),
                }
                self.raise(&id);
                self.auto_apply(&id, sink);
                self.publish_if_opened(&id, sink);
                self.publish_followed(sink);
            }
            Command::IntroSeen => {
                if let Ok(store) = self.store.as_mut()
                    && let Err(code) = store.set_meta(INTRO_SEEN, "1")
                {
                    log_store("intro_seen", &code);
                }
                self.publish(sink);
            }
            Command::Wake => {
                if !self.stop.is_set() && self.due_at.is_none() && self.run.is_none() {
                    self.due_at = Some(self.now() + policy::WAKE_DELAY_MS);
                }
            }
            Command::Pen {
                project,
                edit,
                reply,
            } => {
                let answer = self.write(&project, edit);
                let _ = reply.send(answer);
            }
            Command::RunEnded {
                project,
                end,
                edits,
            } => {
                if let Some(run) = self.run.take() {
                    let _ = run.join.join();
                }
                self.complete(&project, end, edits);
                if self.settle(&project) {
                    self.publish_followed(sink);
                }
                self.auto_apply(&project, sink);
                self.publish_if_opened(&project, sink);
            }
            Command::SessionSeen {
                project,
                session,
                checkout,
                at_unix_ms,
            } => {
                let Some(id) = self.resolve(&project, sink) else {
                    return;
                };
                if self.saw_session(&id, &session, &checkout, at_unix_ms) && self.settle(&id) {
                    self.publish_followed(sink);
                }
                self.publish_if_opened(&id, sink);
            }
            Command::Holding {
                project_id,
                line,
                checkouts,
            } => self.hold(&project_id, line, checkouts),
            Command::Attempted {
                project_id,
                line,
                started_at_unix_ms,
                change,
            } => {
                self.attempted(&project_id, line, started_at_unix_ms, change);
                self.auto_apply(&project_id, sink);
                self.publish_if_opened(&project_id, sink);
                self.publish_followed(sink);
            }
        }
    }

    /// Records the facts the Project's switches let in; answers whether the
    /// store took any.
    fn observe(&mut self, project: &str, facts: Vec<Observation>) -> bool {
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return false;
        };
        let state = match store.state(project) {
            Ok(state) => state,
            Err(code) => {
                log_store("observe", &code);
                return false;
            }
        };
        let kept = facts
            .into_iter()
            .filter(|fact| state.settings.records(fact.signal))
            .collect::<Vec<_>>();
        if kept.is_empty() {
            return false;
        }
        match store.record(project, kept, now) {
            Ok((written, pruned)) => {
                if !written.is_empty() || pruned.records > 0 {
                    crate::diagnostic!(json!({
                        "component": "suggestions", "kind": "records.kept",
                        "project_id": project, "written": written.len(),
                        "pruned": pruned.records, "references_pruned": pruned.references,
                    }));
                }
                !written.is_empty()
            }
            Err(code) => {
                log_store("observe", &code);
                false
            }
        }
    }

    /// Remembers the machine and folder `project` was named from, once per
    /// Project in a run of the worker (B13, B21).
    fn locate(&mut self, project: &str, at: &ProjectRef) {
        if self.located.contains(project) {
            return;
        }
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let location = ProjectLocation {
            node: at.node.as_str().to_owned(),
            path: at.path.clone(),
        };
        let mut state = match store.state(project) {
            Ok(state) => state,
            Err(code) => return log_store("locate", &code),
        };
        if state.location.as_ref() != Some(&location) {
            state.location = Some(location);
            if let Err(code) = store.save_state(project, &state) {
                return log_store("locate", &code);
            }
        }
        if self.located.len() >= policy::IDENTITY_CACHE {
            self.located.clear();
        }
        self.located.insert(project.to_owned());
    }

    /// Begins an attempt on a line and hands it to the runtime to start
    /// (B11-B14).
    fn apply_line(
        &mut self,
        project: &str,
        line_id: LineId,
        retry: bool,
        auto: bool,
        sink: &impl Sink,
    ) {
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let loaded = store.state(project).and_then(|state| {
            let (notebook, lives) = store.notebook(project)?;
            let index = store.index(project, &notebook)?;
            Ok((state, notebook, lives, index))
        });
        let (state, notebook, lives, index) = match loaded {
            Ok(loaded) => loaded,
            Err(code) => return log_store("apply", &code),
        };
        let refused = |reason: &str| {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "apply.refused",
                "project_id": project, "line": line_id.0, "reason": reason,
            }));
        };
        let Some(location) = state.location else {
            return refused("unlocated");
        };
        let Some(mut line) = notebook.line(line_id).cloned() else {
            return refused("unknown_line");
        };
        let mut life = lives.get(&line_id).cloned().unwrap_or_default();
        let Ok((outcome, branch, agent_name, brief)) =
            applying::begin(&mut line, &mut life, &index, retry, auto, now)
        else {
            return refused("not_ready");
        };
        let begun = life.attempt.as_ref().expect("begin wrote the attempt");
        let (issue_title, issue_since) = (begun.issue_title.clone(), begun.issue_since);
        if let Err(code) = store.save_line(project, &line, &life) {
            return log_store("apply", &code);
        }
        crate::diagnostic!(json!({
            "component": "suggestions", "kind": "apply.started",
            "project_id": project, "line": line_id.0, "auto": auto, "retry": retry,
            "outcome": match outcome { Outcome::Pr => "pr", Outcome::Issue => "issue" },
        }));
        sink.apply(ApplyOrder {
            project_id: project.to_owned(),
            location,
            line: line_id,
            started_at_unix_ms: now,
            auto,
            outcome,
            branch,
            agent_name,
            brief,
            issue_title,
            issue_since,
        });
    }

    /// Saves what the runtime saw happen to an attempt.
    fn attempted(&mut self, project: &str, line_id: LineId, started: u64, change: AttemptMove) {
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let (notebook, lives) = match store.notebook(project) {
            Ok(loaded) => loaded,
            Err(code) => return log_store("attempt", &code),
        };
        let Some(mut line) = notebook.line(line_id).cloned() else {
            return;
        };
        let mut life = lives.get(&line_id).cloned().unwrap_or_default();
        if !applying::moved(&mut line, &mut life, started, change, now) {
            return;
        }
        if let Err(code) = store.save_line(project, &line, &life) {
            return log_store("attempt", &code);
        }
        let phase = life.attempt.as_ref().map(|attempt| attempt.phase);
        crate::diagnostic!(json!({
            "component": "suggestions", "kind": "attempt.moved",
            "project_id": project, "line": line_id.0,
            "phase": phase.map(phase_code),
        }));
    }

    /// Applies the next new suggestion by itself in a Project whose mode
    /// is `자동으로 PR 열기` (B13).
    fn auto_apply(&mut self, project: &str, sink: &impl Sink) {
        let Ok(store) = self.store.as_ref() else {
            return;
        };
        let loaded = store
            .state(project)
            .and_then(|state| Ok((state, store.notebook(project)?)));
        let (state, (notebook, lives)) = match loaded {
            Ok(loaded) => loaded,
            Err(code) => return log_store("auto_apply", &code),
        };
        if !state.settings.active() || state.settings.apply_mode != ApplyMode::AutoPr {
            return;
        }
        let Some(location) = state.location.as_ref() else {
            return;
        };
        let Some(line) = applying::next_auto(&notebook, &lives) else {
            return;
        };
        let outcome = notebook
            .line(line)
            .and_then(|line| line.outcome)
            .unwrap_or(Outcome::Pr);
        if sink.can_apply(location, outcome) {
            self.apply_line(project, line, false, true, sink);
        }
    }

    /// Counts a session for each line of the Project being watched; answers
    /// whether any watch took it.
    fn saw_session(
        &mut self,
        project: &str,
        session: &SessionRef,
        checkout: &str,
        at_unix_ms: u64,
    ) -> bool {
        let Ok(store) = self.store.as_mut() else {
            return false;
        };
        let (notebook, lives) = match store.notebook(project) {
            Ok(loaded) => loaded,
            Err(code) => {
                log_store("watch", &code);
                return false;
            }
        };
        let mut took = false;
        for line in notebook
            .lines
            .iter()
            .filter(|line| line.stage == Stage::Watching)
        {
            let mut life = lives.get(&line.id).cloned().unwrap_or_default();
            if watching::saw_session(line, &mut life, session, checkout, at_unix_ms) {
                if let Err(code) = store.save_line(project, line, &life) {
                    log_store("watch", &code);
                    return took;
                }
                took = true;
            }
        }
        took
    }

    /// Keeps the checkouts the runtime judged to hold a line's merge.
    fn hold(&mut self, project: &str, line_id: LineId, checkouts: BTreeSet<String>) {
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let (notebook, lives) = match store.notebook(project) {
            Ok(loaded) => loaded,
            Err(code) => return log_store("watch", &code),
        };
        let Some(line) = notebook
            .line(line_id)
            .filter(|line| line.stage == Stage::Watching)
        else {
            return;
        };
        let mut life = lives.get(&line_id).cloned().unwrap_or_default();
        let Some(watch) = life
            .watch
            .as_mut()
            .filter(|watch| watch.holding != checkouts)
        else {
            return;
        };
        watch.holding = checkouts;
        if let Err(code) = store.save_line(project, line, &life) {
            log_store("watch", &code);
        }
    }

    /// Ends each watch of the Project that is over (B15); answers whether
    /// any ended.
    fn settle(&mut self, project: &str) -> bool {
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return false;
        };
        let (notebook, lives) = match store.notebook(project) {
            Ok(loaded) => loaded,
            Err(code) => {
                log_store("watch", &code);
                return false;
            }
        };
        let mut ended = false;
        for line in notebook
            .lines
            .iter()
            .filter(|line| line.stage == Stage::Watching)
        {
            let records = match store.records_of(project, &line.records) {
                Ok(records) => records,
                Err(code) => {
                    log_store("watch", &code);
                    continue;
                }
            };
            let mut line = line.clone();
            let mut life = lives.get(&line.id).cloned().unwrap_or_default();
            let Some(end) = watching::settle(&mut line, &mut life, &records, now) else {
                continue;
            };
            if let Err(code) = store.save_line(project, &line, &life) {
                log_store("watch", &code);
                continue;
            }
            ended = true;
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "watch.ended",
                "project_id": project, "line": line.id.0, "end": end.as_str(),
                "level": line.level,
            }));
        }
        ended
    }

    /// Looks at every Project's watches, for the ones whose time ran out.
    fn settle_all(&mut self, sink: &impl Sink) {
        let projects = match self.store.as_ref().map(SuggestionStore::projects) {
            Ok(Ok(projects)) => projects,
            Ok(Err(code)) => return log_store("watch", &code),
            Err(_) => return,
        };
        let mut ended = false;
        for project in &projects {
            if self.settle(project) {
                ended = true;
                self.publish_if_opened(project, sink);
            }
        }
        if ended {
            self.publish_followed(sink);
        }
    }

    /// Hands the runtime every attempt still to follow, in every Project.
    fn publish_followed(&self, sink: &impl Sink) {
        let Ok(store) = self.store.as_ref() else {
            return;
        };
        let projects = match store.projects() {
            Ok(projects) => projects,
            Err(code) => return log_store("followed", &code),
        };
        let mut followed = Vec::new();
        let mut watches = Vec::new();
        for project in projects {
            let loaded = store
                .state(&project)
                .and_then(|state| Ok((state, store.notebook(&project)?)));
            match loaded {
                Ok((state, (notebook, lives))) => {
                    if let Some(location) = &state.location {
                        followed.extend(applying::followed(&project, location, &notebook, &lives));
                        watches.extend(watching::watched(&project, location, &notebook, &lives));
                    }
                }
                Err(code) => log_store("followed", &code),
            }
        }
        sink.followed(followed, watches);
    }

    fn change_settings(&mut self, project: &str, change: SettingsChange, sink: &impl Sink) {
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let mut state = match store.state(project) {
            Ok(state) => state,
            Err(code) => return log_store("settings", &code),
        };
        let settings = &mut state.settings;
        let mut restore = None;
        match change {
            SettingsChange::HideAiOff { memory_on } => {
                if settings.hide_ai_enabled {
                    settings.hide_ai_enabled = false;
                    settings.memory_was_on = memory_on;
                }
            }
            SettingsChange::HideAiOn { workspace_id } => {
                if !settings.hide_ai_enabled {
                    settings.hide_ai_enabled = true;
                    if std::mem::take(&mut settings.memory_was_on) {
                        restore = Some(workspace_id);
                    }
                }
            }
            SettingsChange::Suggestions(on) => settings.suggestions_enabled = on,
            SettingsChange::Quantity(quantity) => settings.quantity = quantity,
            SettingsChange::ApplyMode(mode) => settings.apply_mode = mode,
            SettingsChange::Signal(signal, on) => {
                if on {
                    settings.signals_off.remove(&signal);
                } else {
                    settings.signals_off.insert(signal);
                }
            }
        }
        let active = state.settings.active();
        if let Err(code) = store.save_state(project, &state) {
            return log_store("settings", &code);
        }
        if let Some(workspace_id) = restore {
            sink.restore_memory(workspace_id);
        }
        // Turning a Project off ends its run; what it wrote stays.
        if !active && let Some(run) = self.run.as_ref().filter(|run| run.project == project) {
            run.stop.set();
        }
    }

    fn act(&mut self, project: &str, line_id: LineId, action: LineAction) {
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let (notebook, lives) = match store.notebook(project) {
            Ok(loaded) => loaded,
            Err(code) => return log_store("action", &code),
        };
        let Some(mut line) = notebook.line(line_id).cloned() else {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "action.unknown_line",
                "project_id": project, "line": line_id.0,
            }));
            return;
        };
        let life = lives.get(&line_id).cloned().unwrap_or_default();
        match action {
            LineAction::Dismiss => {
                // An attempt still at work keeps its line; a failed one, or
                // one whose checks failed, may be let go (B12).
                if line.stage == Stage::Applying
                    && life
                        .attempt
                        .as_ref()
                        .is_some_and(|a| a.phase.followed() && a.phase != AttemptPhase::CiFailed)
                {
                    crate::diagnostic!(json!({
                        "component": "suggestions", "kind": "action.line_busy",
                        "project_id": project, "line": line_id.0,
                    }));
                    return;
                }
                line.struck = Some(Strike {
                    by: StruckBy::Operator,
                    reason: None,
                    at_unix_ms: now,
                });
                line.stage = Stage::Candidate;
            }
            LineAction::Restore => {
                if !line
                    .struck
                    .as_ref()
                    .is_some_and(|strike| strike.by == StruckBy::Operator)
                {
                    return;
                }
                line.struck = None;
            }
            // Begun by `apply_line`, never here.
            LineAction::Apply | LineAction::Retry => return,
        }
        line.updated_at_unix_ms = now;
        if let Err(code) = store.save_line(project, &line, &life) {
            return log_store("action", &code);
        }
        crate::diagnostic!(json!({
            "component": "suggestions", "kind": "line.acted",
            "project_id": project, "line": line_id.0,
            "action": if action == LineAction::Dismiss { "dismiss" } else { "restore" },
        }));
    }

    /// Raises what the scores and caps let rise (D-12, D-42).
    fn raise(&mut self, project: &str) {
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let loaded = store.state(project).and_then(|state| {
            let (notebook, lives) = store.notebook(project)?;
            let index = store.index(project, &notebook)?;
            Ok((state, notebook, lives, index))
        });
        let (mut state, notebook, lives, index) = match loaded {
            Ok(loaded) => loaded,
            Err(code) => return log_store("raise", &code),
        };
        if !state.settings.active() {
            return;
        }
        let open = notebook
            .lines
            .iter()
            .filter(|line| line.stage == Stage::Suggested && line.struck.is_none())
            .count();
        let scored = notebook
            .lines
            .iter()
            .filter(|line| line.stage == Stage::Candidate && line.struck.is_none())
            .map(|line| (line, scoring::score(line, &index, now)))
            .collect::<Vec<_>>();
        state
            .raised_at
            .retain(|at| now.saturating_sub(*at) < policy::WEEK_MS);
        let first_look = state.quiet_since.is_none();
        let quiet_since = *state.quiet_since.get_or_insert(now);
        let chosen = scoring::choose(
            &scored,
            open,
            &state.raised_at,
            quiet_since,
            state.settings.quantity,
            now,
        );
        if chosen.is_empty() {
            if first_look && let Err(code) = store.save_state(project, &state) {
                log_store("raise", &code);
            }
            return;
        }
        for id in &chosen {
            let Some(mut line) = notebook.line(*id).cloned() else {
                continue;
            };
            let points = scored
                .iter()
                .find(|(candidate, _)| candidate.id == *id)
                .map_or(0.0, |(_, score)| score.points);
            line.stage = Stage::Suggested;
            line.updated_at_unix_ms = now;
            let mut life = lives.get(id).cloned().unwrap_or_default();
            life.raised_at_unix_ms = Some(now);
            if let Err(code) = store.save_line(project, &line, &life) {
                return log_store("raise", &code);
            }
            state.raised_at.push(now);
            state.quiet_since = Some(now);
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "line.raised",
                "project_id": project, "line": id.0, "points": (points * 100.0).round() / 100.0,
                "records": line.records.len(),
            }));
        }
        if let Err(code) = store.save_state(project, &state) {
            log_store("raise", &code);
        }
    }

    /// One edit from a run's pen: checked against the store's notebook and
    /// the run's index, saved, then answered.
    fn write(
        &mut self,
        project: &str,
        edit: NotebookEdit,
    ) -> Result<(Written, Notebook), PenRefused> {
        let now = self.now();
        let Some(run) = self.run.as_mut().filter(|run| run.project == project) else {
            return Err(PenRefused::Stopped);
        };
        if run.stop.is_set() {
            return Err(PenRefused::Stopped);
        }
        let store = self
            .store
            .as_mut()
            .map_err(|code| PenRefused::Store { code: code.clone() })?;
        let (mut notebook, _) = store
            .notebook(project)
            .map_err(|code| PenRefused::Store { code })?;
        let kind = edit.kind();
        let applied = notebook
            .apply(edit, &run.index, now)
            .map_err(PenRefused::Edit)?;
        store
            .save_edit(project, &notebook, &applied)
            .map_err(|code| PenRefused::Store { code })?;
        run.edits += 1;
        for evicted in &applied.evicted {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "line.evicted",
                "project_id": project, "line": evicted.0,
            }));
        }
        crate::diagnostic!(json!({
            "component": "suggestions", "kind": "notebook.edited",
            "project_id": project, "edit": kind,
            "line": applied.created.or_else(|| applied.touched.iter().next().copied()).map(|line| line.0),
            "records": applied.seen.len(),
        }));
        Ok((
            applied.created.map_or(Written::Done, Written::Line),
            notebook,
        ))
    }

    /// Starts the day's runs once they are due, one Project at a time.
    fn advance_runs(&mut self, sink: &impl Sink) {
        if self.run.is_some() || self.stop.is_set() {
            return;
        }
        if let Some(due) = self.due_at
            && due <= self.now()
        {
            self.due_at = None;
            self.round = match self.store.as_ref() {
                Ok(store) => store.projects().unwrap_or_else(|code| {
                    log_store("round", &code);
                    Vec::new()
                }),
                Err(_) => Vec::new(),
            };
        }
        while let Some(project) = self.round.pop() {
            if self.start_run(&project, sink) {
                return;
            }
        }
    }

    /// Starts `project`'s run if it is on, has not run today and has new
    /// records; a Project with none completes its day at once. Answers
    /// whether a run is now going.
    fn start_run(&mut self, project: &str, sink: &impl Sink) -> bool {
        let today = self.today();
        let Ok(store) = self.store.as_mut() else {
            return false;
        };
        let loaded = store.state(project).and_then(|state| {
            let (notebook, _) = store.notebook(project)?;
            let index = store.index(project, &notebook)?;
            Ok((state, index))
        });
        let (state, index) = match loaded {
            Ok(loaded) => loaded,
            Err(code) => {
                log_store("run", &code);
                return false;
            }
        };
        if !state.settings.active() || state.last_completed_day.as_deref() == Some(today.as_str()) {
            return false;
        }
        if index.fresh().next().is_none() && !state.uninvestigated {
            self.complete(project, AnalysisEnd::Done, 0);
            self.auto_apply(project, sink);
            self.publish_if_opened(project, sink);
            return false;
        }
        let gate = sink.gate();
        let stop = StopFlag::default();
        let analysis = (self.options.analysis)(&RunPlan {
            project_id: project,
            gate,
            stop: &stop,
        });
        if analysis.uses_provider() && (!gate.ai_ready || !gate.usage_ok) {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "run.deferred",
                "project_id": project,
                "reason": if gate.ai_ready { "usage_limit" } else { "ai_not_ready" },
            }));
            return false;
        }
        let input = AnalysisInput {
            project_id: project.to_owned(),
            records: index.clone(),
            language: gate.language,
            budget: AnalysisBudget {
                max_turns: policy::RUN_TURNS,
                deadline: Instant::now() + Duration::from_millis(policy::RUN_SPAN_MS),
                stop: stop.clone(),
            },
        };
        let notebook = match store.notebook(project) {
            Ok((notebook, _)) => notebook,
            Err(code) => {
                log_store("run", &code);
                return false;
            }
        };
        let pen = ChannelPen {
            project: project.to_owned(),
            notebook,
            client: self.own.clone(),
            edits: 0,
        };
        let name = analysis.name();
        let spawned = thread::Builder::new()
            .name("hide-suggestions-run".into())
            .spawn(move || {
                let mut analysis = analysis;
                let mut pen = pen;
                let end = analysis.run(&input, &mut pen);
                pen.client.deliver(Command::RunEnded {
                    project: input.project_id.clone(),
                    end,
                    edits: pen.edits,
                });
            });
        match spawned {
            Ok(join) => {
                crate::diagnostic!(json!({
                    "component": "suggestions", "kind": "run.started",
                    "project_id": project, "analysis": name,
                    "fresh": index.fresh().count(), "records": index.entries.len(),
                }));
                self.run = Some(RunHandle {
                    project: project.to_owned(),
                    index,
                    stop,
                    join,
                    edits: 0,
                });
                self.publish_if_opened(project, sink);
                true
            }
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": "suggestions", "kind": "run.not_started",
                    "project_id": project, "message": error.to_string(),
                }));
                false
            }
        }
    }

    /// Ends `project`'s day, which is kept as done so it never runs twice a
    /// day. A run that failed raises nothing from the scores alone (D-35):
    /// what it left unread is grouped by tag when it wrote nothing
    /// (`not_investigated`), the Project shows `조사 못 함`, and the next
    /// run investigates whether or not new records came; raised lines and
    /// watches stay as they are. Any other end raises what rose.
    fn complete(&mut self, project: &str, end: AnalysisEnd, edits: usize) {
        let today = self.today();
        let failed = end == AnalysisEnd::Partial(PartialReason::Failed);
        let mut ended = match end {
            AnalysisEnd::Done => "done",
            AnalysisEnd::Partial(reason) => reason.as_str(),
        };
        if failed && edits == 0 {
            self.group_by_tag(project);
            ended = "not_investigated";
        }
        crate::diagnostic!(json!({
            "component": "suggestions", "kind": "run.ended",
            "project_id": project, "end": ended, "edits": edits,
        }));
        if !failed {
            self.raise(project);
        }
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let mut state = match store.state(project) {
            Ok(state) => state,
            Err(code) => return log_store("complete", &code),
        };
        state.last_completed_day = Some(today);
        state.last_run_at_unix_ms = Some(now);
        state.last_run_end = Some(ended.to_owned());
        state.uninvestigated = failed;
        if let Err(code) = store.save_state(project, &state) {
            log_store("complete", &code);
        }
    }

    /// The deterministic grouping over the store directly, for a run that
    /// could not write.
    fn group_by_tag(&mut self, project: &str) {
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let loaded = store.notebook(project).and_then(|(notebook, _)| {
            let index = store.index(project, &notebook)?;
            Ok((notebook, index))
        });
        let (notebook, index) = match loaded {
            Ok(loaded) => loaded,
            Err(code) => return log_store("fallback", &code),
        };
        let input = AnalysisInput {
            project_id: project.to_owned(),
            records: index,
            language: hide_ai::OutputLanguage::English,
            budget: AnalysisBudget {
                max_turns: 0,
                deadline: Instant::now(),
                stop: StopFlag::default(),
            },
        };
        let mut pen = StorePen {
            project,
            store,
            notebook,
            index: &input.records,
            now,
        };
        TagGrouping.run(&input, &mut pen);
    }

    fn publish_if_opened(&mut self, project: &str, sink: &impl Sink) {
        if self
            .opened
            .as_ref()
            .is_some_and(|(_, id)| id.as_deref() == Some(project))
        {
            self.publish(sink);
        }
    }

    fn publish(&mut self, sink: &impl Sink) {
        let Some((workspace_id, project)) = self.opened.clone() else {
            return;
        };
        let today = self.today();
        let investigating = self
            .run
            .as_ref()
            .is_some_and(|run| Some(&run.project) == project.as_ref());
        let read = match (&project, self.store.as_ref()) {
            (Some(project), Ok(store)) => (|| {
                let state = store.state(project)?;
                let (notebook, lives) = store.notebook(project)?;
                let index = store.index(project, &notebook)?;
                let intro_seen = store.meta(INTRO_SEEN)?.is_some();
                Ok::<_, String>(read::project(&ReadInput {
                    workspace_id: &workspace_id,
                    state: &state,
                    notebook: &notebook,
                    lives: &lives,
                    index: &index,
                    investigating,
                    intro_seen,
                    today: &today,
                }))
            })(),
            (_, Err(code)) => Err(code.clone()),
            (None, _) => Err("project_unresolved".to_owned()),
        };
        let read = read.unwrap_or_else(|code| {
            log_store("read", &code);
            ProjectSuggestionsSnapshot {
                workspace_id: workspace_id.clone(),
                unavailable: Some(crate::model::SuggestionsUnavailable::Unreadable),
                ..ProjectSuggestionsSnapshot::default()
            }
        });
        sink.read(workspace_id, read);
    }
}

/// The pen a run on its own thread writes with: each edit goes to the
/// worker, which saves it, and the answer carries the notebook as saved.
struct ChannelPen {
    project: String,
    notebook: Notebook,
    client: SuggestionClient,
    edits: usize,
}

impl NotebookPen for ChannelPen {
    fn notebook(&self) -> &Notebook {
        &self.notebook
    }

    fn write(&mut self, edit: NotebookEdit) -> Result<Written, PenRefused> {
        let (reply, answer) = mpsc::channel();
        if !self.client.deliver(Command::Pen {
            project: self.project.clone(),
            edit,
            reply,
        }) {
            return Err(PenRefused::Stopped);
        }
        let (written, notebook) = answer.recv().map_err(|_| PenRefused::Stopped)??;
        self.notebook = notebook;
        self.edits += 1;
        Ok(written)
    }
}

/// The pen the worker itself writes with, straight to the store.
struct StorePen<'a> {
    project: &'a str,
    store: &'a mut SuggestionStore,
    notebook: Notebook,
    index: &'a RecordIndex,
    now: u64,
}

impl NotebookPen for StorePen<'_> {
    fn notebook(&self) -> &Notebook {
        &self.notebook
    }

    fn write(&mut self, edit: NotebookEdit) -> Result<Written, PenRefused> {
        let mut next = self.notebook.clone();
        let applied = next
            .apply(edit, self.index, self.now)
            .map_err(PenRefused::Edit)?;
        self.store
            .save_edit(self.project, &next, &applied)
            .map_err(|code| PenRefused::Store { code })?;
        self.notebook = next;
        Ok(applied.created.map_or(Written::Done, Written::Line))
    }
}

fn phase_code(phase: AttemptPhase) -> &'static str {
    match phase {
        AttemptPhase::Starting => "starting",
        AttemptPhase::Working => "working",
        AttemptPhase::PrOpen => "pr_open",
        AttemptPhase::CiFailed => "ci_failed",
        AttemptPhase::Failed => "failed",
        AttemptPhase::IssueOpen => "issue_open",
        AttemptPhase::Merged => "merged",
        AttemptPhase::Closed => "closed",
    }
}

fn log_store(during: &str, code: &str) {
    crate::diagnostic!(json!({
        "component": "suggestions", "kind": "store.failed", "during": during, "code": code,
    }));
}

/// The local calendar day of `unix_ms`, `YYYY-MM-DD`.
pub(crate) fn local_day(unix_ms: u64, zone: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_millisecond(unix_ms as i64)
        .map(|at| at.to_zoned(zone.clone()).date().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{SuggestionGroup, SuggestionState, SuggestionsStatus};
    use crate::suggestions::ledger::SessionRef;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    const START: u64 = 400 * policy::DAY_MS + 9 * 3_600_000;
    const WAIT: Duration = Duration::from_secs(10);

    struct TestSink {
        reads: mpsc::Sender<ProjectSuggestionsSnapshot>,
        restored: mpsc::Sender<String>,
        orders: mpsc::Sender<ApplyOrder>,
        followed: mpsc::Sender<Vec<FollowedAttempt>>,
        gate: RunGate,
    }

    impl Sink for TestSink {
        fn read(&self, _workspace_id: String, read: ProjectSuggestionsSnapshot) {
            let _ = self.reads.send(read);
        }
        fn gate(&self) -> RunGate {
            self.gate
        }
        fn restore_memory(&self, workspace_id: String) {
            let _ = self.restored.send(workspace_id);
        }
        fn link(&self, _node: &NodeId) -> Option<Arc<dyn NodeLink>> {
            None
        }
        fn apply(&self, order: ApplyOrder) {
            let _ = self.orders.send(order);
        }
        fn followed(&self, attempts: Vec<FollowedAttempt>, _watches: Vec<WatchedLine>) {
            let _ = self.followed.send(attempts);
        }
        fn can_apply(&self, _location: &ProjectLocation, _outcome: Outcome) -> bool {
            true
        }
    }

    /// What the worker handed the runtime to start and follow.
    struct Handed {
        orders: Receiver<ApplyOrder>,
        followed: Receiver<Vec<FollowedAttempt>>,
    }

    struct Fixture {
        worker: Option<SuggestionWorker>,
        analysis: AnalysisFactory,
        clock: Arc<AtomicU64>,
        reads: Receiver<ProjectSuggestionsSnapshot>,
        restored: Receiver<String>,
        handed: Handed,
        _dir: tempfile::TempDir,
        store: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_analysis(Arc::new(|_: &RunPlan<'_>| {
                Box::new(TagGrouping) as Box<dyn Analysis>
            }))
        }

        fn with_analysis(analysis: AnalysisFactory) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = dir.path().join("suggestions.sqlite3");
            let clock = Arc::new(AtomicU64::new(START));
            let (worker, reads, restored, handed) = Self::spawn(&store, &clock, &analysis);
            Self {
                worker: Some(worker),
                analysis,
                clock,
                reads,
                restored,
                handed,
                _dir: dir,
                store,
            }
        }

        fn spawn(
            store: &std::path::Path,
            clock: &Arc<AtomicU64>,
            analysis: &AnalysisFactory,
        ) -> (
            SuggestionWorker,
            Receiver<ProjectSuggestionsSnapshot>,
            Receiver<String>,
            Handed,
        ) {
            let (reads_tx, reads) = mpsc::channel();
            let (restored_tx, restored) = mpsc::channel();
            let (orders_tx, orders) = mpsc::channel();
            let (followed_tx, followed) = mpsc::channel();
            let time = Arc::clone(clock);
            let options = Options {
                store: store.to_owned(),
                clock: Arc::new(move || time.load(Ordering::SeqCst)),
                zone: jiff::tz::TimeZone::UTC,
                analysis: Arc::clone(analysis),
                resolver: Box::new(|project: &ProjectRef, _| {
                    Ok(format!("project{}", project.path))
                }),
            };
            let sink = TestSink {
                reads: reads_tx,
                restored: restored_tx,
                orders: orders_tx,
                followed: followed_tx,
                gate: RunGate {
                    ai_ready: false,
                    usage_ok: true,
                    language: hide_ai::OutputLanguage::English,
                },
            };
            (
                SuggestionWorker::spawn(options, sink).unwrap(),
                reads,
                restored,
                Handed { orders, followed },
            )
        }

        /// Stops the worker and starts another on the same store, as a
        /// hided restart does.
        fn restart(&mut self) {
            self.worker.take();
            let (worker, reads, restored, handed) =
                Self::spawn(&self.store, &self.clock, &self.analysis);
            self.worker = Some(worker);
            self.reads = reads;
            self.restored = restored;
            self.handed = handed;
        }

        fn send(&self, command: Command) {
            assert!(self.worker.as_ref().unwrap().client().send(command));
        }

        fn open(&self) -> ProjectSuggestionsSnapshot {
            self.send(Command::Open {
                workspace_id: "w1".into(),
                project: project(),
            });
            self.next_read()
        }

        fn next_read(&self) -> ProjectSuggestionsSnapshot {
            self.reads.recv_timeout(WAIT).expect("a read")
        }

        /// Reads until one satisfies `done`.
        fn read_until(
            &self,
            done: impl Fn(&ProjectSuggestionsSnapshot) -> bool,
        ) -> ProjectSuggestionsSnapshot {
            loop {
                let read = self.next_read();
                if done(&read) {
                    return read;
                }
            }
        }

        fn settings(&self, change: SettingsChange) -> ProjectSuggestionsSnapshot {
            self.send(Command::Settings {
                project: project(),
                change,
            });
            self.next_read()
        }

        fn fail_in_sessions(&self, tag: &str, sessions: &[&str]) {
            let at = self.clock.load(Ordering::SeqCst);
            self.send(Command::Observe {
                project: project(),
                facts: sessions
                    .iter()
                    .map(|session| Observation {
                        signal: Signal::FailedTool,
                        tag: tag.into(),
                        session: Some(SessionRef {
                            agent: "claude".into(),
                            id: (*session).into(),
                        }),
                        pr: None,
                        checkout: Some("/repo".into()),
                        at_unix_ms: at,
                        always: false,
                    })
                    .collect(),
            });
        }

        /// Wakes the worker and moves the clock past the wake's delay. The
        /// intro command answers with a read, so the first one proves the
        /// wake was taken before the clock moved, and the second only makes
        /// the worker look at the clock.
        fn wake_and_wait_out_the_delay(&self) {
            self.send(Command::Wake);
            self.send(Command::IntroSeen);
            self.next_read();
            self.clock
                .fetch_add(policy::WAKE_DELAY_MS + 1, Ordering::SeqCst);
            self.send(Command::IntroSeen);
        }
    }

    fn project() -> ProjectRef {
        ProjectRef {
            node: NodeId::parse("test-node").unwrap(),
            path: "/repo".into(),
        }
    }

    fn shown(read: &ProjectSuggestionsSnapshot) -> Vec<&str> {
        read.rows.iter().map(|row| row.id.as_str()).collect()
    }

    fn chosen(read: &ProjectSuggestionsSnapshot) -> Vec<&str> {
        read.rows
            .iter()
            .filter(|row| row.group == SuggestionGroup::Choose && row.state == SuggestionState::New)
            .map(|row| row.id.as_str())
            .collect()
    }

    #[test]
    fn one_failure_in_three_sessions_is_suggested_by_the_day_after_a_wake() {
        let fixture = Fixture::new();
        let first = fixture.open();
        assert_eq!(first.status, SuggestionsStatus::FirstRunPending);
        assert!(first.rows.is_empty());

        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3"]);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| !chosen(read).is_empty());
        assert_eq!(chosen(&read), ["s1"]);
        let row = &read.rows[0];
        assert_eq!(row.evidence.len(), 3, "one evidence row per session");
        assert_eq!(
            row.basis.as_ref().map(|basis| basis.tag.as_str()),
            Some("cargo test")
        );
        assert_eq!(
            read.status,
            SuggestionsStatus::Idle,
            "a row waits to be chosen"
        );
    }

    /// D-10: a new Project's first look raises nothing under Normal's
    /// threshold; only fourteen quiet days from that look raise its
    /// strongest line: not on day 13, on day 14.
    #[test]
    fn a_weak_line_waits_fourteen_quiet_days_from_the_first_look() {
        let fixture = Fixture::new();
        fixture.open();
        fixture.fail_in_sessions("cargo test", &["s1"]);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| read.status == SuggestionsStatus::NoneToday);
        assert!(chosen(&read).is_empty(), "day 0");

        let looked = fixture.clock.load(Ordering::SeqCst);
        fixture
            .clock
            .store(looked + 13 * policy::DAY_MS, Ordering::SeqCst);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| read.status == SuggestionsStatus::NoneToday);
        assert!(chosen(&read).is_empty(), "day 13");

        fixture
            .clock
            .store(looked + 14 * policy::DAY_MS, Ordering::SeqCst);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| !chosen(read).is_empty());
        assert_eq!(chosen(&read), ["s1"], "day 14");
    }

    /// An investigation that fails on the days `fails` says, and otherwise
    /// groups as the shipped analysis does; it counts its runs.
    struct Flaky {
        fails: Arc<AtomicBool>,
        runs: Arc<AtomicUsize>,
    }

    impl Analysis for Flaky {
        fn name(&self) -> &'static str {
            "flaky"
        }
        fn uses_provider(&self) -> bool {
            false
        }
        fn run(&mut self, input: &AnalysisInput, pen: &mut dyn NotebookPen) -> AnalysisEnd {
            self.runs.fetch_add(1, Ordering::SeqCst);
            if self.fails.load(Ordering::SeqCst) {
                return AnalysisEnd::Partial(PartialReason::Failed);
            }
            TagGrouping.run(input, pen)
        }
    }

    /// D-35 as the mission decided it: a run that fails raises nothing from
    /// the scores alone and leaves a raised line as it was; the Project
    /// shows `조사 못 함`, and the next day's run investigates though no
    /// record came since, and only then does the waiting line rise.
    #[test]
    fn a_failed_investigation_raises_nothing_until_a_run_investigates() {
        let fails = Arc::new(AtomicBool::new(false));
        let runs = Arc::new(AtomicUsize::new(0));
        let (flaky, counted) = (Arc::clone(&fails), Arc::clone(&runs));
        let fixture = Fixture::with_analysis(Arc::new(move |_: &RunPlan<'_>| {
            Box::new(Flaky {
                fails: Arc::clone(&flaky),
                runs: Arc::clone(&counted),
            }) as Box<dyn Analysis>
        }));
        fixture.open();
        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3"]);
        fixture.wake_and_wait_out_the_delay();
        fixture.read_until(|read| chosen(read) == ["s1"]);

        fails.store(true, Ordering::SeqCst);
        fixture.clock.fetch_add(policy::DAY_MS, Ordering::SeqCst);
        fixture.fail_in_sessions("pnpm lint", &["s4", "s5", "s6"]);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| read.status == SuggestionsStatus::NotInvestigated);
        assert_eq!(
            chosen(&read),
            ["s1"],
            "nothing new rose; the raised line stays"
        );

        fails.store(false, Ordering::SeqCst);
        fixture.clock.fetch_add(policy::DAY_MS, Ordering::SeqCst);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| chosen(read).len() == 2);
        assert_eq!(chosen(&read), ["s2", "s1"]);
        assert_eq!(read.status, SuggestionsStatus::Idle);
        assert_eq!(
            runs.load(Ordering::SeqCst),
            3,
            "the third day ran with no new record"
        );
    }

    /// B11-B13, B20: a press hands the runtime one attempt to start and to
    /// follow; what the runtime saw is saved and survives a restart; in the
    /// automatic mode a raised line is applied without a press, marked
    /// automatic.
    #[test]
    fn a_pressed_line_is_handed_on_and_its_moves_are_kept() {
        let mut fixture = Fixture::new();
        fixture.open();
        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3"]);
        fixture.wake_and_wait_out_the_delay();
        fixture.read_until(|read| !chosen(read).is_empty());

        fixture.send(Command::Action {
            project: project(),
            line: LineId(1),
            action: LineAction::Apply,
        });
        let order = fixture.handed.orders.recv_timeout(WAIT).expect("an order");
        assert_eq!(order.line, LineId(1));
        assert_eq!(order.outcome, Outcome::Pr);
        assert_eq!(order.location.path, "/repo");
        assert!(!order.auto);
        assert_eq!(
            order.brief.basis,
            Some((Signal::FailedTool, "cargo test".to_owned()))
        );
        let followed = loop {
            let list = fixture.handed.followed.recv_timeout(WAIT).expect("a list");
            if !list.is_empty() {
                break list;
            }
        };
        assert_eq!(followed[0].phase, AttemptPhase::Starting);
        let read = fixture.read_until(|read| read.rows.iter().any(|row| row.attempt.is_some()));
        assert_eq!(read.rows[0].group, SuggestionGroup::Applying);
        assert_eq!(read.rows[0].state, SuggestionState::HideAiWorking);

        fixture.send(Command::Attempted {
            project_id: "project/repo".into(),
            line: LineId(1),
            started_at_unix_ms: order.started_at_unix_ms,
            change: AttemptMove::PrOpen(crate::suggestions::ledger::PrRef {
                number: 12,
                url: "https://github.com/acme/app/pull/12".into(),
            }),
        });
        fixture.read_until(|read| {
            read.rows[0]
                .attempt
                .as_ref()
                .and_then(|attempt| attempt.pr_number)
                == Some(12)
        });
        fixture.restart();
        let read = fixture.open();
        let attempt = read.rows[0].attempt.clone().expect("kept across a restart");
        assert_eq!(attempt.pr_number, Some(12));
        assert_eq!(read.rows[0].state, SuggestionState::MyTurn);

        fixture.settings(SettingsChange::ApplyMode(ApplyMode::AutoPr));
        fixture.fail_in_sessions("npm run build", &["s4", "s5", "s6"]);
        fixture.clock.fetch_add(policy::DAY_MS, Ordering::SeqCst);
        fixture.wake_and_wait_out_the_delay();
        let order = fixture
            .handed
            .orders
            .recv_timeout(WAIT)
            .expect("an automatic order");
        assert_eq!(order.line, LineId(2));
        assert!(order.auto);
    }

    /// B15: a merge starts the watch; sessions in checkouts the runtime
    /// said hold the merge are counted, and the fifth ends it effective.
    #[test]
    fn a_merged_line_is_watched_until_enough_sessions_pass() {
        let fixture = Fixture::new();
        fixture.open();
        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3"]);
        fixture.wake_and_wait_out_the_delay();
        fixture.read_until(|read| !chosen(read).is_empty());
        fixture.send(Command::Action {
            project: project(),
            line: LineId(1),
            action: LineAction::Apply,
        });
        let order = fixture.handed.orders.recv_timeout(WAIT).expect("an order");
        let merged_at = fixture.clock.load(Ordering::SeqCst);
        fixture.send(Command::Attempted {
            project_id: "project/repo".into(),
            line: LineId(1),
            started_at_unix_ms: order.started_at_unix_ms,
            change: AttemptMove::Merged {
                pr: crate::suggestions::ledger::PrRef {
                    number: 12,
                    url: "https://github.com/acme/app/pull/12".into(),
                },
                at_unix_ms: merged_at,
                base: Some("main".into()),
            },
        });
        fixture.read_until(|read| read.rows[0].state == SuggestionState::Watching);
        fixture.send(Command::Holding {
            project_id: "project/repo".into(),
            line: LineId(1),
            checkouts: BTreeSet::from(["/repo".to_owned()]),
        });
        let seen = |id: &str, checkout: &str| {
            fixture.send(Command::SessionSeen {
                project: project(),
                session: SessionRef {
                    agent: "claude".into(),
                    id: id.into(),
                },
                checkout: checkout.into(),
                at_unix_ms: merged_at + 1,
            });
        };
        seen("elsewhere", "/repo-before-merge");
        for id in ["a", "b", "c", "d"] {
            seen(id, "/repo/web");
        }
        let read = fixture.read_until(|read| {
            read.rows[0]
                .watch
                .is_some_and(|watch| watch.sessions_seen == 4)
        });
        assert_eq!(read.rows[0].group, SuggestionGroup::Applied);
        seen("e", "/repo");
        let read = fixture.read_until(|read| read.rows[0].state == SuggestionState::Effective);
        assert!(read.rows[0].watch.is_none());
    }

    #[test]
    fn a_dismissed_line_leaves_the_tab_and_comes_back_when_restored() {
        let fixture = Fixture::new();
        fixture.open();
        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3"]);
        fixture.wake_and_wait_out_the_delay();
        fixture.read_until(|read| !chosen(read).is_empty());

        fixture.send(Command::Action {
            project: project(),
            line: LineId(1),
            action: LineAction::Dismiss,
        });
        let read = fixture.read_until(|read| chosen(read).is_empty());
        assert_eq!(read.settings.dismissed.len(), 1);
        assert_eq!(read.settings.dismissed[0].id, "s1");

        // Its records keep coming and never raise it while it is struck.
        fixture.fail_in_sessions("cargo test", &["s4", "s5", "s6"]);
        fixture.send(Command::Action {
            project: project(),
            line: LineId(1),
            action: LineAction::Restore,
        });
        let read = fixture.read_until(|read| read.settings.dismissed.is_empty());
        assert_eq!(chosen(&read), ["s1"], "restored and judged again");
    }

    #[test]
    fn the_head_switch_off_keeps_no_record_and_on_gives_memory_back() {
        let fixture = Fixture::new();
        fixture.open();
        let off = fixture.settings(SettingsChange::HideAiOff { memory_on: true });
        assert!(!off.enabled);
        assert_eq!(off.status, SuggestionsStatus::Off);

        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3"]);
        fixture.wake_and_wait_out_the_delay();
        fixture.next_read();
        assert!(fixture.restored.try_recv().is_err());

        let on = fixture.settings(SettingsChange::HideAiOn {
            workspace_id: "w1".into(),
        });
        assert!(on.enabled);
        assert_eq!(fixture.restored.recv_timeout(WAIT).unwrap(), "w1");
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| read.status != SuggestionsStatus::FirstRunPending);
        assert!(
            read.rows.is_empty(),
            "nothing recorded while off, so the day's run has nothing to raise"
        );

        // On again while on gives nothing back a second time.
        fixture.settings(SettingsChange::HideAiOn {
            workspace_id: "w1".into(),
        });
        assert!(fixture.restored.try_recv().is_err());
    }

    #[test]
    fn settings_and_the_day_survive_a_restart() {
        let mut fixture = Fixture::new();
        fixture.open();
        fixture.settings(SettingsChange::Quantity(Quantity::Many));
        fixture.settings(SettingsChange::ApplyMode(ApplyMode::AutoPr));
        fixture.settings(SettingsChange::Signal(Signal::AgentWait, false));
        fixture.fail_in_sessions("cargo test", &["s1", "s2"]);
        fixture.wake_and_wait_out_the_delay();
        // The automatic mode applies the line it raised at once.
        let before = fixture.read_until(|read| !shown(read).is_empty());

        fixture.restart();
        let after = fixture.open();
        assert_eq!(after.settings.quantity, Quantity::Many);
        assert_eq!(after.settings.apply_mode, ApplyMode::AutoPr);
        assert!(
            after
                .settings
                .signals
                .iter()
                .any(|signal| signal.signal == Signal::AgentWait && !signal.enabled)
        );
        assert_eq!(shown(&after), shown(&before));
        assert_eq!(after.intro_seen, before.intro_seen);

        // The day already ran: a second wake the same day starts nothing.
        fixture.fail_in_sessions("npm test", &["s7", "s8", "s9"]);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.next_read();
        assert_ne!(read.status, SuggestionsStatus::Investigating);
        assert_eq!(shown(&read), shown(&before));
    }

    /// A run that ends while the worker's queue is full still ends: its end
    /// waits for room instead of being dropped, so the round goes on to the
    /// next Project, whose line is then raised.
    #[test]
    fn a_run_that_ends_on_a_full_queue_still_lets_the_next_project_run() {
        /// Says the run started, then waits to be let go.
        type Gate = (mpsc::Sender<()>, Receiver<()>);
        /// The first run says it started, waits to be let go and writes
        /// nothing; every later run groups by tag.
        struct Held {
            first: Arc<std::sync::Mutex<Option<Gate>>>,
        }
        impl Analysis for Held {
            fn name(&self) -> &'static str {
                "held"
            }
            fn uses_provider(&self) -> bool {
                false
            }
            fn run(&mut self, input: &AnalysisInput, pen: &mut dyn NotebookPen) -> AnalysisEnd {
                if let Some((started, release)) = self.first.lock().unwrap().take() {
                    started.send(()).unwrap();
                    release.recv().unwrap();
                    return AnalysisEnd::Done;
                }
                TagGrouping.run(input, pen)
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let (started_tx, started) = mpsc::channel();
        let (release_run, held) = mpsc::channel();
        let first = Arc::new(std::sync::Mutex::new(Some((started_tx, held))));
        let (entered_tx, entered) = mpsc::channel();
        let (release_resolve, resolving) = mpsc::channel::<()>();
        let (reads_tx, reads) = mpsc::channel();
        let clock = Arc::new(AtomicU64::new(START));
        let time = Arc::clone(&clock);
        let worker = SuggestionWorker::spawn(
            Options {
                store: dir.path().join("suggestions.sqlite3"),
                clock: Arc::new(move || time.load(Ordering::SeqCst)),
                zone: jiff::tz::TimeZone::UTC,
                analysis: Arc::new(move |_: &RunPlan<'_>| {
                    Box::new(Held {
                        first: Arc::clone(&first),
                    }) as Box<dyn Analysis>
                }),
                // The worker answers nothing while it names this folder.
                resolver: Box::new(move |project: &ProjectRef, _| {
                    if project.path == "/slow" {
                        entered_tx.send(()).unwrap();
                        resolving.recv().unwrap();
                    }
                    Ok(format!("project{}", project.path))
                }),
            },
            TestSink {
                reads: reads_tx,
                restored: mpsc::channel().0,
                orders: mpsc::channel().0,
                followed: mpsc::channel().0,
                gate: RunGate {
                    ai_ready: false,
                    usage_ok: true,
                    language: hide_ai::OutputLanguage::English,
                },
            },
        )
        .unwrap();
        let client = worker.client();
        let at = |path: &str| ProjectRef {
            node: NodeId::parse("test-node").unwrap(),
            path: path.into(),
        };
        let failed = |path: &str| Command::Observe {
            project: at(path),
            facts: ["s1", "s2", "s3"]
                .iter()
                .map(|session| Observation {
                    signal: Signal::FailedTool,
                    tag: "cargo test".into(),
                    session: Some(SessionRef {
                        agent: "claude".into(),
                        id: (*session).into(),
                    }),
                    pr: None,
                    checkout: Some(path.into()),
                    at_unix_ms: START,
                    always: false,
                })
                .collect(),
        };
        let read_until = |done: &dyn Fn(&ProjectSuggestionsSnapshot) -> bool| loop {
            let read = reads.recv_timeout(WAIT).expect("a read");
            if done(&read) {
                return read;
            }
        };
        client.send(Command::Open {
            workspace_id: "w1".into(),
            project: at("/a"),
        });
        client.send(failed("/a"));
        client.send(failed("/b"));
        client.send(Command::Wake);
        client.send(Command::IntroSeen);
        read_until(&|_| true);
        read_until(&|_| true);
        clock.fetch_add(policy::WAKE_DELAY_MS + 1, Ordering::SeqCst);
        client.send(Command::IntroSeen);
        started
            .recv_timeout(WAIT)
            .expect("the round's first run started");

        client.send(failed("/slow"));
        entered
            .recv_timeout(WAIT)
            .expect("the worker is naming a folder");
        let mut queued = 0;
        while client.send(Command::IntroSeen) {
            queued += 1;
        }
        assert!(queued <= policy::QUEUE, "the queue filled");
        release_run.send(()).unwrap();
        release_resolve.send(()).unwrap();

        let read = read_until(&|read| !chosen(read).is_empty());
        assert_eq!(
            chosen(&read),
            ["s1"],
            "the next Project ran after the full queue"
        );
        client.send(Command::Open {
            workspace_id: "w2".into(),
            project: at("/b"),
        });
        let read = read_until(&|read| read.workspace_id == "w2");
        assert_eq!(
            read.status,
            SuggestionsStatus::NoneToday,
            "the held run ended"
        );
        drop(worker);
    }

    #[test]
    fn a_run_that_needs_the_provider_waits_while_hide_ai_is_not_ready() {
        let dir = tempfile::tempdir().unwrap();
        let (reads_tx, reads) = mpsc::channel();
        let (restored, _) = mpsc::channel();
        struct Asks;
        impl Analysis for Asks {
            fn name(&self) -> &'static str {
                "asks"
            }
            fn run(&mut self, _: &AnalysisInput, _: &mut dyn NotebookPen) -> AnalysisEnd {
                panic!("a run that needs the provider started while it was not ready")
            }
        }
        let clock = Arc::new(AtomicU64::new(START));
        let time = Arc::clone(&clock);
        let worker = SuggestionWorker::spawn(
            Options {
                store: dir.path().join("suggestions.sqlite3"),
                clock: Arc::new(move || time.load(Ordering::SeqCst)),
                zone: jiff::tz::TimeZone::UTC,
                analysis: Arc::new(|_: &RunPlan<'_>| Box::new(Asks) as Box<dyn Analysis>),
                resolver: Box::new(|_: &ProjectRef, _| Ok("project".into())),
            },
            TestSink {
                reads: reads_tx,
                restored,
                orders: mpsc::channel().0,
                followed: mpsc::channel().0,
                gate: RunGate {
                    ai_ready: false,
                    usage_ok: true,
                    language: hide_ai::OutputLanguage::English,
                },
            },
        )
        .unwrap();
        let client = worker.client();
        client.send(Command::Open {
            workspace_id: "w1".into(),
            project: project(),
        });
        client.send(Command::Observe {
            project: project(),
            facts: vec![Observation {
                signal: Signal::AgentWait,
                tag: "question".into(),
                session: Some(SessionRef {
                    agent: "codex".into(),
                    id: "s1".into(),
                }),
                pr: None,
                checkout: None,
                at_unix_ms: START,
                always: false,
            }],
        });
        client.send(Command::Wake);
        client.send(Command::IntroSeen);
        reads.recv_timeout(WAIT).unwrap();
        reads.recv_timeout(WAIT).unwrap();
        clock.fetch_add(policy::WAKE_DELAY_MS + 1, Ordering::SeqCst);
        client.send(Command::IntroSeen);
        let read = reads.recv_timeout(WAIT).unwrap();
        assert_eq!(
            read.status,
            SuggestionsStatus::FirstRunPending,
            "the day stays to do"
        );
        drop(worker);
    }
}
