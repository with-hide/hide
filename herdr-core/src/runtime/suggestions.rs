//! The Hide AI tool's 제안 tab as the runtime holds it (PRD
//! hide-ai-suggestions): which Project the tool opened, the suggestion
//! worker's last read of it, and the section composed from that read and
//! the facts only the lock has (the Memory tab's own count, Hide AI's
//! readiness, GitHub's answer). Reading and writing the suggestion store
//! happens on the worker; the lock opens, composes and hands commands over.

use super::*;
use crate::model::{
    MemoryAction, ProjectSuggestionsSnapshot, SignalUnavailable, SuggestionCountsSnapshot, SuggestionGroup,
    SuggestionHold, SuggestionState, SuggestionsAi, SuggestionsGithub, SuggestionsSnapshot,
};
use crate::suggestions::ledger::Signal;
use crate::suggestions::notebook::Outcome;
use crate::suggestions::policy::Quantity;
use crate::suggestions::read::line_of_row;
use crate::suggestions::settings::ApplyMode;
use crate::suggestions::store::ProjectLocation;
use crate::suggestions::worker::{
    Command, LineAction, Options, ProjectRef, RunGate, SettingsChange, Sink, SuggestionClient,
    SuggestionWorker,
};
use hide_node_link::gh::GithubFailureCategory;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::sync::{Arc, Weak};

#[derive(Default)]
pub(super) struct SuggestionsWork {
    pub(super) client: Option<SuggestionClient>,
    /// The catalog Project the Hide AI tool opened, with the machine that
    /// holds its folder.
    pub(super) opened: Option<OpenedProject>,
    /// The worker's last read of the opened Project, before the lock's
    /// facts are laid on it.
    pub(super) read: Option<Arc<ProjectSuggestionsSnapshot>>,
    /// The lock's facts the section was last composed with.
    laid: Option<LockFacts>,
    /// The Memory facts its Memory rows were last drawn from (D-02).
    laid_memory: Option<super::suggestion_memory::MemoryFacts>,
    pub(super) promoting: super::suggestion_memory::PromotingCache,
    /// The attempts applying lines, started and followed here (B11-B14).
    pub(super) apply: super::suggestion_apply::ApplyWork,
    /// The pull request records already handed to the worker, so a GitHub
    /// read that moved nothing for them sends nothing.
    sent_prs: HashSet<String>,
    /// Open pull requests a review asked changes of, until they merge
    /// (B4: a review's change that was fixed and merged).
    changes_requested: HashSet<String>,
}

/// The most pull request keys either set keeps; past it the set starts
/// over, which costs at most one resend the store folds.
const PR_KEYS: usize = 8192;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OpenedProject {
    pub(super) workspace_id: String,
    pub(super) node: crate::node::NodeId,
    pub(super) root: String,
}

/// What the lock alone knows about the opened Project's suggestions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LockFacts {
    ai: SuggestionsAi,
    github: SuggestionsGithub,
}

/// `suggestion_action`: one press on a row (B11, B16).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SuggestionActionPayload {
    workspace_id: String,
    id: String,
    action: SuggestionActionKind,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SuggestionActionKind {
    Apply,
    Dismiss,
    Restore,
    Retry,
}

/// `suggestion_settings`: exactly one of ⚙'s settings.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SuggestionSettingsPayload {
    workspace_id: String,
    #[serde(default)]
    suggestions_enabled: Option<bool>,
    #[serde(default)]
    quantity: Option<Quantity>,
    #[serde(default)]
    apply_mode: Option<ApplyMode>,
    #[serde(default)]
    signal: Option<SignalSwitch>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalSwitch {
    signal: Signal,
    enabled: bool,
}

/// `hide_ai_switch`: the tool head's switch (B3).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HideAiSwitchPayload {
    workspace_id: String,
    on: bool,
}

/// `hide_ai_wake`: the app opened, the device woke or the screen unlocked
/// (B6), as the desktop host saw it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HideAiWakePayload {
    cause: WakeCause,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum WakeCause {
    AppOpen,
    Resume,
    Unlock,
}

