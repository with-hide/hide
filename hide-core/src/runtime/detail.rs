//! The sidebar's lens, 태스크 view option and project (PRD sidebar-task-lens
//! D-10, D-11, D-25, D-48), one UI value each kept in `core-state.json` with
//! the rest of the UI state, and Hide's cards (D-50): a pull request's card,
//! a session's record or plan, a mission's map, a project's worktrees, cost
//! or what a worktree left, a Hide AI branch. A card opens as a tab on the
//! tray of the Agent tab it was pressed for (PRD lane-desk-pair-view D-12,
//! D-13), in one event with the tab coming forward, and its data is read
//! only while it is the tray's tab in front (`sync_card_reads`). An older
//! build's 보기 slots move onto their pane's tray once (`move_slot_cards`).

use super::workspace_view::WorkspaceKey;
use super::*;
use crate::model::{
    DetailItem, PRACTICE_CHAPTER_LIMIT, PRACTICE_STEP_LIMIT, PracticeProgress, SidebarLens,
    SidebarProject, TaskView, practice_chapter_id_valid,
};
use crate::workspace_views::Place;
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

/// Which Agent tab's tray a card opens on: where it was pressed (PRD
/// lane-desk-pair-view D-12, B6, B7). `detail_open` and `pr_open` carry it.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum DetailPlace {
    /// The tray in front, beside what it holds, with the tab and the
    /// keyboard where they are: what a card opens from inside a card (a
    /// child's record, a pull request in a record or a mission node's plan)
    /// is a new tab of the same tray (B7).
    #[default]
    Here,
    /// The tray of the tab of the pane whose agent row, head or chip was
    /// pressed, which takes the keyboard (`{"pane": "<pane id>"}`).
    Pane(String),
    /// The tray of the tab of the agent that owns the item, pressed where
    /// no agent is (the PR list, Memory, a mission's row): a pull request's
    /// maker, a session's own pane, a mission's Conductor
    /// (`detail_owner_pane`); with none, the tray in front.
    Owner,
}

