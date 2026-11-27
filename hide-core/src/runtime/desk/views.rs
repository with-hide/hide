//! Desk's own desk (PRD desk-own-desk D-01-D-04, D-10-D-13): the View areas
//! Desk opens files, pages and cards in, the same tree a Workspace's Agent
//! tab keeps and the same operations on it, but no checkout's.
//!
//! The tree is kept in Desk's store (`Stored::views`, written off the lock
//! with the rest of it) and reached by the runtime's View code under its own
//! key, [`Runtime::desk_key`], whose place is always [`Place::Desk`]: a `view_layout`,
//! `browser_open` or `browser_state` that names that key acts on Desk's desk,
//! and the shell reads it from the snapshot's `desk_view`. A file on it names
//! its checkout (`Display::origin`) and is that checkout's own document,
//! read, edited and saved as the Workspace's file view does; the core holds
//! the document while a View of Desk's desk shows it, and no Workspace takes
//! it in for that (`reconcile_view_displays`). A page is the operator's and
//! no agent drives it; a card reads its data while it is in front.
//!
//! Whether the desk is drawn is the window's: the core keeps the tabs, the
//! areas and each area's tab in front. Desk switched off lets every
//! document go and keeps the tree, and switched on reads them back.

use std::collections::HashSet;

use serde::Deserialize;

use crate::model::{BrowserViewInventoryRow, DeskViewSnapshot, DetailItem};
use crate::runtime::Runtime;
use crate::runtime::documents::OpenRequestFields;
use crate::runtime::workspace_view::WorkspaceKey;
use crate::view_layout::{DisplayKind, Layout, LayoutError, ViewOrigin};
use crate::workspace_views::Place;

/// The folder Desk's desk is keyed under: Desk's caller name, which no
/// checkout's absolute path can be on any system. Its device is the core's
/// own node, where Desk runs; a page a session opened loads on that
/// session's machine (`Display::origin`).
pub(in crate::runtime) const DESK_VIEWS_PATH: &str = crate::workspace_control::DESK_CALLER;

/// `desk_view_open`: what a press in Desk's room opens on Desk's desk, in
/// front (PRD desk-own-desk B10, B11, B14, B37-B39).
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(in crate::runtime) enum OpenPayload {
    /// A file of a registered checkout: `preview` takes the area's italic
    /// tab, `beside` opens it in the area to the right (⌘↵). `device_id`
    /// routes it in hided, which checks the path against the checkout.
    File {
        workspace_id: String,
        checkout_id: String,
        path: String,
        #[serde(default)]
        #[allow(dead_code)]
        device_id: Option<String>,
        #[serde(default)]
        preview: bool,
        #[serde(default)]
        beside: bool,
    },
    /// One of Hide's cards: a mission's map, a pull request.
    Card { card: DetailItem },
    /// A View a session opened (D-12): the same file or address opens as a
    /// tab of Desk's own, and the session's View is seen where it is.
    Output {
        device_id: String,
        checkout: String,
        id: String,
    },
}

impl Runtime {
    /// The key the runtime's View code reaches Desk's desk under.
    pub(in crate::runtime) fn desk_key(&self) -> WorkspaceKey {
        (self.node.as_str().to_owned(), DESK_VIEWS_PATH.to_owned())
    }

    /// Whether `key` is Desk's desk: Desk's folder on the core's own node,
    /// the one answer every place that asks reads.
    pub(in crate::runtime) fn is_desk_key(&self, key: &WorkspaceKey) -> bool {
        key.1 == DESK_VIEWS_PATH && key.0 == self.node.as_str()
    }

    /// Whether Desk's desk is in use: Desk installed and on, in a shell
    /// that draws View areas.
    pub(in crate::runtime) fn desk_views_on(&self) -> bool {
        self.desk.worker.is_some()
            && self.desk.stored.settings.enabled
            && self.separate_view_areas()
    }

    /// Desk's desk while it is in use.
    pub(in crate::runtime) fn desk_views(&self) -> Option<&Layout> {
        self.desk_views_on().then_some(&self.desk.stored.views)
    }