impl WakeCause {
    fn as_str(self) -> &'static str {
        match self {
            Self::AppOpen => "app_open",
            Self::Resume => "resume",
            Self::Unlock => "unlock",
        }
    }
}

impl Runtime {
    /// Follows the Hide AI tool's `memory_open`: the 제안 tab opens and closes
    /// with the Memory tab, since they are one tool (D-01).
    pub(super) fn open_suggestions(&mut self, workspace_id: Option<String>) -> bool {
        let found = workspace_id.and_then(|workspace_id| self.suggestion_project(&workspace_id));
        let opened = found.as_ref().map(|(opened, _)| opened.clone());
        if self.suggestions_work.opened == opened && self.snapshot.suggestions.is_some() {
            return false;
        }
        if self.suggestions_work.opened != opened {
            self.suggestions_work.read = None;
            self.send_suggestions(match found {
                Some((opened, project)) => Command::Open {
                    workspace_id: opened.workspace_id,
                    project,
                },
                None => Command::Close,
            });
        }
        self.suggestions_work.opened = opened;
        self.compose_suggestions()
    }

    /// A catalog Project's folder, the node that reads it and its link.
    fn suggestion_project(&self, workspace_id: &str) -> Option<(OpenedProject, ProjectRef)> {
        let workspace = self
            .catalog_workspaces()
            .find(|workspace| workspace.id == workspace_id)?;
        let node = match self.linked_node(&workspace.device_id) {
            Ok((node, _)) => node,
            Err("link_down") => crate::node::NodeId::parse(&workspace.device_id).ok()?,
            Err(_) => return None,
        };
        Some((
            OpenedProject {
                workspace_id: workspace_id.to_owned(),
                node: node.clone(),
                root: workspace.path.clone(),
            },
            ProjectRef {
                node,
                path: workspace.path.clone(),
            },
        ))
    }

    pub(super) fn send_suggestions(&self, command: Command) {
        if let Some(client) = self.suggestions_work.client.as_ref() {
            client.send(command);
        }
    }

