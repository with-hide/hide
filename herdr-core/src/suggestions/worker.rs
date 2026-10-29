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
use super::apply::{self as applying, ApplyOrder, Begin, FollowedAttempt};
use super::ledger::{Observation, RecordIndex, SessionRef, SignalGroup};
use super::lifecycle::{AttemptMove, AttemptPhase, LocalFailure, LocalWrite};
use super::notebook::{Line, LineId, Notebook, NotebookEdit, Outcome, Stage, Strike, StruckBy};
use super::policy::{self, Quantity};
use super::read::{self, ReadInput};
use super::scoring;
use super::settings::ApplyMode;
use super::store::{ProjectLocation, SuggestionStore};
use super::watch::{self as watching, WatchedLine};
use crate::global_instruction;
use crate::model::ProjectSuggestionsSnapshot;
use crate::node::NodeId;
use crate::node_access::NodeLink;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
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
    /// One place ⚙ reads from, every signal of it at once (B5, D-52).
    SignalGroup(SignalGroup, bool),
}

/// An operator's action on a line (B16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineAction {
    /// `넘어가기`: the line is struck, and its new records raise it once
    /// more only when they double its evidence; a second skip is for good
    /// (D-43).
    Dismiss,
    /// ⚙'s `되살리기`: the strike goes and the line is judged again.
    Restore,
    /// `PR 만들기` or `이슈 만들기` on a new suggestion (B11, B14).
    Apply,
    /// `다시 시도` on an attempt that failed (B12).
    Retry,
    /// `PR 올리기` on a ready branch (PRD hide-ai-branch-first-apply D-06).
    OpenPr,
    /// `다시 맞추기` on a ready branch the base moved under (B8, B12).
    Rebase,
    /// `되돌리기` on a line Hide wrote into its global instruction: only that
    /// line comes out (PRD memory-self-improve D-30, B9).
    Revert,
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
        /// The global instruction preview the operator pressed on, which the
        /// file must still match (B8).
        seen: Option<String>,
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
    /// What GitHub said of a watched line's merge: its merge commit, and
    /// whether each pull request head asked about has it.
    HeadsJudged {
        project_id: String,
        line: LineId,
        merge_commit: String,
        heads: BTreeMap<String, bool>,
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
    /// The sessions of `sessions` on `node` whose record a press can still
    /// open, so the rest are drawn gone (B10); `None` when that cannot be
    /// read.
    fn kept_sessions(
        &self,
        node: &str,
        sessions: &BTreeSet<SessionRef>,
    ) -> Option<BTreeSet<SessionRef>>;
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
            Command::HeadsJudged { .. } => "heads_judged",
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
                if self.observe(&id, facts, sink) {
                    self.locate(&id, &project);
                }
            }
            Command::ObserveProject { project_id, facts } => {
                let memory = facts.iter().any(|fact| fact.signal.tag_grouped());
                if self.observe(&project_id, facts, sink) && memory {
                    // Memory named what was said: it is lined now, and a
                    // line Hide wrote that was said again is offered again.
                    self.group_by_tag(&project_id, true);
                    self.recur(&project_id);
                    self.raise(&project_id);
                    self.publish_if_opened(&project_id, sink);
                }
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
                seen,
            } => {
                let Some(id) = self.resolve(&project, sink) else {
                    return;
                };
                self.locate(&id, &project);
                match action {
                    LineAction::Apply | LineAction::Retry if self.is_local(&id, line) => {
                        self.apply_local(&id, line, seen.as_deref());
                    }
                    LineAction::Apply => self.apply_line(&id, line, Begin::New, false, sink),
                    LineAction::Retry => self.apply_line(&id, line, Begin::Retry, false, sink),
                    LineAction::OpenPr => self.apply_line(&id, line, Begin::OpenPr, false, sink),
                    LineAction::Rebase => self.apply_line(&id, line, Begin::Rebase, false, sink),
                    LineAction::Revert => self.revert_local(&id, line),
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
                // The run may have put new pull request records on a
                // watched line: the runtime asks about their heads first.
                self.publish_followed(sink);
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
            Command::HeadsJudged {
                project_id,
                line,
                merge_commit,
                heads,
            } => {
                if self.judge_heads(&project_id, line, merge_commit, heads) {
                    if self.settle(&project_id) {
                        self.publish_if_opened(&project_id, sink);
                    }
                    self.publish_followed(sink);
                }
            }
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
    fn observe(&mut self, project: &str, facts: Vec<Observation>, sink: &impl Sink) -> bool {
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
        let recorded = match store.record(project, kept, now) {
            Ok(recorded) => recorded,
            Err(code) => {
                log_store("observe", &code);
                return false;
            }
        };
        let (written, moved, pruned) = (recorded.written, recorded.moved, recorded.pruned);
        if !written.is_empty() || !moved.is_empty() || pruned.records > 0 {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "records.kept",
                "project_id": project, "written": written.len(), "moved": moved.len(),
                "pruned": pruned.records, "references_pruned": pruned.references,
            }));
        }
        if !moved.is_empty() {
            // A watched line's record may now name a head GitHub was not
            // asked about.
            self.publish_followed(sink);
        }
        !written.is_empty()
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
        how: Begin,
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
        let previous_agent = matches!(how, Begin::OpenPr | Begin::Rebase)
            .then(|| life.attempt.as_ref()?.agent_id.clone())
            .flatten();
        let Ok((outcome, branch, agent_name, brief, purpose)) =
            applying::begin(&mut line, &mut life, &index, how, auto, now)
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
            "project_id": project, "line": line_id.0, "auto": auto, "how": begin_code(how),
            "outcome": match outcome { Outcome::Pr => "pr", Outcome::Issue => "issue", Outcome::Local => "local" },
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
            purpose,
            previous_agent,
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
        let closed = matches!(change, AttemptMove::Closed | AttemptMove::Discarded);
        if !applying::moved(&mut line, &mut life, started, change, now) {
            return;
        }
        if closed {
            // Closed unmerged, the line is skipped as `넘어가기` skips it.
            match store.index(project, &notebook) {
                Ok(index) => life.skipped_with = Some(scoring::evidence(&line, &index, |_| true)),
                Err(code) => return log_store("attempt", &code),
            }
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
            self.apply_line(project, line, Begin::New, true, sink);
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

    /// Keeps what GitHub said about a watched line's merge; answers whether
    /// the watch changed.
    fn judge_heads(
        &mut self,
        project: &str,
        line_id: LineId,
        merge_commit: String,
        heads: BTreeMap<String, bool>,
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
        let Some(line) = notebook.line(line_id) else {
            return false;
        };
        let mut life = lives.get(&line_id).cloned().unwrap_or_default();
        if !watching::judged(line, &mut life, merge_commit, heads) {
            return false;
        }
        if let Err(code) = store.save_line(project, line, &life) {
            log_store("watch", &code);
            return false;
        }
        true
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
                        let records_of = |line: &Line| {
                            store
                                .records_of(&project, &line.records)
                                .unwrap_or_else(|code| {
                                    log_store("followed", &code);
                                    Vec::new()
                                })
                        };
                        watches.extend(watching::watched(
                            &project, location, &notebook, &lives, records_of,
                        ));
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
            SettingsChange::SignalGroup(group, on) => {
                if on {
                    settings.groups_off.remove(&group);
                } else {
                    settings.groups_off.insert(group);
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
        let mut life = lives.get(&line_id).cloned().unwrap_or_default();
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
                let index = match store.index(project, &notebook) {
                    Ok(index) => index,
                    Err(code) => return log_store("action", &code),
                };
                line.struck = Some(Strike {
                    by: StruckBy::Operator,
                    reason: None,
                    at_unix_ms: now,
                });
                line.stage = Stage::Candidate;
                life.skipped_with = Some(scoring::evidence(&line, &index, |_| true));
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
            // Begun by `apply_line` or `apply_local`, never here.
            LineAction::Apply
            | LineAction::Retry
            | LineAction::OpenPr
            | LineAction::Rebase
            | LineAction::Revert => return,
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

    /// Raises what the scores and caps let rise (D-12, D-42), unless the
    /// Project's last run failed: until a run investigates, nothing rises
    /// from the scores alone, whatever asks (D-35). A line the operator
    /// skipped competes again once its new evidence outgrew the skip, and
    /// only the first time (D-43).
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
        if !state.settings.active() || state.uninvestigated {
            return;
        }
        let open = notebook
            .lines
            .iter()
            .filter(|line| line.stage == Stage::Suggested && line.struck.is_none())
            .count();
        let returning = notebook
            .lines
            .iter()
            .filter(|line| {
                line.stage == Stage::Candidate
                    && line.struck.as_ref().is_some_and(|strike| {
                        strike.by == StruckBy::Operator
                            && lives.get(&line.id).is_some_and(|life| {
                                life.skip_may_return()
                                    && life.skipped_with.as_ref().is_some_and(|with| {
                                        scoring::outgrew_skip(line, &index, strike.at_unix_ms, with)
                                    })
                            })
                    })
            })
            .map(|line| Line {
                struck: None,
                ..line.clone()
            })
            .collect::<Vec<_>>();
        let scored = notebook
            .lines
            .iter()
            .filter(|line| line.stage == Stage::Candidate && line.struck.is_none())
            .chain(&returning)
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
            let returned = line.struck.take().is_some();
            line.stage = Stage::Suggested;
            line.updated_at_unix_ms = now;
            let mut life = lives.get(id).cloned().unwrap_or_default();
            life.raised_at_unix_ms = Some(now);
            if returned {
                life.returned_at_unix_ms = Some(now);
            }
            if let Err(code) = store.save_line(project, &line, &life) {
                return log_store("raise", &code);
            }
            state.raised_at.push(now);
            state.quiet_since = Some(now);
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "line.raised",
                "project_id": project, "line": id.0, "points": (points * 100.0).round() / 100.0,
                "records": line.records.len(), "returned": returned,
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
    /// watches stay as they are. Any other end raises what rose, once the
    /// day is saved as investigated.
    fn complete(&mut self, project: &str, end: AnalysisEnd, edits: usize) {
        let today = self.today();
        let failed = end == AnalysisEnd::Partial(PartialReason::Failed);
        let mut ended = match end {
            AnalysisEnd::Done => "done",
            AnalysisEnd::Partial(reason) => reason.as_str(),
        };
        if failed && edits == 0 {
            self.group_by_tag(project, false);
            ended = "not_investigated";
        }
        crate::diagnostic!(json!({
            "component": "suggestions", "kind": "run.ended",
            "project_id": project, "end": ended, "edits": edits,
        }));
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
            return log_store("complete", &code);
        }
        self.raise(project);
    }

    /// The deterministic grouping over the store directly, for a run that
    /// could not write, or, with `memory_only`, for the records Memory named
    /// (PRD memory-self-improve D-37), which no investigation regroups.
    fn group_by_tag(&mut self, project: &str, memory_only: bool) {
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let loaded = store.notebook(project).and_then(|(notebook, _)| {
            let index = store.index(project, &notebook)?;
            Ok((notebook, index))
        });
        let (notebook, mut index) = match loaded {
            Ok(loaded) => loaded,
            Err(code) => return log_store("fallback", &code),
        };
        if memory_only {
            for entry in &mut index.entries {
                entry.fresh &= entry.signal.tag_grouped();
            }
        }
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

    /// The core's state folder, where the Hide global instruction is.
    fn state_dir(&self) -> PathBuf {
        self.options
            .store
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default()
    }

    /// Whether `line` is fixed in the Hide global instruction.
    fn is_local(&mut self, project: &str, line: LineId) -> bool {
        let Ok(store) = self.store.as_mut() else {
            return false;
        };
        store.notebook(project).is_ok_and(|(notebook, _)| {
            notebook
                .line(line)
                .is_some_and(|line| line.outcome == Some(Outcome::Local))
        })
    }

    /// Writes a new suggestion's line into the Hide global instruction on the
    /// operator's press, whatever the apply mode, with no GitHub (PRD
    /// memory-self-improve D-07, D-33, B8, B10, B29). The press carries the
    /// token of the line and file it was shown; any other line or file is
    /// refused with a new preview. The same press twice, or again after a
    /// write whose answer was lost, writes it once; a line the operator
    /// already had is adopted, not written, and its Undo takes nothing out.
    fn apply_local(&mut self, project: &str, line_id: LineId, seen: Option<&str>) {
        let now = self.now();
        let state_dir = self.state_dir();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let loaded = store.notebook(project).and_then(|(notebook, lives)| {
            let index = store.index(project, &notebook)?;
            Ok((notebook, lives, index))
        });
        let (notebook, lives, index) = match loaded {
            Ok(loaded) => loaded,
            Err(code) => return log_store("apply_local", &code),
        };
        let Some(mut line) = notebook.line(line_id).cloned() else {
            return;
        };
        if line.stage != Stage::Suggested || line.struck.is_some() {
            return;
        }
        let mut life = lives.get(&line_id).cloned().unwrap_or_default();
        let Some(text) = read::local_line(&line, &index) else {
            return;
        };
        let local = life.local.get_or_insert_with(Default::default);
        // A write saved before it was made and the line now in the file: that
        // write landed and its answer was lost.
        let ours = local.writing == Some(LocalWrite::Apply(text.clone()));
        let mut adopted = false;
        let written = match global_instruction::holds(&state_dir, &text) {
            Ok(true) => {
                adopted = !ours;
                Ok(())
            }
            Ok(false) => match global_instruction::preview(&state_dir, &text) {
                Ok(preview) if seen != Some(preview.token().as_str()) => {
                    Err(global_instruction::Refusal::Changed { preview })
                }
                Ok(preview) => {
                    local.writing = Some(LocalWrite::Apply(text.clone()));
                    if let Err(code) = store.save_line(project, &line, &life) {
                        return log_store("apply_local", &code);
                    }
                    let local = life.local.get_or_insert_with(Default::default);
                    let written = global_instruction::apply(&state_dir, &preview);
                    local.writing = None;
                    written
                }
                Err(refusal) => Err(refusal),
            },
            Err(unusable) => Err(global_instruction::Refusal::Unusable { unusable }),
        };
        let local = life.local.get_or_insert_with(Default::default);
        local.writing = None;
        let outcome = match written {
            Ok(()) => {
                local.adopted = adopted;
                local.line = Some(text);
                local.applied_at_unix_ms = Some(now);
                local.recurred_at_unix_ms = None;
                local.failure = None;
                line.stage = Stage::Settled;
                "applied"
            }
            Err(refusal) => {
                let failure = LocalFailure::of(&refusal);
                local.failure = Some(failure);
                match failure {
                    LocalFailure::NoRoom => "no_room",
                    LocalFailure::TooLarge => "too_large",
                    LocalFailure::Unreadable => "unreadable",
                    LocalFailure::Changed => "changed",
                    LocalFailure::Edited => "edited",
                    LocalFailure::WriteFailed => "write_failed",
                    LocalFailure::EvidenceUnconfirmed => "evidence_unconfirmed",
                    LocalFailure::BadLine => "bad_line",
                }
            }
        };
        line.updated_at_unix_ms = now;
        if let Err(code) = store.save_line(project, &line, &life) {
            return log_store("apply_local", &code);
        }
        crate::diagnostic!(json!({
            "component": "suggestions", "kind": "global_instruction.applied",
            "project_id": project, "line": line_id.0, "outcome": outcome,
        }));
    }

    /// Takes the line Hide wrote back out of its global instruction, and only
    /// it (D-30, B9): the row returns to 고를 것. A line the operator edited
    /// or removed is left as it is and the row says so; a line they had
    /// before the press stays; a second press, or one after a write whose
    /// answer was lost, changes nothing more.
    fn revert_local(&mut self, project: &str, line_id: LineId) {
        let now = self.now();
        let state_dir = self.state_dir();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let (notebook, lives) = match store.notebook(project) {
            Ok(loaded) => loaded,
            Err(code) => return log_store("revert_local", &code),
        };
        let Some(mut line) = notebook.line(line_id).cloned() else {
            return;
        };
        let mut life = lives.get(&line_id).cloned().unwrap_or_default();
        let Some(local) = life.local.as_mut() else {
            return;
        };
        let Some(text) = local.line.clone().filter(|_| line.stage == Stage::Settled) else {
            return;
        };
        let reverted = if local.adopted {
            Ok(())
        } else {
            let ours = local.writing == Some(LocalWrite::Revert(text.clone()));
            match global_instruction::holds(&state_dir, &text) {
                Ok(false) if ours => Ok(()),
                _ => {
                    local.writing = Some(LocalWrite::Revert(text.clone()));
                    if let Err(code) = store.save_line(project, &line, &life) {
                        return log_store("revert_local", &code);
                    }
                    global_instruction::revert(&state_dir, &text)
                }
            }
        };
        let Some(local) = life.local.as_mut() else {
            return;
        };
        local.writing = None;
        let outcome = match reverted {
            Ok(()) => {
                local.adopted = false;
                local.line = None;
                local.applied_at_unix_ms = None;
                local.recurred_at_unix_ms = None;
                local.failure = None;
                line.stage = Stage::Suggested;
                "reverted"
            }
            Err(refusal) => {
                local.failure = Some(LocalFailure::of(&refusal));
                "refused"
            }
        };
        line.updated_at_unix_ms = now;
        if let Err(code) = store.save_line(project, &line, &life) {
            return log_store("revert_local", &code);
        }
        crate::diagnostic!(json!({
            "component": "suggestions", "kind": "global_instruction.reverted",
            "project_id": project, "line": line_id.0, "outcome": outcome,
        }));
    }

    /// Offers a global instruction line again when the operator said it in a
    /// session that began after Hide wrote it, so was given it (D-33, B11).
    /// Nothing climbs to a check or a hook by itself.
    fn recur(&mut self, project: &str) {
        let now = self.now();
        let Ok(store) = self.store.as_mut() else {
            return;
        };
        let loaded = store.notebook(project).and_then(|(notebook, lives)| {
            let index = store.index(project, &notebook)?;
            Ok((notebook, lives, index))
        });
        let (notebook, lives, index) = match loaded {
            Ok(loaded) => loaded,
            Err(code) => return log_store("recur", &code),
        };
        for line in &notebook.lines {
            if line.stage != Stage::Settled || line.outcome != Some(Outcome::Local) {
                continue;
            }
            let Some(mut life) = lives.get(&line.id).cloned() else {
                continue;
            };
            let Some(applied) = life
                .local
                .as_ref()
                .and_then(|local| local.applied_at_unix_ms)
            else {
                continue;
            };
            let given = line
                .records
                .iter()
                .filter_map(|id| index.get(*id))
                .any(|record| {
                    record.signal.tag_grouped()
                        && record.at_unix_ms > applied
                        && record
                            .said
                            .as_ref()
                            .and_then(|said| said.session_started_at_unix_ms)
                            .is_some_and(|started| started > applied)
                });
            if !given {
                continue;
            }
            let mut line = line.clone();
            line.stage = Stage::Suggested;
            line.updated_at_unix_ms = now;
            if let Some(local) = life.local.as_mut() {
                local.recurred_at_unix_ms = Some(now);
            }
            if let Err(code) = store.save_line(project, &line, &life) {
                return log_store("recur", &code);
            }
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "global_instruction.recurred",
                "project_id": project, "line": line.id.0,
            }));
        }
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
                let sources =
                    evidence_sources(store, project, &state, &notebook, &lives, &index, sink)?;
                let global = read::GlobalRead::of(&self.state_dir());
                Ok::<_, String>(read::project(&ReadInput {
                    global: &global,
                    workspace_id: &workspace_id,
                    state: &state,
                    notebook: &notebook,
                    lives: &lives,
                    index: &index,
                    investigating,
                    intro_seen,
                    today: &today,
                    sources: &sources,
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

/// What a read's evidence needs beyond the index: each pull request's
/// address from its record, and which sessions the link record on the
/// Project's machine still holds (B10).
fn evidence_sources(
    store: &SuggestionStore,
    project: &str,
    state: &super::store::ProjectState,
    notebook: &Notebook,
    lives: &BTreeMap<LineId, super::lifecycle::LineLife>,
    index: &RecordIndex,
    sink: &impl Sink,
) -> Result<read::EvidenceSources, String> {
    let shown = read::shown_evidence(notebook, lives, index);
    let prs = shown
        .iter()
        .filter(|record| record.pr.is_some())
        .map(|record| record.id)
        .collect::<Vec<_>>();
    let pr_urls = store
        .records_of(project, &prs)?
        .into_iter()
        .filter_map(|record| Some((record.id, record.fact.pr?.url)))
        .collect();
    let sessions = shown
        .iter()
        .filter_map(|record| record.session.clone())
        .collect::<BTreeSet<_>>();
    let kept = match &state.location {
        Some(location) if !sessions.is_empty() => sink.kept_sessions(&location.node, &sessions),
        _ => None,
    };
    Ok(read::EvidenceSources { kept, pr_urls })
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
        AttemptPhase::Ready => "ready",
        AttemptPhase::PrOpen => "pr_open",
        AttemptPhase::CiFailed => "ci_failed",
        AttemptPhase::Failed => "failed",
        AttemptPhase::IssueOpen => "issue_open",
        AttemptPhase::Merged => "merged",
        AttemptPhase::Closed => "closed",
    }
}

fn begin_code(how: Begin) -> &'static str {
    match how {
        Begin::New => "new",
        Begin::Retry => "retry",
        Begin::OpenPr => "open_pr",
        Begin::Rebase => "rebase",
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
    use crate::suggestions::ledger::{ImprovementKind, SessionRef, Signal};
    use crate::suggestions::lifecycle::LineLife;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    const START: u64 = 400 * policy::DAY_MS + 9 * 3_600_000;
    const WAIT: Duration = Duration::from_secs(10);

    struct TestSink {
        reads: mpsc::Sender<ProjectSuggestionsSnapshot>,
        restored: mpsc::Sender<String>,
        orders: mpsc::Sender<ApplyOrder>,
        followed: mpsc::Sender<Vec<FollowedAttempt>>,
        gate: RunGate,
        /// The sessions whose record can no longer be opened.
        gone: Gone,
    }

    type Gone = Arc<std::sync::Mutex<BTreeSet<SessionRef>>>;

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
        fn kept_sessions(
            &self,
            _node: &str,
            sessions: &BTreeSet<SessionRef>,
        ) -> Option<BTreeSet<SessionRef>> {
            let gone = self.gone.lock().unwrap();
            Some(sessions.difference(&gone).cloned().collect())
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
        gone: Gone,
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
            let gone = Gone::default();
            let (worker, reads, restored, handed) = Self::spawn(&store, &clock, &analysis, &gone);
            Self {
                worker: Some(worker),
                analysis,
                clock,
                reads,
                restored,
                handed,
                _dir: dir,
                store,
                gone,
            }
        }

        fn spawn(
            store: &std::path::Path,
            clock: &Arc<AtomicU64>,
            analysis: &AnalysisFactory,
            gone: &Gone,
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
                gone: Arc::clone(gone),
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
                Self::spawn(&self.store, &self.clock, &self.analysis, &self.gone);
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
                        head: None,
                        improvement: None,
                        said: None,
                    })
                    .collect(),
            });
        }

        /// One improvement with the same words in each session, of each
        /// session's kind.
        fn improve_in_sessions(&self, text: &str, sessions: &[(&str, ImprovementKind)]) {
            let at = self.clock.load(Ordering::SeqCst);
            self.send(Command::Observe {
                project: project(),
                facts: sessions
                    .iter()
                    .map(|(session, kind)| Observation {
                        signal: Signal::Improvement,
                        tag: text.into(),
                        session: Some(SessionRef {
                            agent: "codex".into(),
                            id: (*session).into(),
                        }),
                        pr: None,
                        checkout: Some("/repo".into()),
                        at_unix_ms: at,
                        always: false,
                        head: None,
                        improvement: Some(*kind),
                        said: None,
                    })
                    .collect(),
            });
        }

        /// Wakes the worker and moves the clock past the wake's delay. The
        /// clock moves only once the worker has taken the wake and looked at
        /// the clock after it (`handled`); the intro command then makes it
        /// look again. A read does not prove this: one an earlier command
        /// published can still be waiting, and a clock moved before the wake
        /// was taken put the run three real minutes away.
        fn wake_and_wait_out_the_delay(&self) {
            self.send(Command::Wake);
            self.handled();
            self.clock
                .fetch_add(policy::WAKE_DELAY_MS + 1, Ordering::SeqCst);
            self.send(Command::IntroSeen);
        }

        /// Returns once the worker has handled every command sent before
        /// and looked at the clock after them. A pen with no run of its
        /// Project is refused and changes nothing; its answer comes after
        /// the worker has passed over each earlier command.
        fn handled(&self) {
            let (reply, answer) = mpsc::channel();
            self.send(Command::Pen {
                project: "no-run".into(),
                edit: NotebookEdit::Merge {
                    into: LineId(1),
                    from: LineId(2),
                },
                reply,
            });
            assert!(matches!(
                answer.recv_timeout(WAIT).expect("the pen's answer"),
                Err(PenRefused::Stopped)
            ));
        }
    }

    /// The operator saying `meaning` at `offset` in `session`, begun at
    /// `started`, as Memory's analysis hands it.
    fn said(session: &str, offset: u64, at: u64, started: u64) -> Observation {
        Observation {
            signal: crate::suggestions::ledger::Signal::RepeatedRequest,
            tag: "답은 한국어로".into(),
            session: Some(SessionRef {
                agent: "claude".into(),
                id: session.into(),
            }),
            pr: None,
            checkout: Some("/repo".into()),
            at_unix_ms: at,
            always: false,
            head: None,
            improvement: None,
            said: Some(crate::suggestions::ledger::Said {
                personal: true,
                procedure: false,
                line: "Answer the operator in Korean.".into(),
                offset,
                words: Some(format!("{session}-{offset}")),
                session_started_at_unix_ms: Some(started),
                given: None,
            }),
        }
    }

    /// PRD memory-self-improve B6, B8, B9, B11, B13: a personal request said
    /// in two sessions with one more rises at once as a global instruction
    /// line with no GitHub; the press writes it once, Undo takes only it out
    /// and returns the row, and saying it again in a session begun after it
    /// was written offers it again.
    #[test]
    fn a_repeated_personal_request_is_written_into_the_global_instruction_and_offered_again() {
        let fixture = Fixture::new();
        let state_dir = fixture.store.parent().unwrap().to_owned();
        std::fs::write(
            hide_kit::layout::global_instruction(&state_dir),
            "My own line.\n",
        )
        .unwrap();
        fixture.open();
        let observe = |facts| {
            fixture.send(Command::ObserveProject {
                project_id: "project/repo".into(),
                facts,
            })
        };
        observe(vec![said("a", 1, START - 3, START - 10)]);
        observe(vec![
            said("a", 4, START - 2, START - 10),
            said("b", 2, START - 1, START - 5),
            // A re-read of the same words is the same thing said.
            said("b", 2, START - 1, START - 5),
        ]);
        let read = fixture.read_until(|read| !chosen(read).is_empty());
        let row = &read.rows[0];
        assert_eq!(row.outcome, Outcome::Local);
        assert_eq!(row.hold, None, "no GitHub is needed");
        let local = row.local.as_ref().unwrap();
        assert_eq!(local.line, "Answer the operator in Korean.");
        let preview = local.preview.clone().unwrap();
        let line = read::line_of_row(&row.id).unwrap();
        let act = |action, seen: Option<String>| {
            fixture.send(Command::Action {
                project: project(),
                line,
                action,
                seen,
            })
        };
        act(LineAction::Apply, Some(preview.clone()));
        act(LineAction::Apply, Some(preview));
        let read = fixture.read_until(|read| read.rows[0].group == SuggestionGroup::Applied);
        assert!(read.rows[0].local.as_ref().unwrap().present);
        let file = || std::fs::read_to_string(hide_kit::layout::global_instruction(&state_dir));
        assert_eq!(
            file().unwrap(),
            "My own line.\nAnswer the operator in Korean.\n",
            "two presses write it once, and the operator's line stays"
        );
        act(LineAction::Revert, None);
        act(LineAction::Revert, None);
        fixture.read_until(|read| read.rows[0].group == SuggestionGroup::Choose);
        assert_eq!(file().unwrap(), "My own line.\n");

        let read = fixture.next_read();
        let preview = read.rows[0].local.as_ref().unwrap().preview.clone();
        act(LineAction::Apply, preview);
        fixture.read_until(|read| read.rows[0].group == SuggestionGroup::Applied);
        fixture.clock.fetch_add(policy::DAY_MS, Ordering::SeqCst);
        // A session begun before it was written was not given it.
        observe(vec![said("c", 3, START + 10, START - 1)]);
        observe(vec![said("d", 3, START + 20, START + 15)]);
        let read = fixture.read_until(|read| read.rows[0].group == SuggestionGroup::Choose);
        assert!(read.rows[0].local.as_ref().unwrap().recurred);
        assert!(read.rows[0].local.as_ref().unwrap().present);
    }

    /// B8, B9: a press without the token of the line and file it was shown
    /// writes nothing, and a line the operator already had is adopted, so
    /// its Undo leaves their file as it is.
    #[test]
    fn a_press_writes_only_what_it_was_shown_and_never_takes_out_the_operators_own_line() {
        let fixture = Fixture::new();
        let state_dir = fixture.store.parent().unwrap().to_owned();
        let path = hide_kit::layout::global_instruction(&state_dir);
        std::fs::write(&path, "My own line.\n").unwrap();
        fixture.open();
        let observe = |facts| {
            fixture.send(Command::ObserveProject {
                project_id: "project/repo".into(),
                facts,
            })
        };
        observe(vec![said("a", 1, START - 3, START - 10)]);
        observe(vec![
            said("a", 4, START - 2, START - 10),
            said("b", 2, START - 1, START - 5),
        ]);
        let read = fixture.read_until(|read| !chosen(read).is_empty());
        let line = read::line_of_row(&read.rows[0].id).unwrap();
        let act = |action, seen: Option<String>| {
            fixture.send(Command::Action {
                project: project(),
                line,
                action,
                seen,
            })
        };
        act(LineAction::Apply, None);
        act(LineAction::Apply, Some("another line's token".into()));
        let read = fixture.read_until(|read| {
            read.rows[0].local.as_ref().unwrap().failure == Some(LocalFailure::Changed)
        });
        assert_eq!(read.rows[0].group, SuggestionGroup::Choose);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "My own line.\n");

        // The operator writes the same line by hand, then presses.
        std::fs::write(&path, "My own line.\n  Answer the operator in Korean.\n").unwrap();
        act(LineAction::Apply, None);
        fixture.read_until(|read| read.rows[0].group == SuggestionGroup::Applied);
        act(LineAction::Revert, None);
        fixture.read_until(|read| read.rows[0].group == SuggestionGroup::Choose);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "My own line.\n  Answer the operator in Korean.\n",
            "the operator's own line stays"
        );
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

    /// B10: evidence whose session the link record no longer holds is drawn
    /// gone on the next read, and a pull request's evidence carries the
    /// address its record was seen with, so both open or neither does.
    #[test]
    fn evidence_whose_session_is_gone_is_drawn_gone_and_a_pull_request_has_its_address() {
        let fixture = Fixture::new();
        fixture.open();
        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3"]);
        let at = fixture.clock.load(Ordering::SeqCst);
        fixture.send(Command::Observe {
            project: project(),
            facts: [41, 42, 43]
                .into_iter()
                .map(|number| Observation {
                    signal: Signal::CheckFailed,
                    tag: "ci / test".into(),
                    session: None,
                    pr: Some(super::super::ledger::PrRef {
                        number,
                        url: format!("https://github.com/acme/app/pull/{number}"),
                    }),
                    checkout: None,
                    at_unix_ms: at,
                    always: false,
                    head: None,
                    improvement: None,
                    said: None,
                })
                .collect(),
        });
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| chosen(read).len() == 2);
        let evidence = |read: &ProjectSuggestionsSnapshot| {
            read.rows
                .iter()
                .flat_map(|row| &row.evidence)
                .map(|evidence| {
                    let name = evidence
                        .session_id
                        .clone()
                        .or(evidence.url.clone())
                        .expect("a session or an address");
                    (name, evidence.gone)
                })
                .collect::<BTreeMap<_, _>>()
        };
        let open = |names: &[&str]| {
            names
                .iter()
                .map(|name| ((*name).to_owned(), false))
                .collect::<BTreeMap<_, _>>()
        };
        let all = [
            "s1",
            "s2",
            "s3",
            "https://github.com/acme/app/pull/41",
            "https://github.com/acme/app/pull/42",
            "https://github.com/acme/app/pull/43",
        ];
        assert_eq!(evidence(&read), open(&all));

        fixture.gone.lock().unwrap().insert(SessionRef {
            agent: "claude".into(),
            id: "s2".into(),
        });
        let mut expected = open(&all);
        expected.insert("s2".into(), true);
        assert_eq!(evidence(&fixture.open()), expected);
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
    /// record came since, and only then does the waiting line rise. A
    /// settings change or a row's action in between raises nothing either.
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

        let read = fixture.settings(SettingsChange::Quantity(Quantity::Many));
        assert_eq!(read.settings.quantity, Quantity::Many);
        assert_eq!(chosen(&read), ["s1"], "a settings change raises nothing");
        fixture.send(Command::Action {
            project: project(),
            line: LineId(1),
            action: LineAction::Dismiss,
            seen: None,
        });
        let read = fixture.read_until(|read| chosen(read).is_empty());
        assert!(
            !shown(&read).contains(&"s2"),
            "the freed place is not filled from the scores"
        );
        assert_eq!(read.status, SuggestionsStatus::NotInvestigated);

        fails.store(false, Ordering::SeqCst);
        fixture.clock.fetch_add(policy::DAY_MS, Ordering::SeqCst);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| !chosen(read).is_empty());
        assert_eq!(chosen(&read), ["s2"]);
        assert_eq!(read.status, SuggestionsStatus::Idle);
        assert_eq!(
            runs.load(Ordering::SeqCst),
            3,
            "the third day ran with no new record"
        );
    }

    /// B25: the same cause in three sessions rises as one line through the
    /// same fold, grouping and scores as any record, and each evidence row
    /// names its session's kind.
    #[test]
    fn improvements_from_three_sessions_rise_as_one_line_with_their_kinds() {
        let fixture = Fixture::new();
        fixture.open();
        let text = "dev 서버와 e2e 포트 규칙이 문서에 없어 설정 파일 6개를 뒤져 찾음";
        fixture.improve_in_sessions(
            text,
            &[
                ("a", ImprovementKind::NoDoc),
                ("b", ImprovementKind::SearchedAgain),
                ("c", ImprovementKind::WrongGuess),
            ],
        );
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| !chosen(read).is_empty());
        let row = &read.rows[0];
        assert_eq!(
            row.basis
                .as_ref()
                .map(|basis| (basis.signal, basis.tag.as_str())),
            Some((Signal::Improvement, text))
        );
        let mut kinds = row
            .evidence
            .iter()
            .map(|evidence| evidence.improvement)
            .collect::<Vec<_>>();
        kinds.sort();
        assert_eq!(
            kinds,
            [
                Some(ImprovementKind::NoDoc),
                Some(ImprovementKind::SearchedAgain),
                Some(ImprovementKind::WrongGuess),
            ]
        );
    }

    /// B5, B25: with 세션 기록 off, an improvement is not kept, as no other
    /// session record is.
    #[test]
    fn the_session_switch_off_keeps_no_improvement() {
        let fixture = Fixture::new();
        fixture.open();
        let read = fixture.settings(SettingsChange::SignalGroup(SignalGroup::Session, false));
        assert!(
            read.settings
                .signals
                .iter()
                .filter(|signal| signal.signal.group() == SignalGroup::Session)
                .all(|signal| !signal.enabled),
            "the new signal is off with its group"
        );
        fixture.improve_in_sessions("포트 규칙", &[("a", ImprovementKind::NoDoc)]);
        // Commands are taken in order: the read this answers comes after the
        // observation was handled.
        fixture.settings(SettingsChange::SignalGroup(SignalGroup::Session, true));
        let (store, _) = SuggestionStore::open(&fixture.store).unwrap();
        let projects = store.projects().unwrap();
        assert!(
            projects
                .iter()
                .all(|project| store.records(project).unwrap().is_empty()),
            "{projects:?}"
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
            seen: None,
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
            seen: None,
        });
        let order = fixture.handed.orders.recv_timeout(WAIT).expect("an order");
        let merged_at = fixture.clock.load(Ordering::SeqCst);
        fixture.send(Command::Attempted {
            project_id: "project/repo".into(),
            line: LineId(1),
            started_at_unix_ms: order.started_at_unix_ms,
            change: AttemptMove::Merged {
                pr: Some(crate::suggestions::ledger::PrRef {
                    number: 12,
                    url: "https://github.com/acme/app/pull/12".into(),
                }),
                at_unix_ms: merged_at,
                base: Some("main".into()),
                merge_commit: None,
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

    /// B16, D-43: a skipped line comes back once, flagged, when its new
    /// sessions double its evidence; skipped again it stays away however
    /// much more piles up, until ⚙ restores it.
    #[test]
    fn a_skipped_line_comes_back_once_when_its_evidence_doubles() {
        let fixture = Fixture::new();
        fixture.open();
        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3"]);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| !chosen(read).is_empty());
        assert!(!read.rows[0].returned_after_skip);
        let skip = || {
            fixture.send(Command::Action {
                project: project(),
                line: LineId(1),
                action: LineAction::Dismiss,
                seen: None,
            });
            fixture.read_until(|read| chosen(read).is_empty())
        };
        let next_day = |sessions: &[&str]| {
            fixture.clock.fetch_add(policy::DAY_MS, Ordering::SeqCst);
            fixture.fail_in_sessions("cargo test", sessions);
            fixture.wake_and_wait_out_the_delay();
        };
        let dismissed = |read: &ProjectSuggestionsSnapshot| {
            read.settings
                .dismissed
                .iter()
                .map(|line| (line.id.clone(), line.may_return))
                .collect::<Vec<_>>()
        };

        let read = skip();
        assert_eq!(dismissed(&read), [("s1".to_owned(), true)]);

        // s1 goes on past the skip and is not new: two of three.
        next_day(&["s1", "s4", "s5"]);
        let read = fixture.read_until(|read| read.status == SuggestionsStatus::NoneToday);
        assert!(chosen(&read).is_empty(), "not yet doubled");

        next_day(&["s6"]);
        let read = fixture.read_until(|read| !chosen(read).is_empty());
        assert_eq!(chosen(&read), ["s1"]);
        assert!(read.rows[0].returned_after_skip, "it says it came back");
        assert!(read.settings.dismissed.is_empty());

        let read = skip();
        assert_eq!(
            dismissed(&read),
            [("s1".to_owned(), false)],
            "a second skip is for good"
        );
        next_day(&["s7", "s8", "s9", "s10", "s11", "s12", "s13"]);
        let read = fixture.read_until(|read| read.status == SuggestionsStatus::NoneToday);
        assert!(chosen(&read).is_empty(), "never again on its own");

        fixture.send(Command::Action {
            project: project(),
            line: LineId(1),
            action: LineAction::Restore,
            seen: None,
        });
        let read = fixture.read_until(|read| read.settings.dismissed.is_empty());
        assert_eq!(chosen(&read), ["s1"], "restored and judged again");
        assert!(
            !read.rows[0].returned_after_skip,
            "the operator brought it back"
        );
    }

    /// D-43: the line keeps what it stood on at the skip, so the records
    /// leaving the ledger with age do not lower what a return has to reach.
    #[test]
    fn a_skip_is_measured_against_its_evidence_after_the_records_are_gone() {
        let fixture = Fixture::new();
        fixture.open();
        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3", "s4", "s5"]);
        fixture.wake_and_wait_out_the_delay();
        fixture.read_until(|read| !chosen(read).is_empty());
        fixture.send(Command::Action {
            project: project(),
            line: LineId(1),
            action: LineAction::Dismiss,
            seen: None,
        });
        fixture.read_until(|read| chosen(read).is_empty());

        // The five sessions' records are past the ledger's keeping now.
        fixture.clock.fetch_add(
            policy::RECORD_RETENTION_MS + policy::DAY_MS,
            Ordering::SeqCst,
        );
        fixture.fail_in_sessions("cargo test", &["s6", "s7", "s8"]);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| read.status == SuggestionsStatus::NoneToday);
        assert!(
            chosen(&read).is_empty(),
            "three new sessions of five, though none of the five is left"
        );

        fixture.clock.fetch_add(policy::DAY_MS, Ordering::SeqCst);
        fixture.fail_in_sessions("cargo test", &["s9", "s10"]);
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| !chosen(read).is_empty());
        assert!(read.rows[0].returned_after_skip);
    }

    /// D-43: a skip that kept nothing of what the line stood on is for
    /// good, however much piles up after it; ⚙ still restores it.
    #[test]
    fn a_skip_without_kept_evidence_stays_skipped() {
        let mut fixture = Fixture::new();
        fixture.open();
        fixture.fail_in_sessions("cargo test", &["s1", "s2", "s3"]);
        fixture.wake_and_wait_out_the_delay();
        fixture.read_until(|read| !chosen(read).is_empty());
        fixture.send(Command::Action {
            project: project(),
            line: LineId(1),
            action: LineAction::Dismiss,
            seen: None,
        });
        fixture.read_until(|read| chosen(read).is_empty());

        // The store as a skip that kept nothing leaves it.
        fixture.worker.take();
        let (mut store, _) = SuggestionStore::open(&fixture.store).unwrap();
        let id = store.projects().unwrap().remove(0);
        let (notebook, lives) = store.notebook(&id).unwrap();
        let life = LineLife {
            skipped_with: None,
            ..lives[&LineId(1)].clone()
        };
        store
            .save_line(&id, notebook.line(LineId(1)).unwrap(), &life)
            .unwrap();
        drop(store);
        fixture.restart();
        let read = fixture.open();
        assert_eq!(
            read.settings
                .dismissed
                .iter()
                .map(|line| (line.id.as_str(), line.may_return))
                .collect::<Vec<_>>(),
            [("s1", false)]
        );

        fixture.clock.fetch_add(policy::DAY_MS, Ordering::SeqCst);
        fixture.fail_in_sessions(
            "cargo test",
            &["s4", "s5", "s6", "s7", "s8", "s9", "s10", "s11", "s12"],
        );
        fixture.wake_and_wait_out_the_delay();
        let read = fixture.read_until(|read| read.status == SuggestionsStatus::NoneToday);
        assert!(chosen(&read).is_empty(), "nine new sessions of three");

        fixture.send(Command::Action {
            project: project(),
            line: LineId(1),
            action: LineAction::Restore,
            seen: None,
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
        let read = fixture.settings(SettingsChange::SignalGroup(SignalGroup::Github, false));
        let off = |read: &ProjectSuggestionsSnapshot| {
            read.settings
                .signals
                .iter()
                .filter(|signal| !signal.enabled)
                .map(|signal| signal.signal)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            off(&read),
            [Signal::CheckFailed, Signal::ChangeRequest],
            "one press turns GitHub's every signal off"
        );
        fixture.fail_in_sessions("cargo test", &["s1", "s2"]);
        fixture.wake_and_wait_out_the_delay();
        // The automatic mode applies the line it raised at once.
        let before = fixture.read_until(|read| !shown(read).is_empty());

        fixture.restart();
        let after = fixture.open();
        assert_eq!(after.settings.quantity, Quantity::Many);
        assert_eq!(after.settings.apply_mode, ApplyMode::AutoPr);
        assert_eq!(off(&after), [Signal::CheckFailed, Signal::ChangeRequest]);
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
                gone: Gone::default(),
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
                    head: None,
                    improvement: None,
                    said: None,
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
                gone: Gone::default(),
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
                head: None,
                improvement: None,
                said: None,
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
