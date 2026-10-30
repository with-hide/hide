//! The Hide AI tool's 제안 tab as the runtime holds it (PRD
//! hide-ai-suggestions): which Project the tool opened, the suggestion
//! worker's last read of it, and the section composed from that read and
//! the facts only the lock has (the Memory tab's own count, Hide AI's
//! readiness, GitHub's answer). Reading and writing the suggestion store
//! happens on the worker; the lock opens, composes and hands commands over.

use super::*;
use crate::model::{
    MemoryAction, ProjectSuggestionsSnapshot, SignalUnavailable, SuggestionAttemptPhase,
    SuggestionCountsSnapshot, SuggestionGroup, SuggestionHold, SuggestionState, SuggestionsAi,
    SuggestionsGithub, SuggestionsSnapshot,
};
use crate::suggestions::analysis::{Analysis, TagGrouping};
use crate::suggestions::investigation::node::{Checkout, ProjectReads};
use crate::suggestions::investigation::{Admission, Investigation};
use crate::suggestions::ledger::Signal;
use crate::suggestions::notebook::Outcome;
use crate::suggestions::policy::Quantity;
use crate::suggestions::read::line_of_row;
use crate::suggestions::settings::ApplyMode;
use crate::suggestions::store::ProjectLocation;
use crate::suggestions::worker::{
    AnalysisFactory, Command, LineAction, Options, ProjectRef, RunGate, RunPlan, SettingsChange,
    Sink, SuggestionClient, SuggestionWorker,
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
    /// Each live attempt agent's pane as the agent lists name it, by its
    /// coordination id, when the section was last composed.
    laid_agents: Vec<(String, String)>,
    pub(super) promoting: super::suggestion_memory::PromotingCache,
    /// The attempts applying lines, started and followed here (B11-B14).
    pub(super) apply: super::suggestion_apply::ApplyWork,
    /// The pull request records already handed to the worker, so a GitHub
    /// read that moved nothing for them sends nothing.
    sent_prs: HashSet<String>,
    /// Open pull requests a review asked changes of, until they merge
    /// (B4: a review's change that was fixed and merged).
    changes_requested: HashSet<String>,
    /// The open row's excerpts (PRD memory-self-improve B7).
    pub(super) evidence: super::suggestion_evidence::EvidenceWork,
}

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
    /// Hide AI and its turn-end analysis are on, so improvements are found
    /// (D-57).
    analysis: bool,
}

/// `suggestion_action`: one press on a row (B11, B16).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SuggestionActionPayload {
    workspace_id: String,
    id: String,
    action: SuggestionActionKind,
    /// The global instruction preview the operator pressed on (PRD
    /// memory-self-improve B8).
    #[serde(default)]
    preview: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SuggestionActionKind {
    Apply,
    Dismiss,
    Restore,
    Retry,
    /// Takes a global instruction line back out (B9).
    Revert,
    /// `main에 합치기` on a ready branch (PRD hide-ai-branch-first-apply D-05).
    Merge,
    /// `PR 올리기` on a ready branch (D-06).
    OpenPr,
    /// `버리기` on a ready branch (D-07).
    Discard,
    /// `다시 맞추기` on a ready branch the base moved under (B8, B12).
    Rebase,
}