/// A task's row asked for from outside the list (B40): a pane head's
/// agent-tree button or a started backlog item.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TaskRevealPayload {
    pub(super) pane_id: String,
    /// The agent's session, whose record opens as a card on the tray in
    /// front; absent for an agent with no session yet, which only shows its
    /// row.
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

    /// `detail_open` and `pr_open`: `item` as a card on the tray of the tab
    /// `at` names, in one event (PRD lane-desk-pair-view B6, B7). An owner
    /// with no pane on this machine leaves the card on the tray in front.
    pub(super) fn open_detail_at(&mut self, item: DetailItem, at: DetailPlace) -> bool {
        let pane = match at {
            DetailPlace::Here => None,
            DetailPlace::Pane(pane) => Some(pane),
            DetailPlace::Owner => self.detail_owner_pane(&item),
        };
        match pane {
            Some(pane) => self.open_card_for_pane(pane, item),
            None => self.open_card_in_front(item),
        }
    }

    /// `item` on the tray of the Workspace in front, the tab and keyboard
    /// staying where they are.
    pub(super) fn open_card_in_front(&mut self, item: DetailItem) -> bool {
        let Some(key) = self.front_workspace_key() else {
            crate::diagnostic!(serde_json::json!({
                "component": "detail", "kind": "card_open.no_workspace",
            }));
            return false;
        };
        let place = self.front_place(&key);
        self.open_card_on(&key, &place, item)
    }

    /// `item` on the tray of `pane`'s tab, the pane taking the keyboard. A
    /// pane of this machine comes forward with its tab on this event; the
    /// card is placed on that tab's tray by the tab, not by whichever tray
    /// the Workspace shows before the move settles. A device's pane comes
    /// forward only when the device answers, so the card waits on the tray
    /// of the pane's tab there, its lane opened for it. A pane no catalog
    /// lists any more (it closed, or its agent sleeps) leaves the card on
    /// the tray in front, as a card pressed where no agent is opens.
    fn open_card_for_pane(&mut self, pane: String, item: DetailItem) -> bool {
        if self.local_tab_pane(&pane) {
            let place = self
                .pane_workspace_of(&pane)
                .and_then(|key| Some((Place::Tray(self.pane_tab(&key, &pane)?), key)));
            self.focus_pane(pane, PaneFocusOrigin::Operator, None);
            if self.snapshot.status.last_error.is_none()
                && !self.device_in_front(self.node.as_str())
            {
                self.bring_device_forward(self.node.as_str().to_owned());
            }
            return match place {
                Some((place, key)) => self.open_card_on(&key, &place, item),
                None => self.open_card_in_front(item),
            };
        }
        let Some((device, key, tab)) = self.device_pane_place(&pane) else {
            crate::diagnostic!(serde_json::json!({
                "component": "detail", "kind": "detail_open.pane_unplaced", "pane": pane,
            }));
            return self.open_card_in_front(item);
        };
        self.open_card_on(&key, &Place::Tray(tab), item);
        self.request_remote_control(RemoteControlPayload {
            target_id: device,
            request_id: format!("detail-open:{}:{pane}", unix_milliseconds()),
            report_pane_focus_outcome: false,
            focus_device: true,
            request: RemoteControlRequest::FocusPane { pane_id: pane },
        });
        true
    }

    /// Puts `item` on the tray at `place` as its tab in front: the card the
    /// checkout holds already comes there (D-20), else a new tab, room made
    /// first. The operator pressed it, so a closed lane opens on the tray
    /// (conductor decision 3, as opening a file does), and the card is read
    /// again even when it was the tab in front.
    pub(super) fn open_card_on(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        item: DetailItem,
    ) -> bool {
        if self.place_card_on(key, place, item) {
            self.open_tray_for(key, place);
        }
        true
    }

    /// `open_card_on` without opening the lane: pair view draws its desk
    /// whatever the lane's state, so a card it puts there leaves the lane as
    /// it was (PRD lane-desk-pair-view D-44). Whether the card was placed; a
    /// refusal is set as the error.
    pub(super) fn place_card_on(
        &mut self,
        key: &WorkspaceKey,
        place: &Place,
        item: DetailItem,
    ) -> bool {
        let existing =
            self.claim_existing_view(key, place, |display| display.card.as_ref() == Some(&item));
        if existing.is_none() && !self.make_view_room(key, place) {
            let error = crate::view_layout::LayoutError::DisplayLimit;
            self.set_error(error.kind(), error.message(), false);
            return false;
        }
        let placed = self.change_view_layout(key, place, |layout, stamp| {
            match &existing {
                Some((id, _)) => {
                    layout.focus(id, stamp)?;
                }
                None => {
                    let display = layout.new_card_display(item.clone());
                    let area = layout.active_area().id.clone();
                    layout.insert(&area, display, stamp)?;
                }
            }
            Ok(((), true))
        });
        if let Err(error) = placed {
            self.set_error(error.kind(), error.message(), false);
            return false;
        }
        if let Some(store) = self.workspace_views.as_mut() {
            store.card_read_again = true;
        }
        true
    }

    /// The device a pane of a connected device runs on, the Workspace that
    /// holds it and its Agent tab, as that device's session lists them.
    fn device_pane_place(&self, pane: &str) -> Option<(String, WorkspaceKey, String)> {
        self.snapshot.status.remote.iter().find_map(|remote| {
            let (checkout, tab) = remote
                .session
                .as_ref()?
                .workspaces
                .iter()
                .flat_map(|workspace| &workspace.checkouts)
                .find_map(|checkout| {
                    checkout
                        .tabs
                        .iter()
                        .find(|tab| tab.panes.iter().any(|row| row.id == pane))
                        .and_then(|tab| tab.id.clone())
                        .map(|tab| (checkout, tab))
                })?;
            Some((
                remote.target_id.clone(),
                (remote.target_id.clone(), checkout.path.clone()),
                tab,
            ))
        })
    }

    /// The pane on this machine whose agent owns `item`: the session's own
    /// pane for its record or plan, the Conductor's for a mission while it
    /// holds the mission, the agent that made a pull request for its card.
    /// An owner on a connected device is not looked up, so its card opens
    /// on the tray in front (`docs/INSTALL.md`, What each system supports).
    pub(super) fn detail_owner_pane(&self, item: &DetailItem) -> Option<String> {
        self.detail_owner_pane_among(item, &self.live_sessions())
    }

    /// `detail_owner_pane` over `sessions`, the machine's live sessions by
    /// pane, so a caller that asks for many cards reads them once; it
    /// borrows the project it looks in rather than copying it.
    pub(super) fn detail_owner_pane_among(
        &self,
        item: &DetailItem,
        sessions: &[(String, crate::links::ledger::SessionIdentity)],
    ) -> Option<String> {
        match item {
            DetailItem::Record { session } | DetailItem::Plan { session } => {
                if session.device_id != self.node.as_str() {
                    return None;
                }
                sessions
                    .iter()
                    .find(|(_, live)| live == session)
                    .map(|(pane, _)| pane.clone())
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
                let (_, project) = self.issue_project_ref(workspace_id)?;
                super::pull_requests::pull_request_lead(project, *number)
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

    /// The Agent tab that holds `pane` in the Workspace `key`, on this
    /// machine or a device.
    pub(super) fn pane_tab(&self, key: &WorkspaceKey, pane: &str) -> Option<String> {
        self.checkout_tabs(key)
            .find(|tab| tab.panes.iter().any(|row| row.id == pane))
            .and_then(|tab| tab.id.clone())
    }

    /// The Agent tabs of the Workspace `key`, on this machine or a device.
    pub(super) fn checkout_tabs(
        &self,
        key: &WorkspaceKey,
    ) -> impl Iterator<Item = &crate::model::TabSnapshot> {
        let workspaces: &[crate::model::WorkspaceSnapshot] = if key.0 == self.node.as_str() {
            &self.snapshot.navigator.workspaces
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == key.0)
                .and_then(|remote| remote.session.as_ref())
                .map_or(&[], |session| session.workspaces.as_slice())
        };
        workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .filter(move |checkout| checkout.path == key.1)
            .flat_map(|checkout| &checkout.tabs)
    }

    /// B40: the sidebar shows the 태스크 list with the row's ancestors
    /// unfolded, and the agent's record opens as a card, in one event.
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
            self.open_card_in_front(DetailItem::Record { session });
        }
        // The lens, the sidebar and the unfolded ancestors are saved even
        // when the record was already in front.
        self.persist_ui_state();
        true
    }

    /// Starts the read of the card the tray in front shows when it changed,
    /// or when a press opened it again: a card's body, files and threads, or
    /// a record. A record no tray shows in front any more stops being read,
    /// and a branch file's diff is forgotten (conductor decision 3: only the
    /// tray's tab in front reads its card).
    pub(super) fn sync_card_reads(&mut self) {
        let top = self.shown_card().cloned();
        let Some(store) = self.workspace_views.as_mut() else {
            return;
        };
        let again = std::mem::take(&mut store.card_read_again);
        if store.card_read == top && !again {
            return;
        }
        let before = std::mem::replace(&mut store.card_read, top.clone());
        let branch_again = match &top {
            Some(DetailItem::Branch {
                workspace_id,
                row,
                path: Some(path),
            }) if again => Some((workspace_id.clone(), row.clone(), path.clone())),
            _ => None,
        };
        self.sync_branch_diffs(branch_again.as_ref());
        let was_record = matches!(before, Some(DetailItem::Record { .. }));
        match top {
            Some(DetailItem::PullRequest {
                workspace_id,
                number,
            }) => {
                if was_record {
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
                if was_record {
                    self.close_session_record();
                }
            }
        }
    }

    /// Moves an older build's 보기 slot cards onto the tray of their pane's
    /// Agent tab, once that pane's tab is known (PRD lane-desk-pair-view
    /// D-18, B34): cards of a pane that closed go, as its slot went with it,
    /// and a device's wait while the device is away. Whether anything moved.
    pub(super) fn move_slot_cards(&mut self) -> bool {
        if self.snapshot.ui_state.moving_cards.is_empty() || self.workspace_views.is_none() {
            return false;
        }
        let front = self.front_detail_pane();
        let waiting = std::mem::take(&mut self.snapshot.ui_state.moving_cards);
        let mut kept = Vec::new();
        let mut moved = false;
        for held in waiting {
            let Some(pane) = held.pane.clone().or_else(|| front.clone()) else {
                kept.push(held);
                continue;
            };
            let place = self
                .pane_workspace_of(&pane)
                .and_then(|key| Some((self.pane_tab(&key, &pane)?, key)));
            match place {
                Some((tab, key)) => {
                    self.place_slot_cards(&key, &tab, held.cards, held.shown);
                    moved = true;
                }
                None if self.detail_pane_closed(&pane) => {
                    crate::diagnostic!(serde_json::json!({
                        "component": "detail", "kind": "slot_cards.pane_gone",
                        "pane": pane, "cards": held.cards.len(),
                    }));
                    moved = true;
                }
                None => kept.push(held),
            }
        }
        self.snapshot.ui_state.moving_cards = kept;
        if moved {
            self.persist_ui_state();
        }
        moved
    }

    /// The cards of one slot as tabs of `tab`'s tray, oldest first, the one
    /// the slot showed last in front; the tray is shown when the slot was. A
    /// card the checkout holds already stays on the desk it is on (D-20).
    /// The move closes none of the desk's Views to make room (B34): the
    /// cards the slot showed last take the room the desk has, and the rest,
    /// which the operator can open again, are left out and said.
    fn place_slot_cards(
        &mut self,
        key: &WorkspaceKey,
        tab: &str,
        cards: Vec<DetailItem>,
        shown: bool,
    ) {
        let place = Place::Tray(tab.to_owned());
        let mut cards: Vec<DetailItem> = cards
            .into_iter()
            .filter(|card| {
                !self
                    .view_displays(key)
                    .any(|display| display.card.as_ref() == Some(card))
            })
            .collect();
        let room = crate::view_layout::MAX_VIEW_DISPLAYS.saturating_sub(self.views_at(key, &place));
        let dropped = cards.len().saturating_sub(room);
        if dropped > 0 {
            crate::diagnostic!(serde_json::json!({
                "component": "detail", "kind": "slot_cards.dropped",
                "device": key.0, "tab": tab, "cards": dropped,
            }));
            cards.drain(..dropped);
        }
        for card in cards {
            let _ = self.change_view_layout(key, &place, |layout, stamp| {
                let display = layout.new_card_display(card);
                let area = layout.active_area().id.clone();
                layout.insert(&area, display, stamp)?;
                Ok(((), true))
            });
        }
        if shown && let Some(store) = self.workspace_views.as_mut() {
            store.views.entry(&key.0, &key.1).tray_entry(tab).shown = true;
            self.persist_workspace_views();
        }
        crate::diagnostic!(serde_json::json!({
            "component": "detail", "kind": "slot_cards.moved",
            "device": key.0, "tab": tab,
        }));
    }

    /// The Workspace that holds `pane`, on this machine or a device.
    pub(super) fn pane_workspace_of(&self, pane: &str) -> Option<WorkspaceKey> {
        let local = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .map(move |checkout| (&workspace.device_id, checkout))
            })
            .find(|(_, checkout)| {
                checkout
                    .tabs
                    .iter()
                    .any(|tab| tab.panes.iter().any(|row| row.id == pane))
            })
            .map(|(device, checkout)| (device.clone(), checkout.path.clone()));
        local.or_else(|| self.device_pane_place(pane).map(|(_, key, _)| key))
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

    /// Whether the catalog that owns `pane` is here and does not list it: a
    /// pane of this machine is judged against the navigator once it lists any
    /// pane, a device's against that device's session once it has one, so a
    /// launch before a session arrives, or a device that is offline, drops no
    /// card.
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
}
