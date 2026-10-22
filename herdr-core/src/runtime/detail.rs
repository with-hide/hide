//! The sidebar's lens and 태스크 view option, and the lane's 보기 slot (PRD
//! sidebar-task-lens D-10, D-11, D-25, D-48, D-50): one window-level UI value
//! each, kept in `core-state.json` with the rest of the UI state. The slot
//! holds a pull request's card or a session's record, belongs to no checkout
//! and no Agent tab, and starts the read of what it shows on the event that
//! opens it, so one operator action is one event.

use super::*;
use crate::model::{DetailItem, SidebarLens, TaskView};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SidebarLensPayload {
    /// `projects` or `tasks`.
    pub(super) lens: String,
    /// Shows the sidebar in the same event: ⇧⌘B with the sidebar hidden
    /// shows it on the other lens (B2, D-47).
    #[serde(default)]
    pub(super) show: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TaskViewPayload {
    /// `status` or `project`.
    pub(super) view: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DetailOpenPayload {
    pub(super) item: DetailItem,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DetailShowPayload {
    pub(super) shown: bool,
}

/// A task's row asked for from outside the list (B40): a pane head's
/// agent-tree button or a started backlog item.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TaskRevealPayload {
    pub(super) pane_id: String,
    /// The agent's session, whose record opens in the 보기 slot; absent for
    /// an agent with no session yet, which only shows its row.
    #[serde(default)]
    pub(super) session: Option<crate::links::ledger::SessionIdentity>,
}

impl Runtime {
    pub(super) fn set_sidebar_lens(&mut self, payload: SidebarLensPayload) -> bool {
        let Some(lens) = SidebarLens::parse(&payload.lens) else {
            self.set_error(
                "sidebar_lens.unknown",
                format!(
                    "{} is not a sidebar lens; expected projects or tasks",
                    payload.lens
                ),
                false,
            );
            return true;
        };
        let state = &mut self.snapshot.ui_state;
        let changed = state.sidebar_lens != lens || (payload.show && !state.left_sidebar_visible);
        if !changed {
            return false;
        }
        state.sidebar_lens = lens;
        if payload.show {
            state.left_sidebar_visible = true;
        }
        self.persist_ui_state();
        true
    }

    pub(super) fn set_task_view(&mut self, payload: TaskViewPayload) -> bool {
        let Some(view) = TaskView::parse(&payload.view) else {
            self.set_error(
                "task_view.unknown",
                format!(
                    "{} is not a 태스크 view; expected status or project",
                    payload.view
                ),
                false,
            );
            return true;
        };
        if self.snapshot.ui_state.task_view == view {
            return false;
        }
        self.snapshot.ui_state.task_view = view;
        self.persist_ui_state();
        true
    }

    /// Opens `item` in the slot, in front, and the front Workspace's lane
    /// opens to draw it in its viewing column (B14, B15, B47; lane-two-
    /// columns B9); the Workspace, its Agent tab and the keyboard stay
    /// (D-43).
    pub(super) fn open_detail(&mut self, item: DetailItem) -> bool {
        let before = self.snapshot.ui_state.detail.clone();
        self.snapshot.ui_state.detail.open(item);
        // Opening the item shown again reads it again, as Look again does.
        self.read_detail_top(before.top(), true);
        if self.snapshot.ui_state.detail != before {
            self.persist_ui_state();
        }
        self.open_front_lane();
        true
    }

    /// ←: the item opened before the one shown, or the slot empties as ✕
    /// empties it (B47).
    pub(super) fn detail_back(&mut self) -> bool {
        if self.snapshot.ui_state.detail.items.is_empty() {
            return false;
        }
        let before = self.snapshot.ui_state.detail.clone();
        self.snapshot.ui_state.detail.back();
        self.read_detail_top(before.top(), false);
        self.persist_ui_state();
        if self.snapshot.ui_state.detail.items.is_empty() {
            self.show_front_documents_if_any();
        }
        true
    }

    /// ✕: the slot empties and the rail drops it; the viewing column shows
    /// the documents when the Workspace has some, else closes (lane-two-
    /// columns B13).
    pub(super) fn close_detail(&mut self) -> bool {
        let before = self.snapshot.ui_state.detail.clone();
        if before == crate::model::DetailSlot::default() {
            return false;
        }
        self.snapshot.ui_state.detail.close();
        if matches!(before.top(), Some(DetailItem::Record { .. })) {
            self.close_session_record();
        }
        self.persist_ui_state();
        self.show_front_documents_if_any();
        true
    }

    /// The rail's 보기 button (B46; lane-two-columns B11): pressed while
    /// the slot is behind, the lane opens and draws it in front; pressed
    /// while it is in front, the viewing column folds, the documents with
    /// it, and the slot stays on the rail to come back to.
    pub(super) fn show_detail(&mut self, shown: bool) -> bool {
        let detail = &mut self.snapshot.ui_state.detail;
        if detail.items.is_empty() {
            return false;
        }
        if detail.shown == shown {
            // The slot is the window's: shown, it may still stand in a
            // checkout whose own lane is closed, and the button opens that
            // lane (B22, B11).
            return shown && self.open_front_lane();
        }
        detail.shown = shown;
        if shown {
            let top = self.snapshot.ui_state.detail.top().cloned();
            self.read_detail_top(top.as_ref(), true);
            self.open_front_lane();
        } else {
            self.fold_front_documents();
        }
        self.persist_ui_state();
        true
    }

    /// Opens the front Workspace's lane, so the slot is drawn; its columns
    /// come back as they were (lane-two-columns B15). Whether it was closed.
    fn open_front_lane(&mut self) -> bool {
        let Some(key) = self.front_workspace_key() else {
            return false;
        };
        let Some(store) = self.workspace_views.as_mut() else {
            return false;
        };
        let entry = store.views.entry(&key.0, &key.1);
        if entry.open {
            return false;
        }
        entry.open = true;
        store.call_lane(&key, crate::model::LaneCallSide::View);
        self.persist_workspace_views();
        true
    }

    /// The slot emptied: the documents take the viewing column when the
    /// front Workspace has some, else the column closes (B13).
    fn show_front_documents_if_any(&mut self) {
        let Some(key) = self.front_workspace_key() else {
            return;
        };
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let entry = store.views.entry(&key.0, &key.1);
        let documents = entry.open && entry.layout.display_count() > 0;
        if entry.documents == documents {
            return;
        }
        entry.documents = documents;
        self.persist_workspace_views();
    }

    /// The viewing column folded behind the rail's 보기 button (B11).
    fn fold_front_documents(&mut self) {
        let Some(key) = self.front_workspace_key() else {
            return;
        };
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let entry = store.views.entry(&key.0, &key.1);
        if !entry.documents {
            return;
        }
        entry.documents = false;
        self.persist_workspace_views();
    }

    /// The documents came in front, or the lane closed: the slot stops
    /// being drawn and stays on the rail to come back to (B46, B49;
    /// lane-two-columns B10). Whether anything changed.
    pub(super) fn leave_detail(&mut self) -> bool {
        if !self.snapshot.ui_state.detail.shown {
            return false;
        }
        self.snapshot.ui_state.detail.shown = false;
        self.persist_ui_state();
        true
    }

    /// B40: the sidebar shows the 태스크 list with the row's ancestors
    /// unfolded, and the agent's record opens in the slot, in one event.
    pub(super) fn reveal_task(&mut self, payload: TaskRevealPayload) -> bool {
        let parents: std::collections::HashMap<&str, &str> = self
            .snapshot
            .navigator
            .agents
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| &session.agents),
            )
            .filter_map(|agent| {
                Some((
                    agent.pane_id.as_str(),
                    agent.lineage_parent_pane_id.as_deref()?,
                ))
            })
            .collect();
        let mut ancestors = Vec::new();
        let mut at = payload.pane_id.as_str();
        while let Some(parent) = parents.get(at).copied() {
            if parent == payload.pane_id || ancestors.contains(&parent) {
                break;
            }
            ancestors.push(parent);
            at = parent;
        }
        let ancestors: Vec<String> = ancestors.into_iter().map(str::to_owned).collect();
        let state = &mut self.snapshot.ui_state;
        state.left_sidebar_visible = true;
        state.sidebar_lens = SidebarLens::Tasks;
        for ancestor in ancestors {
            if !state.sessions_expanded_agent_pane_ids.contains(&ancestor) {
                state.sessions_expanded_agent_pane_ids.push(ancestor);
            }
        }
        if let Some(session) = payload.session {
            self.open_detail(DetailItem::Record { session });
        }
        // The lens, the sidebar and the unfolded ancestors are saved even
        // when the record was already in front and the slot did not move.
        self.persist_ui_state();
        true
    }

    /// Starts the read of what the slot shows now when it changed from
    /// `before`, or `again`: a card's body, files and threads, or a record.
    /// A record the slot no longer shows stops being read.
    fn read_detail_top(&mut self, before: Option<&DetailItem>, again: bool) {
        let top = self.snapshot.ui_state.detail.top().cloned();
        if before == top.as_ref() && !again {
            return;
        }
        match top {
            Some(DetailItem::PullRequest {
                workspace_id,
                number,
            }) => {
                if matches!(before, Some(DetailItem::Record { .. })) {
                    self.close_session_record();
                }
                self.read_pr_card(PrCardReadPayload {
                    workspace_id,
                    pr_number: number,
                    look_again: false,
                });
            }
            Some(DetailItem::Record { session }) => {
                self.open_session_record(super::links::RecordOpenPayload { session });
            }
            // A map draws the mission record the snapshot already carries.
            Some(DetailItem::Mission { .. }) | None => {
                if matches!(before, Some(DetailItem::Record { .. })) {
                    self.close_session_record();
                }
            }
        }
    }
}