/// `suggestion_settings`: exactly one of ⚙'s settings; a 읽을 곳 switch is
/// one setting, every signal of its place at once (D-52).
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
    signal_group: Option<SignalGroupSwitch>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignalGroupSwitch {
    group: crate::suggestions::ledger::SignalGroup,
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
        if self.suggestions_work.opened != opened {
            // Another Project's rows share these ids; what was read for the
            // last one says nothing about them.
            self.suggestions_work.evidence.close();
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
        // A new fix waits while five branches wait for a decision (PRD
        // hide-ai-branch-first-apply D-17, B28).
        if matches!(payload.action, SuggestionActionKind::Apply)
            && self.held_row(&payload.workspace_id, &payload.id)
                == Some(SuggestionHold::BranchesFull)
        {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "apply.refused",
                "workspace_id": payload.workspace_id, "id": payload.id,
                "reason": "branches_full",
            }));
            return false;
        }
        // `PR 올리기` is offered only while GitHub is connected and permitted
        // (D-03, B9); an agent asked to push without it would only fail.
        if matches!(payload.action, SuggestionActionKind::OpenPr)
            && self.suggestions_github(&project.node, &project.path) != SuggestionsGithub::Connected
        {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "apply.refused",
                "workspace_id": payload.workspace_id, "id": payload.id,
                "reason": "github_needed",
            }));
            return false;
        }
        if let Some(number) = super::suggestion_memory::card_of_row(&payload.id) {
            let decision = match payload.action {
                SuggestionActionKind::Merge => Some(super::memory_branch::BatchDecision::Merge),
                SuggestionActionKind::OpenPr => Some(super::memory_branch::BatchDecision::OpenPr),
                SuggestionActionKind::Discard => Some(super::memory_branch::BatchDecision::Discard),
                SuggestionActionKind::Rebase => Some(super::memory_branch::BatchDecision::Rebase),
                _ => None,
            };
            if let Some(decision) = decision {
                return self.decide_memory_batch(&payload.workspace_id, number, decision);
            }
            let action = match payload.action {
                SuggestionActionKind::Apply | SuggestionActionKind::Retry => MemoryAction::Promote,
                SuggestionActionKind::Dismiss => MemoryAction::Dismiss,
                SuggestionActionKind::Merge
                | SuggestionActionKind::OpenPr
                | SuggestionActionKind::Discard
                | SuggestionActionKind::Rebase
                | SuggestionActionKind::Restore
                | SuggestionActionKind::Revert => {
                    crate::diagnostic!(json!({
                        "component": "suggestions", "kind": "action.unsupported",
                        "workspace_id": payload.workspace_id, "id": payload.id,
                    }));
                    return false;
                }
            };
            return self.apply_memory_row_action(payload.workspace_id, number, action);
        }
        let decide = |runtime: &mut Self, decision| {
            line_of_row(&payload.id)
                .is_some_and(|line| runtime.decide_suggestion_branch(&project, line.0, decision))
        };
        let action = match payload.action {
            SuggestionActionKind::Merge => {
                return decide(self, super::suggestion_apply::BranchDecision::Merge);
            }
            SuggestionActionKind::Discard => {
                return decide(self, super::suggestion_apply::BranchDecision::Discard);
            }
            SuggestionActionKind::Dismiss => LineAction::Dismiss,
            SuggestionActionKind::Restore => LineAction::Restore,
            SuggestionActionKind::Apply => LineAction::Apply,
            SuggestionActionKind::Retry => LineAction::Retry,
            SuggestionActionKind::Revert => LineAction::Revert,
            SuggestionActionKind::OpenPr => LineAction::OpenPr,
            SuggestionActionKind::Rebase => LineAction::Rebase,
        };
        // A row none of whose words could be read again is not applied
        // (PRD memory-self-improve D-34, B7).
        if matches!(action, LineAction::Apply | LineAction::Retry)
            && self
                .suggestions_work
                .evidence
                .unconfirmed(&payload.workspace_id, &payload.id)
        {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "apply.refused",
                "workspace_id": payload.workspace_id, "id": payload.id,
                "reason": "evidence_unconfirmed",
            }));
            return false;
        }
        let Some(line) = line_of_row(&payload.id) else {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "action.unknown_row",
                "workspace_id": payload.workspace_id, "id": payload.id,
            }));
            return false;
        };
        if matches!(action, LineAction::OpenPr | LineAction::Rebase)
            && self.suggestion_branch_deciding(&project, line.0)
        {
            crate::diagnostic!(json!({
                "component": "suggestions", "kind": "decision.refused",
                "line": line.0, "reason": "deciding",
            }));
            return false;
        }
        self.send_suggestions(Command::Action {
            project,
            line,
            action,
            seen: payload.preview,
        });
        false
    }

    pub(super) fn apply_suggestion_settings(&mut self, payload: SuggestionSettingsPayload) -> bool {
        let SuggestionSettingsPayload {
            workspace_id,
            suggestions_enabled,
            quantity,
            apply_mode,
            signal_group,
        } = payload;
        let changes = [
            suggestions_enabled.map(SettingsChange::Suggestions),
            quantity.map(SettingsChange::Quantity),
            apply_mode.map(SettingsChange::ApplyMode),
            signal_group.map(|switch| SettingsChange::SignalGroup(switch.group, switch.enabled)),
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
            change: None,
        })
    }

    /// Hands the worker what GitHub's reads say went wrong in each Project's
    /// pull requests (B4, D-19): each check an open one failed, again on
    /// each new head it fails on, and each that merged after a review asked
    /// for changes. Only what was not sent before leaves the lock.
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
                        head: pr.head_oid.clone(),
                        improvement: None,
                        said: None,
                    };
                    let open = pr.merged_at_unix_ms.is_none() && pr.closed_at_unix_ms.is_none();
                    if open {
                        let head = pr.head_oid.as_deref().unwrap_or_default();
                        for check in &pr.work.failed_checks {
                            let tag = check.name.clone().unwrap_or_else(|| "check".to_owned());
                            let key = format!("{unit}\u{1f}c\u{1f}{tag}\u{1f}{head}");
                            if remember(&mut work.sent_prs, "sent_prs", key) {
                                facts.push(fact(Signal::CheckFailed, tag, now));
                            }
                        }
                        if pr.review == Some(ReviewDecision::ChangesRequested) {
                            remember(&mut work.changes_requested, "changes_requested", unit);
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
        // The app opening is also what may start a WSL distribution that is
        // off (PRD wsl-device B3); a resume or an unlock is not.
        matches!(payload.cause, WakeCause::AppOpen) && self.wake_devices_for_app()
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
            analysis: self.ai_active()
                && self
                    .ai_settings
                    .as_ref()
                    .is_some_and(|settings| settings.agent_summary),
        })
    }

    /// Composes the section again when Hide AI's readiness or GitHub's
    /// answer moved under it; a delta that moved neither costs two reads.
    pub(super) fn refresh_suggestions(&mut self) {
        if self.suggestion_facts() != self.suggestions_work.laid
            || self.opened_memory_facts() != self.suggestions_work.laid_memory
            || self.attempt_agent_panes() != self.suggestions_work.laid_agents
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
        let branches = self.branch_overlay();
        let agents = self.attempt_agent_panes();
        self.suggestions_work.laid_agents = agents.clone();
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
                lay_branches(&mut project, &branches);
                lay_agents(&mut project, &agents);
                super::suggestion_evidence::lay_excerpts(
                    &mut project,
                    &self.suggestions_work.evidence,
                );
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
        self.follow_branch_diff();
        true
    }

    /// The hold the tab shows on row `id` of the opened Project now.
    fn held_row(&self, workspace_id: &str, id: &str) -> Option<SuggestionHold> {
        self.snapshot
            .suggestions
            .as_ref()?
            .project
            .as_ref()
            .filter(|project| project.workspace_id == workspace_id)?
            .rows
            .iter()
            .find(|row| row.id == id)?
            .hold
    }

    /// Whether the Project at `root` on `node` may start one more document
    /// fix: it is a git repository and fewer than five branches wait for the
    /// operator's decision there (PRD hide-ai-branch-first-apply D-09, D-17).
    pub(super) fn branch_room(&self, node: &crate::node::NodeId, root: &str) -> bool {
        let git = self
            .catalog_workspaces()
            .find(|workspace| workspace.device_id == node.as_str() && workspace.path == root)
            .is_some_and(|workspace| workspace.is_git);
        git && self.branches_waiting(node, root) < super::branch_decision::WAITING_BRANCHES
    }

    /// The branches waiting for the operator's decision in the Project at
    /// `root` on `node`: ready 제안 attempts and ready Memory batches.
    pub(super) fn branches_waiting(&self, node: &crate::node::NodeId, root: &str) -> usize {
        let lines = self
            .suggestions_work
            .apply
            .followed
            .iter()
            .filter(|attempt| {
                attempt.location.node == node.as_str()
                    && attempt.location.path == root
                    && attempt.phase == crate::suggestions::lifecycle::AttemptPhase::Ready
            })
            .count();
        lines + self.memory_branches_waiting(node, root)
    }

    /// What the lock knows about the opened Project's branches: whether it
    /// is a git repository, the ready branches Hide is merging now and where
    /// each attempt branch is checked out (PRD hide-ai-branch-first-apply).
    fn branch_overlay(&self) -> BranchOverlay {
        let Some(opened) = self.suggestions_work.opened.as_ref() else {
            return BranchOverlay::default();
        };
        let git = self
            .catalog_workspaces()
            .find(|workspace| workspace.id == opened.workspace_id)
            .is_some_and(|workspace| workspace.is_git);
        let work = &self.suggestions_work.apply;
        let mut merging = HashSet::new();
        let mut checkouts = HashMap::new();
        for attempt in &work.followed {
            if attempt.location.node != opened.node.as_str() || attempt.location.path != opened.root
            {
                continue;
            }
            if work
                .deciding
                .get(&(attempt.project_id.clone(), attempt.line.0))
                == Some(&super::suggestion_apply::BranchDecision::Merge)
            {
                merging.insert(crate::suggestions::read::row_id(attempt.line));
            }
            if let Some(path) = self.branch_checkout(&opened.node, &opened.root, &attempt.branch) {
                checkouts.insert(attempt.branch.clone(), path);
            }
        }
        for (branch, merge) in self.memory_branch_overlay(&opened.node, &opened.root) {
            if let Some(path) = self.branch_checkout(&opened.node, &opened.root, &branch) {
                checkouts.insert(branch.clone(), path);
            }
            if let Some(row) = merge {
                merging.insert(row);
            }
        }
        BranchOverlay {
            git,
            merging,
            checkouts,
        }
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
/// that cannot record here and the `GitHub 연결 필요` hold (B5, B18, B19,
/// B27).
fn lay_facts(project: &mut ProjectSuggestionsSnapshot, facts: LockFacts, memory_on: bool) {
    project.ai = facts.ai;
    project.github = facts.github;
    let github_missing = facts.github == SuggestionsGithub::Missing;
    for signal in &mut project.settings.signals {
        signal.unavailable = if signal.signal.needs_github() && github_missing {
            Some(SignalUnavailable::NoGithub)
        } else if signal.signal.learned_by_memory() && !memory_on {
            Some(SignalUnavailable::MemoryOff)
        } else if signal.signal == Signal::Improvement && !facts.analysis {
            Some(SignalUnavailable::AnalysisOff)
        } else {
            None
        };
    }
    if github_missing {
        for row in &mut project.rows {
            // A document fix is committed on a branch and a global
            // instruction line is written here, both with no GitHub (PRD
            // hide-ai-branch-first-apply D-03, memory-self-improve B8): only
            // an issue needs it.
            if row.state == SuggestionState::New && row.outcome == Outcome::Issue {
                row.hold = Some(SuggestionHold::GithubNeeded);
            }
        }
    }
}

/// What the lock adds to the rows about their branches (PRD
/// hide-ai-branch-first-apply).
#[derive(Debug, Default)]
struct BranchOverlay {
    /// The Project is a git repository: a folder that is not one gets no
    /// document fix (D-09, B16).
    git: bool,
    /// Rows whose ready branch Hide is merging now.
    merging: HashSet<String>,
    /// Where each attempt branch is checked out, which a branch card's file
    /// opens.
    checkouts: HashMap<String, String>,
}

/// Lays the branch facts on the rows: no document fix in a folder that is
/// not a git repository (B16), a new fix held while five branches wait for
/// a decision (D-17, B28), `합치는 중` on a branch being merged (B6), and
/// the checkout each branch is in.
fn lay_branches(project: &mut ProjectSuggestionsSnapshot, overlay: &BranchOverlay) {
    if !overlay.git {
        project
            .rows
            .retain(|row| !(row.state == SuggestionState::New && row.outcome == Outcome::Pr));
    }
    let waiting = project
        .rows
        .iter()
        .filter(|row| {
            row.attempt
                .as_ref()
                .is_some_and(|attempt| attempt.phase == SuggestionAttemptPhase::Ready)
        })
        .count();
    for row in &mut project.rows {
        if row.state == SuggestionState::New
            && row.outcome == Outcome::Pr
            && row.hold.is_none()
            && waiting >= super::branch_decision::WAITING_BRANCHES
        {
            row.hold = Some(SuggestionHold::BranchesFull);
        }
        let Some(attempt) = row.attempt.as_mut() else {
            continue;
        };
        if attempt.phase == SuggestionAttemptPhase::Ready && overlay.merging.contains(&row.id) {
            attempt.phase = SuggestionAttemptPhase::Merging;
        }
        if let Some(branch) = attempt.branch.as_mut() {
            branch.checkout = overlay.checkouts.get(&branch.name).cloned();
        }
    }
}

/// Names each notebook attempt's live agent by the pane the agent lists
/// show it in, so `에이전트 ↗` and the agent's question reach it; a Memory
/// row names its pane from its promotion already.
fn lay_agents(project: &mut ProjectSuggestionsSnapshot, agents: &[(String, String)]) {
    for attempt in project.rows.iter_mut().filter_map(|row| row.attempt.as_mut()) {
        if attempt.agent_pane.is_some() {
            continue;
        }
        attempt.agent_pane = attempt.agent_id.as_deref().and_then(|id| {
            agents
                .iter()
                .find(|(agent, _)| agent == id)
                .map(|(_, pane)| pane.clone())
        });
    }
}

/// Adds `key` to a bounded set; whether it was new.
fn remember(set: &mut HashSet<String>, cache: &str, key: String) -> bool {
    if set.contains(&key) {
        return false;
    }
    if set.len() >= crate::suggestions::policy::PR_KEYS {
        set.clear();
        cache_cleared(cache, crate::suggestions::policy::PR_KEYS);
    }
    set.insert(key)
}

/// A suggestions cache crossed its bound and starts over.
pub(super) fn cache_cleared(cache: &str, limit: usize) {
    crate::diagnostic!(json!({
        "component": "suggestions", "kind": "cache.full",
        "cache": cache, "limit": limit,
    }));
}

/// Hands the worker's reads to the runtime and answers its questions under
/// a short lock; a runtime that is gone answers nothing.
struct RuntimeSink {
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    /// The link record, read on the worker's thread for which sessions'
    /// records a press can still open (B10).
    links: std::path::PathBuf,
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

    fn followed(
        &self,
        attempts: Vec<crate::suggestions::apply::FollowedAttempt>,
        watches: Vec<crate::suggestions::watch::WatchedLine>,
    ) {
        self.apply(move |runtime| runtime.follow_suggestion_attempts(attempts, watches));
    }

    fn can_apply(&self, location: &ProjectLocation, outcome: Outcome) -> bool {
        let Ok(node) = crate::node::NodeId::parse(&location.node) else {
            return false;
        };
        let Some(runtime) = self.runtime.upgrade() else {
            return false;
        };
        runtime.lock().is_ok_and(|guard| match outcome {
            // A document fix needs a git repository and room for one more
            // waiting branch (PRD hide-ai-branch-first-apply D-09, D-17).
            Outcome::Pr => guard.branch_room(&node, &location.path),
            Outcome::Issue => {
                guard.suggestions_github(&node, &location.path) == SuggestionsGithub::Connected
            }
            Outcome::Local => false,
        })
    }

    fn kept_sessions(
        &self,
        node: &str,
        sessions: &std::collections::BTreeSet<crate::suggestions::ledger::SessionRef>,
    ) -> Option<std::collections::BTreeSet<crate::suggestions::ledger::SessionRef>> {
        let asked = sessions
            .iter()
            .map(|session| crate::links::ledger::SessionIdentity {
                device_id: node.to_owned(),
                agent: session.agent.clone(),
                session_id: session.id.clone(),
            })
            .collect::<Vec<_>>();
        let held = crate::links::store::LinkStore::open_read_only(&self.links)
            .and_then(|store| store.held(&asked));
        match held {
            Ok(held) => Some(
                held.into_iter()
                    .map(|session| crate::suggestions::ledger::SessionRef {
                        agent: session.agent,
                        id: session.session_id,
                    })
                    .collect(),
            ),
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": "suggestions", "kind": "evidence.unread",
                    "node": node, "code": code,
                }));
                None
            }
        }
    }
}