    /// The Project a screen's event names, or a diagnostic saying why it
    /// names none.
    fn suggestion_target(&self, workspace_id: &str, event: &str) -> Option<ProjectRef> {
        let found = self.suggestion_project(workspace_id);
        if found.is_none() {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "event.project_unavailable",
                "event": event, "workspace_id": workspace_id,
            }));
        }
        found.map(|(_, project)| project)
    }

    pub(super) fn apply_suggestion_action(&mut self, payload: SuggestionActionPayload) -> bool {
        let Some(project) = self.suggestion_target(&payload.workspace_id, "suggestion_action")
        else {
            return false;
        };
        if let Some(number) = super::suggestion_memory::card_of_row(&payload.id) {
            let action = match payload.action {
                SuggestionActionKind::Apply | SuggestionActionKind::Retry => MemoryAction::Promote,
                SuggestionActionKind::Dismiss => MemoryAction::Dismiss,
                SuggestionActionKind::Restore => {
                    crate::diagnostic!(json!({
                        "component": "suggestions", "kind": "action.unsupported",
                        "workspace_id": payload.workspace_id, "id": payload.id,
                    }));
                    return false;
                }
            };
            return self.apply_memory_row_action(payload.workspace_id, number, action);
        }
        let action = match payload.action {
            SuggestionActionKind::Dismiss => LineAction::Dismiss,
            SuggestionActionKind::Restore => LineAction::Restore,
            SuggestionActionKind::Apply | SuggestionActionKind::Retry => {
                crate::diagnostic!(json!({
                    "component": "suggestions", "kind": "action.unsupported",
                    "workspace_id": payload.workspace_id, "id": payload.id,
                }));
                return false;
            }
        };
        let Some(line) = line_of_row(&payload.id) else {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "action.unknown_row",
                "workspace_id": payload.workspace_id, "id": payload.id,
            }));
            return false;
        };
        self.send_suggestions(Command::Action {
            project,
            line,
            action,
        });
        false
    }

    pub(super) fn apply_suggestion_settings(&mut self, payload: SuggestionSettingsPayload) -> bool {
        let SuggestionSettingsPayload {
            workspace_id,
            suggestions_enabled,
            quantity,
            apply_mode,
            signal,
        } = payload;
        let changes = [
            suggestions_enabled.map(SettingsChange::Suggestions),
            quantity.map(SettingsChange::Quantity),
            apply_mode.map(SettingsChange::ApplyMode),
            signal.map(|switch| SettingsChange::Signal(switch.signal, switch.enabled)),
        ];
        let mut named = changes.into_iter().flatten();
        let (Some(change), None) = (named.next(), named.next()) else {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "settings.refused",
                "workspace_id": workspace_id, "reason": "one_setting_per_event",
            }));
            return false;
        };
        let Some(project) = self.suggestion_target(&workspace_id, "suggestion_settings") else {
            return false;
        };
        self.send_suggestions(Command::Settings { project, change });
        false
    }

    /// The head switch: off turns this Project's suggestions and its Memory
    /// reading off; on gives each back what it had (B3). Memory keeps its own
    /// consent, so Memory that was off before stays off.
    pub(super) fn apply_hide_ai_switch(&mut self, payload: HideAiSwitchPayload) -> bool {
        let Some(project) = self.suggestion_target(&payload.workspace_id, "hide_ai_switch") else {
            return false;
        };
        if payload.on {
            self.send_suggestions(Command::Settings {
                project,
                change: SettingsChange::HideAiOn {
                    workspace_id: payload.workspace_id,
                },
            });
            return false;
        }
        let memory_on = self.memory_shown_on(&payload.workspace_id);
        self.send_suggestions(Command::Settings {
            project,
            change: SettingsChange::HideAiOff { memory_on },
        });
        if memory_on {
            return self.set_memory_enabled(payload.workspace_id, false);
        }
        false
    }

    /// Whether the Memory tab shows this Project's Memory on; the head switch
    /// is pressed from that same tool, so it is the Project in hand.
    fn memory_shown_on(&self, workspace_id: &str) -> bool {
        self.snapshot
            .memory
            .as_ref()
            .and_then(|memory| memory.project.as_ref())
            .is_some_and(|memory| memory.workspace_id == workspace_id && memory.enabled)
    }

    fn set_memory_enabled(&mut self, workspace_id: String, on: bool) -> bool {
        self.apply_memory_action(events::MemoryActionPayload {
            numbers: Vec::new(),
            workspace_id,
            action: if on {
                crate::model::MemoryAction::Enable
            } else {
                crate::model::MemoryAction::Disable
            },
            number: None,
        })
    }

    /// Hands the worker what GitHub's reads say went wrong in each Project's
    /// pull requests (B4, D-19): each check an open one failed, and each
    /// that merged after a review asked for changes. Only what was not sent
    /// before leaves the lock.
    pub(super) fn feed_suggestion_prs(&mut self) {
        use crate::model::ReviewDecision;
        use crate::suggestions::ledger::{Observation, PrRef};
        let Some(client) = self.suggestions_work.client.clone() else {
            return;
        };
        let now = unix_milliseconds();
        let work = &mut self.suggestions_work;
        let nodes = std::iter::once((self.node.clone(), &self.github)).chain(
            self.device_github.iter().filter_map(|(device, github)| {
                crate::node::NodeId::parse(device)
                    .ok()
                    .map(|node| (node, github))
            }),
        );
        let mut batches = Vec::new();
        for (node, github) in nodes {
            for project in &github.projects {
                let mut facts = Vec::new();
                for pr in &project.pull_requests {
                    let unit = format!(
                        "{}\u{1f}{}\u{1f}{}",
                        node.as_str(),
                        project.root_path,
                        pr.number
                    );
                    let fact = |signal: Signal, tag: String, at: u64| Observation {
                        signal,
                        tag,
                        session: None,
                        pr: Some(PrRef {
                            number: pr.number,
                            url: pr.url.clone(),
                        }),
                        checkout: None,
                        at_unix_ms: at,
                        always: false,
                    };
                    let open = pr.merged_at_unix_ms.is_none() && pr.closed_at_unix_ms.is_none();
                    if open {
                        for check in &pr.work.failed_checks {
                            let tag = check.name.clone().unwrap_or_else(|| "check".to_owned());
                            if remember(&mut work.sent_prs, format!("{unit}\u{1f}c\u{1f}{tag}")) {
                                facts.push(fact(Signal::CheckFailed, tag, now));
                            }
                        }
                        if pr.review == Some(ReviewDecision::ChangesRequested) {
                            remember(&mut work.changes_requested, unit);
                        }
                    } else if let Some(merged) = pr.merged_at_unix_ms
                        && work.changes_requested.remove(&unit)
                    {
                        facts.push(fact(
                            Signal::ChangeRequest,
                            "changes_requested".to_owned(),
                            merged,
                        ));
                    }
                }
                if !facts.is_empty() {
                    batches.push((
                        ProjectRef {
                            node: node.clone(),
                            path: project.root_path.clone(),
                        },
                        facts,
                    ));
                }
            }
        }
        for (project, facts) in batches {
            client.send(Command::Observe { project, facts });
        }
    }

    pub(super) fn note_suggestions_intro_seen(&mut self) -> bool {
        self.send_suggestions(Command::IntroSeen);
        false
    }

    pub(super) fn note_hide_ai_wake(&mut self, payload: HideAiWakePayload) -> bool {
        crate::diagnostic!(json!({
            "component": "suggestions", "kind": "wake", "cause": payload.cause.as_str(),
        }));
        self.send_suggestions(Command::Wake);
        false
    }

    /// The worker's read of the opened Project.
    pub(super) fn ingest_suggestions_read(
        &mut self,
        workspace_id: String,
        read: ProjectSuggestionsSnapshot,
    ) -> bool {
        if self
            .suggestions_work
            .opened
            .as_ref()
            .is_none_or(|opened| opened.workspace_id != workspace_id)
        {
            return false;
        }
        self.suggestions_work.read = Some(Arc::new(read));
        self.compose_suggestions()
    }

    /// Whether the Hide AI agent can investigate (B19, D-39).
    fn suggestions_ai(&self) -> SuggestionsAi {
        match self.ai_settings.as_ref() {
            Some(settings) if settings.enabled && settings.chosen => {
                if settings.provider == hide_ai::ProviderId::OPENCODE {
                    SuggestionsAi::CannotInvestigate
                } else {
                    SuggestionsAi::Ready
                }
            }
            _ => SuggestionsAi::NotReady,
        }
    }

    /// What the worker asks before a run that uses the provider (D-21, D-36).
    fn suggestions_gate(&self) -> RunGate {
        RunGate {
            ai_ready: self.suggestions_ai() == SuggestionsAi::Ready,
            usage_ok: !self.usage_paused(),
            language: self.output_language(),
        }
    }

    /// Whether GitHub answers for the opened Project (D-41): a missing `gh`,
    /// a signed-out one or no GitHub remote. A network failure keeps the
    /// last answer.
    /// Whether GitHub answers for the Project at `root` on `node` (D-41).
    pub(super) fn suggestions_github(
        &self,
        node: &crate::node::NodeId,
        root: &str,
    ) -> SuggestionsGithub {
        let catalog = if *node == self.node {
            self.github.project(root)
        } else {
            self.device_github
                .get(node.as_str())
                .and_then(|github| github.project(root))
        };
        match catalog.and_then(|project| project.status.failure_category) {
            Some(
                GithubFailureCategory::NotInstalled
                | GithubFailureCategory::NotLoggedIn
                | GithubFailureCategory::NoGithubRemote,
            ) => SuggestionsGithub::Missing,
            Some(GithubFailureCategory::NetworkOrRateLimit) | None => SuggestionsGithub::Connected,
        }
    }

    fn suggestion_facts(&self) -> Option<LockFacts> {
        let opened = self.suggestions_work.opened.as_ref()?;
        Some(LockFacts {
            ai: self.suggestions_ai(),
            github: self.suggestions_github(&opened.node, &opened.root),
        })
    }

    /// Composes the section again when Hide AI's readiness or GitHub's
    /// answer moved under it; a delta that moved neither costs two reads.
    pub(super) fn refresh_suggestions(&mut self) {
        if self.suggestion_facts() != self.suggestions_work.laid
            || self.opened_memory_facts() != self.suggestions_work.laid_memory
        {
            self.compose_suggestions();
        }
    }

    fn opened_memory_facts(&mut self) -> Option<super::suggestion_memory::MemoryFacts> {
        let workspace_id = self.suggestions_work.opened.as_ref()?.workspace_id.clone();
        self.suggestion_memory_facts(&workspace_id)
    }

    /// Lays the lock's facts on the worker's last read and publishes the
    /// section as a new `Arc` only when it changed.
    pub(super) fn compose_suggestions(&mut self) -> bool {
        let facts = self.suggestion_facts();
        self.suggestions_work.laid = facts;
        let memory = self.opened_memory_facts();
        self.suggestions_work.laid_memory = memory.clone();
        let memory_on = self
            .suggestions_work
            .opened
            .as_ref()
            .is_some_and(|opened| self.memory_shown_on(&opened.workspace_id));
        let project = self
            .suggestions_work
            .opened
            .as_ref()
            .zip(facts)
            .map(|(opened, facts)| {
                let mut project = self
                    .suggestions_work
                    .read
                    .as_deref()
                    .filter(|read| read.workspace_id == opened.workspace_id)
                    .cloned()
                    .unwrap_or_else(|| ProjectSuggestionsSnapshot {
                        workspace_id: opened.workspace_id.clone(),
                        loading: true,
                        ..ProjectSuggestionsSnapshot::default()
                    });
                if let Some(memory) = &memory {
                    super::suggestion_memory::lay_memory_rows(&mut project, memory);
                }
                lay_facts(&mut project, facts, memory_on);
                project.counts = self.suggestion_counts(&project);
                project
            });
        let next = SuggestionsSnapshot { project };
        if self
            .snapshot
            .suggestions
            .as_deref()
            .is_some_and(|current| *current == next)
        {
            return false;
        }
        self.snapshot.suggestions = Some(Arc::new(next));
        true
    }

    /// The tab's number and the rail's (D-01): the rows that wait on the
    /// operator here, plus the Memory tab's own, and none at all while the
    /// head switch is off (B3).
    fn suggestion_counts(&self, project: &ProjectSuggestionsSnapshot) -> SuggestionCountsSnapshot {
        if !project.enabled {
            return SuggestionCountsSnapshot::default();
        }
        let suggestions = project
            .rows
            .iter()
            .filter(|row| {
                row.group != SuggestionGroup::Applied
                    && matches!(row.state, SuggestionState::New | SuggestionState::MyTurn)
            })
            .count();
        let memory = self
            .snapshot
            .memory
            .as_ref()
            .and_then(|memory| memory.project.as_ref())
            .filter(|memory| memory.enabled && memory.workspace_id == project.workspace_id)
            .map_or(0, |memory| {
                memory
                    .questions
                    .iter()
                    .filter(|question| question.memory_tab_owns())
                    .count()
            });
        SuggestionCountsSnapshot {
            suggestions,
            rail: suggestions + memory,
        }
    }
}