    pub(in crate::runtime) fn desk_views_mut(&mut self) -> Option<&mut Layout> {
        if !self.desk_views_on() {
            return None;
        }
        Some(&mut self.desk.stored.views)
    }

    /// Raised on every change of Desk's desk, so the reconcile and the
    /// browser inventory see it as they see the Workspaces' generation.
    pub(in crate::runtime) fn desk_views_generation(&self) -> u64 {
        self.desk.views_generation
    }

    /// Desk's desk changed: counted, and saved when the change is one the
    /// store keeps (a binding to a document is not).
    pub(in crate::runtime) fn desk_views_changed(&mut self, stored: bool) {
        self.desk.views_generation += 1;
        if stored {
            self.save_desk();
        }
    }

    /// `change_view_layout` on Desk's desk: one change applied whole, or the
    /// tree as it was and why not.
    pub(in crate::runtime) fn change_desk_views<T>(
        &mut self,
        change: impl FnOnce(&mut Layout, u64) -> Result<(T, bool), LayoutError>,
    ) -> Result<T, LayoutError> {
        let now = crate::runtime::unix_milliseconds();
        let Some(layout) = self.desk_views_mut() else {
            return Err(LayoutError::UnknownArea(DESK_VIEWS_PATH.to_owned()));
        };
        let before = layout.clone();
        let stamp = layout.next_stamp(now);
        let (value, changed) = match change(layout, stamp) {
            Ok(done) => done,
            Err(error) => {
                *layout = before;
                return Err(error);
            }
        };
        if !changed {
            *layout = before;
            return Ok(value);
        }
        self.desk_views_changed(true);
        Ok(value)
    }

    /// Whether a View of Desk's desk shows the document of editor tab
    /// `tab_id`: the core holds it for that, and no Workspace takes it in.
    pub(in crate::runtime) fn desk_holds_tab(&self, tab_id: &str) -> bool {
        self.desk_views().is_some_and(|layout| {
            layout
                .displays()
                .any(|display| display.tab_id.as_deref() == Some(tab_id))
        })
    }

    /// The checkout a document opened at `key` is of, when `key` is Desk's
    /// desk: a View there names it.
    pub(in crate::runtime) fn desk_origin(
        &self,
        key: &WorkspaceKey,
        workspace_id: &str,
        checkout_id: &str,
    ) -> Option<ViewOrigin> {
        if !self.is_desk_key(key) {
            return None;
        }
        self.workspace_key(workspace_id, checkout_id)
            .map(|(device_id, path)| ViewOrigin { device_id, path })
    }