/// What a run's investigation is made of, read under the lock: the
/// operator's Hide AI and the folders the Project may be read in.
struct InvestigationParts {
    settings: hide_ai::AiSettings,
    machines: crate::ai::AiMachines,
    gate: Arc<hide_ai::UsageLimits>,
    usage: Arc<dyn hide_ai::AiUsageSink>,
    memory: PathBuf,
    checkouts: Vec<Checkout>,
}

impl Runtime {
    /// `None` while Hide AI cannot investigate (B19, D-39). A catalog
    /// Project on a device whose link is down is not a candidate.
    fn investigation_parts(&self) -> Option<InvestigationParts> {
        if self.suggestions_ai() != SuggestionsAi::Ready {
            return None;
        }
        let checkouts = self
            .catalog_workspaces()
            .filter_map(|workspace| {
                let (node, link) = self.linked_node(&workspace.device_id).ok()?;
                let machine = (node != self.node).then_some(node.as_str());
                Some(Checkout {
                    github: self.github_place(machine, &workspace.path).ok(),
                    root: workspace.path.clone(),
                    node,
                    link,
                })
            })
            .collect();
        Some(InvestigationParts {
            settings: self.ai_settings.clone()?,
            machines: self.ai_machines(),
            gate: self.usage_gate(),
            usage: self.ai_usage(),
            memory: self.memory_database_path(),
            checkouts,
        })
    }
}