/// The lock's facts on a read: the AI chip, GitHub's answer, the switches
/// that cannot record here and the `GitHub 연결 필요` hold (B5, B18, B19).
fn lay_facts(project: &mut ProjectSuggestionsSnapshot, facts: LockFacts, memory_on: bool) {
    project.ai = facts.ai;
    project.github = facts.github;
    let github_missing = facts.github == SuggestionsGithub::Missing;
    for signal in &mut project.settings.signals {
        signal.unavailable = if signal.signal.needs_github() && github_missing {
            Some(SignalUnavailable::NoGithub)
        } else if signal.signal == Signal::Correction && !memory_on {
            Some(SignalUnavailable::MemoryOff)
        } else {
            None
        };
    }
    if github_missing {
        for row in &mut project.rows {
            if row.state == SuggestionState::New && row.outcome == Outcome::Pr {
                row.hold = Some(SuggestionHold::GithubNeeded);
            }
        }
    }
}

/// Adds `key` to a bounded set; whether it was new.
fn remember(set: &mut HashSet<String>, key: String) -> bool {
    if set.contains(&key) {
        return false;
    }
    if set.len() >= PR_KEYS {
        set.clear();
    }
    set.insert(key)
}

/// Hands the worker's reads to the runtime and answers its questions under
/// a short lock; a runtime that is gone answers nothing.
struct RuntimeSink {
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
}

