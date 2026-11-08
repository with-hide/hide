//! The sidebar's lens, 태스크 view option and project, and the lane's 보기
//! slot (PRD sidebar-task-lens D-10, D-11, D-25, D-48, D-50): one
//! UI value each (the slot one per Agent tab), kept in `core-state.json` with
//! the rest of the UI state. The slot
//! holds a pull request's card or a session's record, belongs to the Agent
//! tab that was in front when it was opened (`ui_state.detail_home`), and
//! starts the read of what it shows on the event that opens it, so one
//! operator action is one event. `ui_state.detail` is always the front tab's
//! slot: another tab coming in front swaps its own in (`sync_detail_home`),
//! so every reader and the wire see one slot.

use super::*;
use crate::model::{
    DetailItem, PRACTICE_CHAPTER_LIMIT, PRACTICE_STEP_LIMIT, PracticeProgress, SidebarLens,
    SidebarProject, TaskView, practice_chapter_id_valid,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SidebarLensPayload {
    /// `projects`, `tasks` or `prs`.
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

/// The project the sidebar's 태스크 and PR lists show, or every project for
/// none. A 현황 tile turns the sidebar to its list in the same event.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SidebarProjectPayload {
    pub(super) project: Option<SidebarProject>,
    #[serde(default)]
    pub(super) lens: Option<String>,
    #[serde(default)]
    pub(super) show: bool,
}

/// One practice-mode chapter's progress (`ui_state.practice`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PracticeProgressPayload {
    pub(super) chapter: String,
    pub(super) version: u32,
    pub(super) step: u32,
    pub(super) finished: bool,
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
                    "{} is not a sidebar lens; expected projects, tasks or prs",
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
                    "{} is not a task view; expected status or project",
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

    pub(super) fn set_sidebar_project(&mut self, payload: SidebarProjectPayload) -> bool {
        let lens = match payload.lens.as_deref() {
            None => None,
            Some(value) => match SidebarLens::parse(value) {
                Some(lens) => Some(lens),
                None => {
                    self.set_error(
                        "sidebar_lens.unknown",
                        format!("{value} is not a sidebar lens; expected projects, tasks or prs"),
                        false,
                    );
                    return true;
                }
            },
        };
        let state = &mut self.snapshot.ui_state;
        let changed = state.sidebar_project != payload.project
            || lens.is_some_and(|lens| state.sidebar_lens != lens)
            || (payload.show && !state.left_sidebar_visible);
        if !changed {
            return false;
        }
        state.sidebar_project = payload.project;
        if let Some(lens) = lens {
            state.sidebar_lens = lens;
        }
        if payload.show {
            state.left_sidebar_visible = true;
        }
        self.persist_ui_state();
        true
    }

    pub(super) fn set_practice_progress(&mut self, payload: PracticeProgressPayload) -> bool {
        if !practice_chapter_id_valid(&payload.chapter) {
            self.set_error(
                "practice.chapter_invalid",
                "A practice chapter id is 1 to 32 characters of a-z, 0-9 and -",
                false,
            );
            return true;
        }
        if payload.step > PRACTICE_STEP_LIMIT {
            self.set_error(
                "practice.step_invalid",
                format!("A practice step is at most {PRACTICE_STEP_LIMIT}"),
                false,
            );
            return true;
        }
        let progress = PracticeProgress {
            version: payload.version,
            step: payload.step,
            finished: payload.finished,
        };
        let practice = &mut self.snapshot.ui_state.practice;
        if practice.get(&payload.chapter) == Some(&progress) {
            return false;
        }
        if !practice.contains_key(&payload.chapter) && practice.len() >= PRACTICE_CHAPTER_LIMIT {
            self.set_error(
                "practice.too_many",
                format!("Progress is kept for at most {PRACTICE_CHAPTER_LIMIT} practice chapters"),
                false,
            );
            return true;
        }
        let before = practice.insert(payload.chapter.clone(), progress);
        if let (Some(sink), Some(chapter)) = (
            &self.product_analytics,
            crate::product_analytics::PracticeChapter::of(&payload.chapter),
        ) {
            use crate::product_analytics::{Event, PracticeStage};
            // Each stage counts once per installation: a replayed chapter
            // keeps its progress, and a finished one stays finished.
            if before.is_none() {
                sink.capture(Event::Practice {
                    chapter,
                    stage: PracticeStage::Started,
                });
            }
            if progress.finished && !before.is_some_and(|before| before.finished) {
                sink.capture(Event::Practice {
                    chapter,
                    stage: PracticeStage::Finished,
                });
            }
        }
        self.persist_ui_state();
        true
    }

    /// The sidebar's invitation to practice was closed; it stays closed.
    pub(super) fn close_practice_invite(&mut self) -> bool {
        let state = &mut self.snapshot.ui_state;
        if state.practice_invite_closed {
            return false;
        }
        state.practice_invite_closed = true;
        self.persist_ui_state();
        true
    }

    /// Opens `item` in the slot, in front, and the front Workspace's lane
    /// opens to draw it in its viewing column (B14, B15, B47; lane-two-
    /// columns B9); the Workspace, its Agent tab and the keyboard stay
    /// (D-43).
    pub(super) fn open_detail(&mut self, item: DetailItem) -> bool {
        // An event that moved the tab first (a card opened in its own
        // Workspace) opens in the tab it left in front.
        self.sync_detail_home();
        let before = self.snapshot.ui_state.detail.clone();
        self.snapshot.ui_state.detail.open(item);
        // Opening the item shown again reads it again, as Look again does.
        self.read_detail_top(before.top(), true);
        if self.snapshot.ui_state.detail != before {
            self.persist_ui_state();
        }
        self.call_front_lane();
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
        if !self.snapshot.ui_state.detail.items.is_empty() {
            self.call_front_lane();
        }
        true
    }

    /// ✕: the slot empties and the rail drops it; the viewing column goes
    /// back to what stood under the slot, the documents only when they
    /// showed before it came forward, else the column closes (lane-two-
    /// columns B13; operator decision 2026-10-24).
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
            // The slot is the front tab's: shown, it may still stand in a
            // checkout whose own lane is closed, or behind a stacked lane's
            // tool, and the button brings it forward (B22, B11, B19).
            if shown {
                self.call_front_lane();
            }
            return shown;
        }
        detail.shown = shown;
        if shown {
            let top = self.snapshot.ui_state.detail.top().cloned();
            self.read_detail_top(top.as_ref(), true);
            self.call_front_lane();
        } else {
            self.fold_front_documents();
        }
        self.persist_ui_state();
        true
    }

    /// The one way the slot comes forward, whatever opened it (a card, a
    /// record, the map, ← or the rail's button): the front Workspace's lane
    /// opens when it was closed on the slot alone, the tool column closed
    /// (`open_on`) and no documents under the slot, so its ✕ closes the
    /// lane again (operator decision 2026-10-24); the call is numbered as
    /// the viewing column's, so a stacked lane draws the slot over its tool
    /// (B9, B19).
    fn call_front_lane(&mut self) {
        let Some(key) = self.front_workspace_key() else {
            return;
        };
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let entry = store.views.entry(&key.0, &key.1);
        let was_closed = entry.open_on(crate::model::LaneCallSide::View);
        if was_closed {
            entry.documents = false;
        }
        store.call_lane(&key, crate::model::LaneCallSide::View);
        if was_closed {
            self.persist_workspace_views();
        }
    }

    /// Takes out of every tab's slot the items whose subject is gone (a
    /// deleted session's plan); a slot this empties
    /// is left as ✕ leaves it, what stood under the slot in the viewing column
    /// or the column closed (lane-two-columns B13). Whether anything was
    /// taken.
    pub(super) fn drop_detail_items(&mut self, keep: impl Fn(&DetailItem) -> bool) -> bool {
        let state = &mut self.snapshot.ui_state;
        let mut taken = false;
        for slot in std::iter::once(&mut state.detail).chain(state.detail_tabs.values_mut()) {
            let had = slot.items.len();
            slot.items.retain(|item| keep(item));
            if slot.items.len() == had {
                continue;
            }
            taken = true;
            if slot.items.is_empty() {
                slot.shown = false;
            }
        }
        state.detail_tabs.retain(|_, slot| !slot.items.is_empty());
        if taken {
            self.persist_ui_state();
        }
        taken
    }

    /// Keeps `ui_state.detail` the slot of the Agent tab in front (operator
    /// decision 2026-11-08): the slot a tab opened stays with it while the
    /// operator clicks about inside the tab, another tab in front shows its
    /// own slot or none, and a tab that closed takes its slot with it. With
    /// no Workspace in front nothing moves. After a launch, a slot kept
    /// window-wide by an older build attaches to the tab then in front.
    /// Whether the front slot or a saved slot changed.
    ///
    /// It runs on every event, terminal input included, so the work per call
    /// is one lookup of the front tab and, while the front tab is the slot's
    /// home with no other slot held, nothing more.
    pub(super) fn sync_detail_home(&mut self) -> bool {
        let Some(front) = self
            .front_workspace_key()
            .and_then(|key| self.front_visible_tab(&key))
            .map(|(tab_id, _delegated)| tab_id)
        else {
            return false;
        };
        let state = &self.snapshot.ui_state;
        let home_moved = state.detail_home.as_deref() != Some(front.as_str());
        if !home_moved && state.detail_tabs.is_empty() {
            return false;
        }
        let closed: BTreeSet<String> = state
            .detail_tabs
            .keys()
            .map(String::as_str)
            .chain(state.detail_home.as_deref())
            .filter(|tab| self.tab_closed(tab))
            .map(str::to_owned)
            .collect();
        if !home_moved && closed.is_empty() {
            return false;
        }
        let state = &mut self.snapshot.ui_state;
        let mut changed = false;
        for tab in &closed {
            changed |= state.detail_tabs.remove(tab).is_some();
        }
        if !home_moved {
            if changed {
                self.persist_ui_state();
            }
            return changed;
        }
        let before = state.detail.top().cloned();
        match state.detail_home.replace(front.clone()) {
            // The first tab in front adopts the slot as it was loaded.
            None => {}
            Some(home) => {
                let leaving = std::mem::take(&mut state.detail);
                let leaving_held = leaving != crate::model::DetailSlot::default();
                if leaving_held && !closed.contains(&home) {
                    state.detail_tabs.insert(home, leaving);
                }
                state.detail = state.detail_tabs.remove(&front).unwrap_or_default();
                changed |= leaving_held || state.detail != crate::model::DetailSlot::default();
            }
        }
        if changed {
            self.read_detail_top(before.as_ref(), false);
            self.persist_ui_state();
        }
        changed
    }

    /// Whether the catalog that owns `tab` is here and does not list it: a
    /// tab of this machine is judged against the navigator once it lists any
    /// tab, a device's against that device's session once it has one, so a
    /// launch before a session arrives, or a device that is offline, drops
    /// no slot.
    fn tab_closed(&self, tab: &str) -> bool {
        let lists = |workspaces: &[crate::model::WorkspaceSnapshot]| {
            workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .flat_map(|checkout| &checkout.tabs)
                .any(|row| row.id.as_deref() == Some(tab))
        };
        let device = self.snapshot.status.remote.iter().find(|remote| {
            tab.strip_prefix("remote:")
                .and_then(|rest| rest.strip_prefix(remote.target_id.as_str()))
                .is_some_and(|rest| rest.starts_with(':'))
        });
        match device {
            Some(remote) => remote
                .session
                .as_ref()
                .is_some_and(|session| !lists(&session.workspaces)),
            None if tab.starts_with("remote:") => false,
            None => {
                let local = &self.snapshot.navigator.workspaces;
                local
                    .iter()
                    .flat_map(|workspace| &workspace.checkouts)
                    .any(|checkout| !checkout.tabs.is_empty())
                    && !lists(local)
            }
        }
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
        // A branch file's diff is read while the slot shows it, and
        // forgotten when it shows anything else; opened again, it is read
        // again.
        let again = match &top {
            Some(DetailItem::Branch {
                workspace_id,
                row,
                path: Some(path),
            }) if again => Some((workspace_id.clone(), row.clone(), path.clone())),
            _ => None,
        };
        self.sync_branch_diffs(again.as_ref());
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
            // A map draws the plan the snapshot already carries, a project's
            // worktrees and cost what its row and the usage frame already
            // carry, and a branch card its 제안 row.
            Some(
                DetailItem::Plan { .. }
                | DetailItem::Worktrees { .. }
                | DetailItem::Cost { .. }
                | DetailItem::Remains { .. }
                | DetailItem::Branch { .. },
            )
            | None => {
                if matches!(before, Some(DetailItem::Record { .. })) {
                    self.close_session_record();
                }
            }
        }
    }
}