/// The analysis a day's run uses (D-49): the investigation when the gate
/// says Hide AI is ready, the deterministic grouping otherwise. Its parts
/// are read under the lock on the worker's thread; the router is made and
/// the Project's folder is found outside it, the folder on the run's own
/// thread. Each turn is counted as one of Memory's Hide AI calls against
/// the machine's daily cap, in Memory's store.
pub(super) fn analysis_factory(runtime: Weak<Mutex<Runtime>>) -> AnalysisFactory {
    Arc::new(move |plan: &RunPlan<'_>| {
        let parts = plan
            .gate
            .ai_ready
            .then(|| runtime.upgrade())
            .flatten()
            .and_then(|runtime| runtime.lock().ok()?.investigation_parts());
        let Some(parts) = parts else {
            return Box::new(TagGrouping) as Box<dyn Analysis>;
        };
        let router = Arc::new(crate::ai::memory_router(
            &parts.machines,
            &parts.settings,
            &parts.gate,
            &parts.usage,
        ));
        let reads = ProjectReads::new(plan.project_id.to_owned(), parts.checkouts);
        Box::new(Investigation::new(
            router,
            Box::new(reads),
            memory_admission(parts.memory),
            plan.stop.token(),
        ))
    })
}

/// Admits each turn against Memory's daily call cap; the store is opened on
/// the first turn, on the run's thread.
fn memory_admission(database: PathBuf) -> Admission {
    let mut store: Option<hide_memory::MemoryStore> = None;
    Box::new(move || {
        let opened = match store.take() {
            Some(opened) => opened,
            None => super::memory::open_memory_store(&database)
                .map_err(|_| "memory_store_unavailable".to_owned())?,
        };
        let admitted =
            super::memory_analysis::admit_call(&opened).map_err(|failure| match failure {
                super::memory_analysis::AnalysisFailure::Deferred(cap) => cap.to_owned(),
                _ => "memory_store_unavailable".to_owned(),
            });
        store = Some(opened);
        admitted
    })
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
        Options::production(
            hide_kit::layout::suggestions_store(directory),
            analysis_factory(Arc::downgrade(runtime)),
        ),
        RuntimeSink {
            runtime: Arc::downgrade(runtime),
            notifier,
            links: hide_kit::layout::links_store(directory),
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