impl RuntimeSink {
    fn apply(&self, change: impl FnOnce(&mut Runtime) -> bool) {
        let Some(runtime) = self.runtime.upgrade() else {
            return;
        };
        let changed = match runtime.lock() {
            Ok(mut guard) => change(&mut guard),
            Err(_) => return,
        };
        drop(runtime);
        if changed {
            self.notifier.notify();
        }
    }
}

impl Sink for RuntimeSink {
    fn read(&self, workspace_id: String, read: ProjectSuggestionsSnapshot) {
        self.apply(move |runtime| runtime.ingest_suggestions_read(workspace_id, read));
    }

    fn gate(&self) -> RunGate {
        let closed = RunGate {
            ai_ready: false,
            usage_ok: false,
            language: hide_ai::OutputLanguage::English,
        };
        let Some(runtime) = self.runtime.upgrade() else {
            return closed;
        };
        runtime
            .lock()
            .map_or(closed, |guard| guard.suggestions_gate())
    }

    fn restore_memory(&self, workspace_id: String) {
        self.apply(move |runtime| runtime.set_memory_enabled(workspace_id, true));
    }

    fn link(&self, node: &crate::node::NodeId) -> Option<Arc<dyn crate::node_access::NodeLink>> {
        let runtime = self.runtime.upgrade()?;
        let guard = runtime.lock().ok()?;
        guard.linked_node(node.as_str()).ok().map(|(_, link)| link)
    }

