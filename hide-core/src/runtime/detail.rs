//! The sidebar's lens, 태스크 view option and project, and the lane's 보기
//! slot (PRD sidebar-task-lens D-10, D-11, D-25, D-48, D-50): one
//! UI value each (the slot one per pane), kept in `core-state.json` with the
//! rest of the UI state. The slot
//! holds a pull request's card, a session's record or a mission's map,
//! belongs to the pane it was opened for (`ui_state.detail_pane`; operator
//! decision 2026-11-09, "a card opens where it was pressed"), and starts the
//! read of what it shows on the event that opens it, so one operator action
//! is one event. `ui_state.detail` is always the front pane's slot: another
//! pane in front swaps its own in (`sync_detail_pane`), so every reader and
//! the wire see one slot.

use super::workspace_view::WorkspaceKey;
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
    #[serde(default)]
    pub(super) at: DetailPlace,
}

/// Which pane's 보기 slot an item opens in: where it was pressed (operator
/// decision 2026-11-09). `detail_open` and `pr_open` carry it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum DetailPlace {
    /// The slot in front, over what it shows, with the tab and the keyboard
    /// where they are: what a card opens inside the slot (a child's record,
    /// a pull request in a record or a mission node's panel), for `←` to
    /// come back from.
    #[default]
    Here,
    /// The slot of the pane whose agent row, head or chip was pressed,
    /// which takes the keyboard (`{"pane": "<pane id>"}`).
    Pane(String),
    /// The slot of the agent that owns the item, pressed where no agent is
    /// (the PR list, Memory, a mission's row): a pull request's maker, a
    /// session's own pane, a mission's Conductor (`detail_owner_pane`).
    Owner,
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
        // An event that moved the pane first (an agent row's card, a card
        // opened in its own Workspace) opens in the slot it left in front.
        self.sync_detail_pane();
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

    /// `detail_open` and `pr_open`: `item` in the slot of the pane `at`
    /// names, in one event (operator decision 2026-11-09). An owner with no
    /// pane on this machine leaves the item in the slot in front.
    pub(super) fn open_detail_at(&mut self, item: DetailItem, at: DetailPlace) -> bool {
        let pane = match at {
            DetailPlace::Here => None,
            DetailPlace::Pane(pane) => Some(pane),
            DetailPlace::Owner => self.detail_owner_pane(&item),
        };
        match pane {
            Some(pane) => self.open_detail_in_pane(pane, item),
            None => self.open_detail(item),
        }
    }

    /// `item` in the slot of `pane`, the pane taking the keyboard. A pane of
    /// this machine comes forward with its tab on this event, so the item
    /// opens in front. A device's pane comes forward only when the device
    /// answers, so the item waits in that pane's slot with the lane of its
    /// Workspace opened for it, and shows when the device reports the pane in
    /// front (`sync_detail_pane`). A pane no catalog lists any more (it
    /// closed, or its agent sleeps) leaves the item in the slot in front, as
    /// a card pressed where no agent is opens.
    fn open_detail_in_pane(&mut self, pane: String, item: DetailItem) -> bool {
        if self.local_tab_pane(&pane) {
            self.focus_pane(pane, PaneFocusOrigin::Operator, None);
            if self.snapshot.status.last_error.is_none()
                && !self.device_in_front(self.node.as_str())
            {
                self.bring_device_forward(self.node.as_str().to_owned());
            }
            return self.open_detail(item);
        }
        self.sync_detail_pane();
        if self.snapshot.ui_state.detail_pane.as_deref() == Some(pane.as_str()) {
            return self.open_detail(item);
        }
        let Some((device, key)) = self.device_pane_place(&pane) else {
            crate::diagnostic!(serde_json::json!({
                "component": "detail", "kind": "detail_open.pane_unplaced", "pane": pane,
            }));
            return self.open_detail(item);
        };
        let evicted = self
            .snapshot
            .ui_state
            .detail_panes
            .open_in(pane.clone(), item);
        self.report_evicted_detail(evicted);
        self.persist_ui_state();
        self.call_lane(&key);
        self.request_remote_control(RemoteControlPayload {
            target_id: device,
            request_id: format!("detail-open:{}:{pane}", unix_milliseconds()),
            report_pane_focus_outcome: false,
            focus_device: true,
            request: RemoteControlRequest::FocusPane { pane_id: pane },
        });
        true
    }

    /// A slot the cap pushed out goes to the diagnostic log; the operator
    /// has nothing to act on.
    fn report_evicted_detail(&self, evicted: Option<crate::model::PaneDetailSlot>) {
        if let Some(gone) = evicted {
            crate::diagnostic!(serde_json::json!({
                "component": "detail", "kind": "detail.slot_evicted",
                "pane": gone.pane, "limit": crate::model::DETAIL_PANE_LIMIT,
            }));
        }
    }

    /// The device a pane of a connected device runs on and the Workspace that
    /// holds it, as that device's session lists it.
    fn device_pane_place(&self, pane: &str) -> Option<(String, WorkspaceKey)> {
        self.snapshot.status.remote.iter().find_map(|remote| {
            let checkout = remote
                .session
                .as_ref()?
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .find(|checkout| {
                    checkout
                        .tabs
                        .iter()
                        .any(|tab| tab.panes.iter().any(|row| row.id == pane))
                })?;
            Some((
                remote.target_id.clone(),
                (remote.target_id.clone(), checkout.path.clone()),
            ))
        })
    }

    /// The pane on this machine whose agent owns `item`: the session's own
    /// pane for its record or plan, the Conductor's for a mission while it
    /// holds the mission, the agent that made a pull request for its card.
    /// An owner on a connected device is not looked up, so its item opens
    /// in the slot in front (`docs/INSTALL.md`, What each system supports).
    fn detail_owner_pane(&self, item: &DetailItem) -> Option<String> {
        match item {
            DetailItem::Record { session } | DetailItem::Plan { session } => {
                if session.device_id != self.node.as_str() {
                    return None;
                }
                self.live_sessions()
                    .into_iter()
                    .find(|(_, live)| live == session)
                    .map(|(pane, _)| pane)
            }
            DetailItem::Mission { mission } => {
                let mission = self
                    .snapshot
                    .missions
                    .items
                    .iter()
                    .find(|row| row.id == *mission)?;
                let conductor = &mission.conductor;
                (mission.device_id == self.node.as_str()
                    && conductor.present
                    && self.local_tab_pane(&conductor.pane_id))
                .then(|| conductor.pane_id.clone())
            }
            DetailItem::PullRequest {
                workspace_id,
                number,
            } => {
                let (_, project) = self.issue_project(workspace_id)?;
                super::pull_requests::pull_request_lead(&project, *number)
                    .filter(|pane| self.local_tab_pane(pane))
            }
            _ => None,
        }
    }

    /// Whether a tab of this machine holds `pane`, the place `focus_pane`
    /// brings forward.
    pub(super) fn local_tab_pane(&self, pane: &str) -> bool {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .any(|tab| tab.panes.iter().any(|row| row.id == pane))
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
        self.call_lane(&key);
    }

    /// `call_front_lane` for the Workspace `key`, in front or about to come
    /// forward with a device's pane.
    fn call_lane(&mut self, key: &WorkspaceKey) {
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let entry = store.views.entry(&key.0, &key.1);
        let was_closed = entry.open_on(crate::model::LaneCallSide::View);
        if was_closed {
            entry.documents = false;
        }
        store.call_lane(key, crate::model::LaneCallSide::View);
        if was_closed {
            self.persist_workspace_views();
        }
    }

    /// Takes out of every pane's slot the items whose subject is gone (a
    /// deleted session's plan, a mission no longer kept); a slot this empties
    /// is left as ✕ leaves it, what stood under the slot in the viewing column
    /// or the column closed (lane-two-columns B13). Whether anything was
    /// taken.
    pub(super) fn drop_detail_items(&mut self, keep: impl Fn(&DetailItem) -> bool) -> bool {
        let state = &mut self.snapshot.ui_state;
        let mut taken = false;
        let kept = state.detail_panes.0.iter_mut().map(|held| &mut held.slot);
        for slot in std::iter::once(&mut state.detail).chain(kept) {
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
        // The slots this emptied go.
        state.detail_panes.retain(|_| true);
        if taken {
            self.persist_ui_state();
        }
        taken
    }

    /// Keeps `ui_state.detail` the slot of the pane in front (operator
    /// decision 2026-11-09): the slot a pane opened stays with it, another
    /// pane in front, of the same tab or another, shows its own slot or none,
    /// and a pane that closed takes its slot with it. With no pane in front
    /// nothing moves. After a launch, the slot loaded then (one an older
    /// build kept for its front tab or window-wide) attaches to the pane then
    /// in front. Whether the front slot changed.
    ///
    /// It runs on every event, terminal input included, so the work per call
    /// is one lookup of the front pane and, while that pane is the slot's,
    /// nothing more; another pane in front adds one search of the at most
    /// [`DETAIL_PANE_LIMIT`] kept slots and one look at whether the pane it
    /// leaves still exists.
    ///
    /// [`DETAIL_PANE_LIMIT`]: crate::model::DETAIL_PANE_LIMIT
    pub(super) fn sync_detail_pane(&mut self) -> bool {
        let Some(front) = self.front_detail_pane() else {
            return false;
        };
        let Some(home) = self.snapshot.ui_state.detail_pane.clone() else {
            // The first pane in front adopts the slot as it was loaded.
            self.snapshot.ui_state.detail_pane = Some(front);
            self.persist_ui_state();
            return false;
        };
        if home == front {
            return false;
        }
        let home_closed = self.detail_pane_closed(&home);
        let state = &mut self.snapshot.ui_state;
        let before = std::mem::take(&mut state.detail);
        let empty = crate::model::DetailSlot::default();
        // The incoming slot leaves the kept ones before the outgoing one joins
        // them, so at the cap the slot coming back is never the one dropped.
        let incoming = state.detail_panes.take(&front).unwrap_or_default();
        let evicted = if before != empty && !home_closed {
            state.detail_panes.keep(home, before.clone())
        } else {
            None
        };
        state.detail = incoming;
        state.detail_pane = Some(front);
        let changed = state.detail != before;
        let held = before != empty || state.detail != empty;
        self.report_evicted_detail(evicted);
        if changed {
            self.read_detail_top(before.top(), false);
        }
        if held {
            self.persist_ui_state();
        }
        changed
    }

    /// The pane with the keyboard in the tab the front Workspace shows, as
    /// Hide holds it on this machine (the tab's own focused pane while the
    /// keyboard is elsewhere) and as the device reports it on a device.
    fn front_detail_pane(&self) -> Option<String> {
        let key = self.front_workspace_key()?;
        let (tab_id, _delegated) = self.front_visible_tab(&key)?;
        let holds = |workspaces: &[crate::model::WorkspaceSnapshot], pane: &str| {
            workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .flat_map(|checkout| &checkout.tabs)
                .find(|tab| tab.id.as_deref() == Some(tab_id.as_str()))
                .is_some_and(|tab| tab.panes.iter().any(|row| row.id == pane))
        };
        if key.0 == self.node.as_str() {
            let workspaces = &self.snapshot.navigator.workspaces;
            return self
                .snapshot
                .focused
                .pane_id
                .as_deref()
                .filter(|pane| holds(workspaces, pane))
                .or_else(|| {
                    self.snapshot
                        .pane_layouts
                        .iter()
                        .find(|layout| layout.tab_id == tab_id)
                        .map(|layout| layout.focused_pane_id.as_str())
                })
                .map(str::to_owned);
        }
        let session = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|remote| remote.target_id == key.0)?
            .session
            .as_ref()?;
        session
            .focused_pane_id
            .clone()
            .filter(|pane| holds(&session.workspaces, pane))
    }

    /// Drops the kept slots of the panes that closed, so a slot ends with its
    /// pane; an agent that sleeps and wakes in a new pane finds that pane's
    /// slot empty. Whether any went.
    pub(super) fn prune_detail_panes(&mut self) -> bool {
        let closed: Vec<String> = self
            .snapshot
            .ui_state
            .detail_panes
            .0
            .iter()
            .map(|held| held.pane.as_str())
            .filter(|pane| self.detail_pane_closed(pane))
            .map(str::to_owned)
            .collect();
        if closed.is_empty() {
            return false;
        }
        self.snapshot
            .ui_state
            .detail_panes
            .retain(|held| !closed.contains(&held.pane));
        self.persist_ui_state();
        true
    }

    /// Whether the catalog that owns `pane` is here and does not list it: a
    /// pane of this machine is judged against the navigator once it lists any
    /// pane, a device's against that device's session once it has one, so a
    /// launch before a session arrives, or a device that is offline, drops no
    /// slot; the first session after a launch drops what the saved slots
    /// kept of panes that closed meanwhile.
    fn detail_pane_closed(&self, pane: &str) -> bool {
        let lists = |workspaces: &[crate::model::WorkspaceSnapshot], any: bool| {
            workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .flat_map(|checkout| &checkout.tabs)
                .any(|tab| {
                    if any {
                        !tab.panes.is_empty()
                    } else {
                        tab.panes.iter().any(|row| row.id == pane)
                    }
                })
        };
        match crate::address::pane_device(pane) {
            Some(device) => self
                .snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == device)
                .and_then(|remote| remote.session.as_ref())
                .is_some_and(|session| !lists(&session.workspaces, false)),
            None => {
                let local = &self.snapshot.navigator.workspaces;
                lists(local, true) && !lists(local, false)
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
            // A map draws the mission or the plan the snapshot already
            // carries, a project's worktrees and cost what its row and the
            // usage frame already carry, and a branch card its 제안 row.
            Some(
                DetailItem::Mission { .. }
                | DetailItem::Plan { .. }
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