    /// The checkout of editor tab `tab_id`, as a View of Desk's desk names it.
    pub(in crate::runtime) fn tab_origin(&self, tab_id: &str) -> Option<ViewOrigin> {
        let tab = self
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)?;
        self.workspace_key(&tab.workspace_id, &tab.checkout_id)
            .map(|(device_id, path)| ViewOrigin { device_id, path })
    }

    /// `desk_view_open`. Returns whether the snapshot changed.
    pub(in crate::runtime) fn desk_view_open(&mut self, payload: OpenPayload) -> bool {
        if !self.separate_view_areas() {
            return super::refused("desk_views_unsupported");
        }
        match payload {
            OpenPayload::File {
                workspace_id,
                checkout_id,
                path,
                preview,
                beside,
                ..
            } => {
                // A refused open sets the error the window shows, a change too.
                self.desk_open_file(&workspace_id, &checkout_id, &path, preview, beside);
                true
            }
            OpenPayload::Card { card } => self.open_card_on(&self.desk_key(), &Place::Desk, card),
            OpenPayload::Output {
                device_id,
                checkout,
                id,
            } => self.desk_open_output((device_id, checkout), &id),
        }
    }

    /// A file of the checkout `workspace_id`/`checkout_id` in front of Desk's
    /// desk, as the Workspace's file view opens one (B14, B37, B38). Returns
    /// whether the file was placed or its read began; a refusal (the checkout
    /// gone, no room, an unreadable file) is the error the window shows.
    pub(in crate::runtime) fn desk_open_file(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        preview: bool,
        beside: bool,
    ) -> bool {
        if self.catalog_checkout(workspace_id, checkout_id).is_none() {
            self.set_error(
                "file.invalid_context",
                "The file's project or checkout is no longer available",
                false,
            );
            return false;
        }
        let placed = self.open_file_in_view(
            &self.desk_key(),
            workspace_id,
            checkout_id,
            path,
            preview,
            beside,
        );
        crate::diagnostic!(serde_json::json!({
            "component": "desk",
            "kind": "desk_views.file_open",
            "checkout": checkout_id,
            "preview": preview,
            "beside": beside,
            "placed": placed,
        }));
        placed
    }

    /// Desk's agent's `hide file open`: the file its host already read, put
    /// in front of Desk's desk now through the tab a pane's open takes
    /// (`read_file_tab`), so the View it lands in is known here and can
    /// carry who opened it. The View's id, or `None` when the open was
    /// refused.
    pub(in crate::runtime) fn desk_open_read_file(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        material: crate::workspace_control::ActionMaterial,
    ) -> Option<String> {
        let path = material.path;
        let key = self.desk_key();
        let origin = self.desk_origin(&key, workspace_id, checkout_id);
        // Room first, as a pane's open asks it before its tab exists: a
        // refused open leaves no tab behind for a Workspace to take in (B15).
        let placed = self.catalog_checkout(workspace_id, checkout_id).is_some()
            && self.admit_view_open_from(
                &key,
                &Place::Desk,
                origin.as_ref(),
                &path,
                DisplayKind::File,
                None,
                false,
                false,
            )
            && match self.read_file_tab(workspace_id, checkout_id, &path, material.file) {
                Ok(tab_id) => self.show_file_in_view(
                    super::super::editor::PreparedFileTab::Open(tab_id),
                    &key,
                    workspace_id,
                    checkout_id,
                    &path,
                    false,
                    false,
                ),
                Err(refusal) => super::refused(refusal.reason),
            };
        let view = placed
            .then(|| {
                self.view_showing(
                    &key,
                    &Place::Desk,
                    origin.as_ref(),
                    &path,
                    DisplayKind::File,
                    None,
                )
            })
            .flatten();
        crate::diagnostic!(serde_json::json!({
            "component": "desk",
            "kind": "desk_views.agent_file_open",
            "checkout": checkout_id,
            "placed": view.is_some(),
        }));
        view
    }

    /// A View a session opened, as a tab of Desk's desk (D-12): a file of
    /// its checkout, a page at its address as a new page of Desk's that loads
    /// on the session's machine, a card as itself; a diff shows its file,
    /// since Desk's desk has no diff tab. The session's View stays where it
    /// is, and is seen once it opened here (B25, B34); one that did not open
    /// stays to look at.
    fn desk_open_output(&mut self, key: WorkspaceKey, id: &str) -> bool {
        let Some(display) = self.view_display(&key, id).cloned() else {
            return super::refused("desk_output_gone");
        };
        // A View already on Desk's desk (one Desk's agent opened, listed in
        // 한눈에) comes in front where it is, and is seen.
        if self.is_desk_key(&key) {
            let focused =
                self.change_desk_views(|layout, stamp| Ok(((), layout.focus(id, stamp)?)));
            if let Err(error) = focused {
                crate::diagnostic!(serde_json::json!({
                    "component": "desk",
                    "kind": "desk_views.output_focus_failed",
                    "error": error.kind(),
                }));
            }
            self.mark_view_seen(&key, id);
            return true;
        }
        let opened = match display.kind {
            DisplayKind::File | DisplayKind::Diff => {
                match self.display_source(&key, &display.path) {
                    Some((workspace_id, checkout_id)) => self.desk_open_file(
                        &workspace_id,
                        &checkout_id,
                        &display.path,
                        false,
                        false,
                    ),
                    None => super::refused("desk_output_gone"),
                }
            }
            DisplayKind::Browser => {
                let url = display
                    .url
                    .clone()
                    .unwrap_or_else(|| crate::view_layout::BLANK_PAGE.to_owned());
                let origin = ViewOrigin {
                    device_id: key.0.clone(),
                    path: key.1.clone(),
                };
                self.desk_open_page(&url, origin)
            }
            DisplayKind::Card => match display.card.clone() {
                Some(card) => self.place_card_on(&self.desk_key(), &Place::Desk, card),
                None => super::refused("desk_output_gone"),
            },
            // An outside file is the desktop window's grant, which only its
            // own Workspace's tab stands on.
            DisplayKind::External => super::refused("desk_output_external"),
        };
        if opened {
            self.mark_view_seen(&key, id);
        }
        // A refusal sets the error the window shows, a change either way.
        true
    }

    /// A new page of Desk's at `url` from the checkout `origin`, or the page
    /// Desk's desk holds at it from there brought in front and loaded again.
    /// Returns whether it opened; a refusal is the window's error.
    fn desk_open_page(&mut self, url: &str, origin: ViewOrigin) -> bool {
        match self.open_browser_at(url, self.desk_key(), Some(origin)) {
            Ok(_) => true,
            Err(kind) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "desk",
                    "kind": "desk_views.page_refused",
                    "reason": kind,
                }));
                false
            }
        }
    }

    /// Desk was switched on or off (B33): off lets every document of Desk's
    /// desk go, unless a Workspace's View shows it or it holds unsaved work,
    /// which its checkout then takes in as any document no View shows; the
    /// tree stays. On reads them back.
    pub(in crate::runtime) fn desk_views_switched(&mut self, on: bool) {
        self.desk.views_read.clear();
        self.desk.views_generation += 1;
        if on {
            self.restore_desk_views();
            return;
        }
        let tabs: Vec<String> = self
            .desk
            .stored
            .views
            .displays_mut()
            .into_iter()
            .filter_map(|display| display.tab_id.take())
            .collect();
        let mut released = 0;
        for tab_id in tabs {
            let shown = self
                .every_view()
                .any(|(_, display)| display.tab_id.as_deref() == Some(tab_id.as_str()));
            if shown || self.document_kept(&tab_id) {
                continue;
            }
            if self.close_file_tab_now(&tab_id) {
                released += 1;
            }
        }
        crate::diagnostic!(serde_json::json!({
            "component": "desk",
            "kind": "desk_views.released",
            "documents": released,
        }));
    }

    /// Reads back the documents of Desk's desk whose checkout can be read
    /// now, once per file while Desk is on (B8, B41): after a restart, after
    /// Desk is switched on, and as each device's checkouts become readable.
    /// Nothing takes the screen: each View binds to its document when it
    /// lands. Returns whether a read began.
    pub(in crate::runtime) fn restore_desk_views(&mut self) -> bool {
        let Some(layout) = self.desk_views() else {
            return false;
        };
        let wanted: Vec<(WorkspaceKey, String)> = layout
            .displays()
            .filter(|display| display.kind == DisplayKind::File && display.tab_id.is_none())
            .filter_map(|display| Some((display.origin.as_ref()?.key(), display.path.clone())))
            .collect::<HashSet<_>>()
            .into_iter()
            .filter(|file| !self.desk.views_read.contains(file))
            .collect();
        let mut began = false;
        for (key, path) in wanted {
            if self.view_root_wait(&key).is_some() {
                continue;
            }
            let Some((workspace_id, checkout_id)) = self.checkout_ids(&key.0, &key.1) else {
                continue;
            };
            self.desk.views_read.insert((key, path.clone()));
            let open = self.snapshot.editor.tabs.iter().any(|tab| {
                tab.kind == crate::model::EditorTabKind::File
                    && tab.workspace_id == workspace_id
                    && tab.checkout_id == checkout_id
                    && tab.path == path
            });
            if open || self.document_opens_path(&workspace_id, &checkout_id, &path) {
                continue;
            }
            began = true;
            match self.document_source(&workspace_id, &checkout_id) {
                Ok((root, channel)) => self.start_document_open(
                    root,
                    channel,
                    OpenRequestFields {
                        follow: false,
                        workspace_id,
                        checkout_id,
                        path,
                        preview: false,
                        reload: false,
                        reveal: None,
                        restore: true,
                        placement: None,
                    },
                ),
                Err(message) => {
                    self.insert_unavailable_file_tab(
                        &workspace_id,
                        &checkout_id,
                        &path,
                        false,
                        message,
                    );
                }
            }
        }
        began
    }

    /// The pages of Desk's desk, for the desktop host's inventory: held on
    /// Desk's own desk, loaded while hidden as any desk's page is.
    pub(in crate::runtime) fn desk_browser_inventory(&self) -> Vec<BrowserViewInventoryRow> {
        let Some(layout) = self.desk_views() else {
            return Vec::new();
        };
        layout
            .areas()
            .into_iter()
            .flat_map(|area| {
                area.displays
                    .iter()
                    .filter(|display| display.kind == DisplayKind::Browser)
                    .map(|display| BrowserViewInventoryRow {
                        device_id: self.node.as_str().to_owned(),
                        path: DESK_VIEWS_PATH.to_owned(),
                        area_id: area.id.clone(),
                        view_id: display.id.clone(),
                        tray: Some(DESK_VIEWS_PATH.to_owned()),
                        url: display.url.clone(),
                        load: display.load,
                    })
            })
            .collect()
    }

    /// The documents in front of Desk's desk's areas, which the snapshot's
    /// `documents` section carries beside the front Workspace's.
    pub(in crate::runtime) fn desk_front_documents(&self) -> Vec<String> {
        let Some(layout) = self.desk_views() else {
            return Vec::new();
        };
        layout
            .areas()
            .into_iter()
            .filter_map(|area| {
                let active = area.active.as_ref()?;
                let display = area.displays.iter().find(|display| &display.id == active)?;
                (display.kind == DisplayKind::File)
                    .then(|| display.tab_id.clone())
                    .flatten()
            })
            .collect()
    }

    /// The card in front of Desk's desk's area in use, read while it is
    /// there (B4).
    pub(in crate::runtime) fn desk_front_card(&self) -> Option<&DetailItem> {
        let layout = self.desk_views()?;
        let area = layout.active_area();
        layout.display(area.active.as_ref()?)?.card.as_ref()
    }

    /// The snapshot's `desk_view`, built on every snapshot read as the front
    /// Workspace's is: absent while Desk's desk is not in use.
    pub(in crate::runtime) fn publish_desk_view(&mut self) {
        let published = self.desk_views().map(|layout| DeskViewSnapshot {
            device_id: self.node.as_str().to_owned(),
            path: DESK_VIEWS_PATH.to_owned(),
            layout: self.view_layout_snapshot(&self.desk_key(), layout),
            opened: self.desk.views_opened,
        });
        self.snapshot.desk_view = published;
    }

    /// Desk's agent's `hide file open` placed a file (B35): the window that
    /// shows Desk shows the desk for it, by this count moving.
    pub(in crate::runtime) fn note_desk_agent_open(&mut self) {
        self.desk.views_opened += 1;
    }

    /// The checkout Desk's Explorer changes: the registered checkout at
    /// `root` on `device`, which Desk's session shows whatever is in front
    /// (PRD desk-2-ui B35). Desk switched off owns no Explorer.
    pub(in crate::runtime) fn desk_explorer_target(
        &self,
        device: &str,
        root: &str,
    ) -> Option<crate::runtime::ExplorerTarget> {
        if self.desk.worker.is_none() || !self.desk.stored.settings.enabled {
            return None;
        }
        self.catalog_workspaces()
            .filter(|workspace| workspace.device_id == device)
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.path == root)
                    .map(|checkout| crate::runtime::ExplorerTarget {
                        workspace_id: workspace.id.clone(),
                        checkout_id: checkout.id.clone(),
                        device_id: workspace.device_id.clone(),
                        root: checkout.path.clone(),
                        desk: true,
                    })
            })
    }

    /// Desk's store as it stands, for a test of what is saved.
    #[cfg(test)]
    pub(in crate::runtime) fn desk_stored_for_test(&self) -> &crate::desk::Stored {
        &self.desk.stored
    }

    /// Desk's store as a start reads it from its file, for a test of a
    /// restart.
    #[cfg(test)]
    pub(in crate::runtime) fn read_desk_store_for_test(&mut self, stored: crate::desk::Stored) {
        self.desk.stored = stored;
    }
}