    fn apply(&self, order: crate::suggestions::apply::ApplyOrder) {
        self.apply(move |runtime| runtime.start_suggestion_apply(order));
    }

    fn followed(&self, attempts: Vec<crate::suggestions::apply::FollowedAttempt>) {
        self.apply(move |runtime| runtime.follow_suggestion_attempts(attempts));
    }

    fn can_apply(&self, location: &ProjectLocation, _outcome: Outcome) -> bool {
        let Ok(node) = crate::node::NodeId::parse(&location.node) else {
            return false;
        };
        let Some(runtime) = self.runtime.upgrade() else {
            return false;
        };
        runtime.lock().is_ok_and(|guard| {
            guard.suggestions_github(&node, &location.path) == SuggestionsGithub::Connected
        })
    }
}

/// Starts the suggestion worker on the state folder's store and hands the
/// runtime its client.
pub(crate) fn spawn_worker(
    runtime: &Arc<Mutex<Runtime>>,
    notifier: ChangeNotifier,
) -> Result<SuggestionWorker, String> {
    let state_path = runtime
        .lock()
        .map_err(|_| "runtime lock poisoned")?
        .state_path
        .clone();
    let directory = state_path
        .parent()
        .filter(|directory| !directory.as_os_str().is_empty())
        .ok_or("the state folder is unknown")?;
    let worker = SuggestionWorker::spawn(
        Options::production(hide_kit::layout::suggestions_store(directory)),
        RuntimeSink {
            runtime: Arc::downgrade(runtime),
            notifier,
        },
    )?;
    let mut guard = runtime.lock().map_err(|_| "runtime lock poisoned")?;
    guard.suggestions_work.client = Some(worker.client());
    // What the GitHub reads restored at start already say.
    guard.feed_suggestion_prs();
    guard.track_suggestion_attempts();
    // A Project the tool opened before the worker was there is opened now.
    if let Some(opened) = guard.suggestions_work.opened.clone()
        && let Some((_, project)) = guard.suggestion_project(&opened.workspace_id)
    {
        guard.send_suggestions(Command::Open {
            workspace_id: opened.workspace_id,
            project,
        });
    }
    Ok(worker)
}
