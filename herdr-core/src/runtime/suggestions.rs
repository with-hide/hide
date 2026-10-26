//! The Hide AI tool's 제안 tab as the runtime holds it (PRD
//! hide-ai-suggestions): which Project the tool opened, the suggestion
//! worker's last read of it, and the section composed from that read and
//! the facts only the lock has (the Memory tab's own count, Hide AI's
//! readiness, GitHub's answer). Reading and writing the suggestion store
//! happens on the worker; the lock opens, composes and hands commands over.

use super::*;
use crate::model::{
    ProjectSuggestionsSnapshot, SuggestionCountsSnapshot, SuggestionGroup, SuggestionState,
    SuggestionsSnapshot,
};
use std::sync::Arc;

#[derive(Default)]
pub(super) struct SuggestionsWork {
    /// The catalog Project the Hide AI tool opened, with the machine that
    /// holds its folder.
    pub(super) opened: Option<OpenedProject>,
    /// The worker's last read of the opened Project, before the lock's
    /// facts are laid on it.
    pub(super) read: Option<Arc<ProjectSuggestionsSnapshot>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct OpenedProject {
    pub(super) workspace_id: String,
    pub(super) node: crate::node::NodeId,
    pub(super) root: String,
}

impl Runtime {
    /// Follows the Hide AI tool's `memory_open`: the 제안 tab opens and closes
    /// with the Memory tab, since they are one tool (D-01).
    pub(super) fn open_suggestions(&mut self, workspace_id: Option<String>) -> bool {
        let opened = workspace_id.and_then(|workspace_id| {
            let workspace = self
                .catalog_workspaces()
                .find(|workspace| workspace.id == workspace_id)?;
            let node = match self.linked_node(&workspace.device_id) {
                Ok((node, _)) => Some(node),
                Err(_) => crate::node::NodeId::parse(&workspace.device_id).ok(),
            }?;
            Some(OpenedProject {
                workspace_id,
                node,
                root: workspace.path.clone(),
            })
        });
        if self.suggestions_work.opened == opened && self.snapshot.suggestions.is_some() {
            return false;
        }
        if self.suggestions_work.opened != opened {
            self.suggestions_work.read = None;
        }
        self.suggestions_work.opened = opened;
        self.compose_suggestions()
    }

    /// Lays the lock's facts on the worker's last read and publishes the
    /// section as a new `Arc` only when it changed.
    pub(super) fn compose_suggestions(&mut self) -> bool {
        let project = self.suggestions_work.opened.as_ref().map(|opened| {
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
