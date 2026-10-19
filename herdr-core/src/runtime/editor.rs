use super::documents::CloseAfter;
use super::workspace_view::WorkspaceKey;
use super::*;

pub(super) enum PreparedFileTab {
    /// The file already has a tab; showing it is a focus.
    Open(String),
    /// The file was read and needs a tab of its own.
    Read {
        tab_id: String,
        document: Box<EditorDocumentSnapshot>,
        place: crate::files::DocumentPlace,
    },
    /// The file is being read off the runtime lock; the tab shows when the
    /// read lands.
    Reading {
        root: crate::files::DocumentRoot,
        channel: std::sync::Arc<dyn crate::node_access::NodeLink>,
    },
}

impl Runtime {
    /// The projection can admit a failed terminal close; the effect must still
    /// prove this incarnation and failure against Herdr immediately before closing.
    pub(super) fn failed_terminal_id(&self, pane_id: &str) -> Option<&str> {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|status| status.session.as_ref())
                    .flat_map(|session| session.workspaces.iter()),
            )
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .find(|pane| pane.id == pane_id)
            .and_then(|pane| pane.failed_terminal_id.as_deref())
    }
    pub(super) fn file_tab_id(workspace_id: &str, checkout_id: &str, path: &str) -> String {
        format!("file:{workspace_id}:{checkout_id}:{path}")
    }

    /// The id a new tab for `path` takes. A renamed or moved tab keeps the id
    /// its old path gave it, so a new file later opened at that old path
    /// would collide with it and be dropped as already open; that tab gets
    /// the first free numbered id instead (B9, B18).
    pub(super) fn new_file_tab_id(
        &self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
    ) -> String {
        let base = Self::file_tab_id(workspace_id, checkout_id, path);
        let taken = |id: &str| {
            self.snapshot
                .editor
                .tabs
                .iter()
                .any(|tab| tab.id == id && tab.path != path)
        };
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|n| format!("{base}#{n}"))
            .find(|id| !taken(id))
            .expect("an unbounded range yields a free id")
    }

    pub(super) fn activate_editor_tab(&mut self, tab_id: &str) -> Result<(), String> {
        let tab = self
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .cloned()
            .ok_or_else(|| format!("Editor tab {tab_id} is not open"))?;
        let document = match tab.kind {
            // A restored tab whose file could not be read has no document;
            // showing it shows why (B20).
            EditorTabKind::File if tab.unavailable_reason.is_some() => None,
            EditorTabKind::File => {
                let document = self
                    .editor_documents
                    .get(tab_id)
                    .ok_or_else(|| format!("File tab {tab_id} has no document state"))?;
                // With View areas the documents ride their own snapshot
                // section, one per visible display (PRD S7 A4), so a
                // keystroke re-sends one document, not the editor, and
                // the editor carries no copy.
                (!self.separate_view_areas()).then(|| EditorDocumentSnapshot::clone(document))
            }
            EditorTabKind::Diff => {
                if tab.diff_committed.is_none() {
                    return Err(format!("Diff tab {tab_id} has no comparison scope"));
                }
                None
            }
            EditorTabKind::Session => None,
        };
        if let Some(active_id) = self.snapshot.editor.active_tab_id.as_deref()
            && active_id != tab_id
        {
            self.editor_tab_history.retain(|known| known != active_id);
            self.editor_tab_history.push(active_id.to_owned());
        }
        self.snapshot.editor.active_tab_id = Some(tab_id.to_owned());
        self.snapshot.editor.archive_detail = None;
        match tab.kind {
            EditorTabKind::File => {
                self.snapshot.editor.document = document;
                // The Explorer shows the front checkout's tree, which has no
                // row for another checkout's file a terminal link opened here.
                if self.front_checkout() == Some((&tab.workspace_id, &tab.checkout_id)) {
                    self.snapshot.ui_state.selected_path = Some(tab.path);
                }
            }
            EditorTabKind::Diff => {
                let committed = tab.diff_committed.expect("validated above");
                if self.snapshot.changes.selected_path.as_deref() != Some(tab.path.as_str())
                    || self.snapshot.changes.selected_committed != committed
                {
                    let changes = self.snapshot.changes.edit();
                    changes.diff = None;
                    changes.selected_path = Some(tab.path);
                    changes.selected_committed = committed;
                }
                self.snapshot.editor.document = None;
                self.snapshot.ui_state.selected_path = None;
            }
            EditorTabKind::Session => {
                self.snapshot.editor.document = None;
                self.snapshot.editor.archive_detail = self.archive_documents.get(tab_id).cloned();
                self.snapshot.ui_state.selected_path = None;
            }
        }
        Ok(())
    }

    pub(super) fn deactivate_editor_tab(&mut self) {
        self.snapshot.editor.active_tab_id = None;
        self.snapshot.editor.document = None;
        self.snapshot.editor.archive_detail = None;
        self.editor_tab_history.clear();
    }

    pub(super) fn sync_active_editor_document(&mut self) {
        self.snapshot.editor.document = if self.separate_view_areas() {
            None
        } else {
            self.snapshot
                .editor
                .active_tab_id
                .as_deref()
                .and_then(|tab_id| {
                    self.editor_documents.get(tab_id).and_then(|document| {
                        self.snapshot
                            .editor
                            .tabs
                            .iter()
                            .find(|tab| tab.id == tab_id && tab.kind == EditorTabKind::File)
                            .map(|_| EditorDocumentSnapshot::clone(document))
                    })
                })
        };
        self.snapshot.editor.archive_detail = self
            .snapshot
            .editor
            .active_tab_id
            .as_deref()
            .and_then(|tab_id| self.archive_documents.get(tab_id).cloned());
    }

    pub(super) fn sync_file_tab_dirty(&mut self, tab_id: &str) {
        let dirty = self
            .editor_documents
            .get(tab_id)
            .is_some_and(|document| document.dirty);
        if let Some(tab) = self
            .snapshot
            .editor
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        {
            tab.dirty = dirty;
        }
    }

    /// A close that waits on its save: the tab closes when the save lands
    /// and the draft matches what was written, and then the lane of `lane`
    /// when that close left it nothing to show (`close_emptied_lane`).
    pub(super) fn start_file_save_then_close(
        &mut self,
        close_tab_id: String,
        payload: FileSavePayload,
        lane: Option<WorkspaceKey>,
    ) -> bool {
        if payload.tab_id != close_tab_id {
            self.set_error(
                "file.close_save_mismatch",
                "The pending save did not belong to the file being closed",
                false,
            );
            return true;
        }
        self.request_file_save(payload, CloseAfter::Document { lane })
    }

    pub(super) fn prepare_file_tab(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
    ) -> Result<PreparedFileTab, String> {
        // A tab that could not read its file reads it again on the next open.
        if let Some(tab_id) = self.snapshot.editor.tabs.iter().find_map(|tab| {
            (tab.kind == EditorTabKind::File
                && tab.workspace_id == workspace_id
                && tab.checkout_id == checkout_id
                && tab.path == path
                && tab.unavailable_reason.is_none())
            .then(|| tab.id.clone())
        }) {
            return Ok(PreparedFileTab::Open(tab_id));
        }
        let (root, channel) = self.document_source(workspace_id, checkout_id)?;
        Ok(PreparedFileTab::Reading { root, channel })
    }

    /// Shows a prepared file tab. `preview` is what the click asked for: a
    /// single click wants the checkout's preview slot, every other entry
    /// point wants an ordinary tab (PRD editor-preview-tab D-09). A file that
    /// already has a tab is focused, and an ordinary open of the file that
    /// currently holds the preview slot promotes it where it sits (D-10, B4).
    pub(super) fn show_file_tab(
        &mut self,
        prepared: PreparedFileTab,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        preview: bool,
    ) {
        if self.separate_view_areas() {
            let Some(key) = self.workspace_key(workspace_id, checkout_id) else {
                self.set_error(
                    "file.invalid_context",
                    "The file's project or checkout is no longer available",
                    false,
                );
                return;
            };
            self.show_file_in_view(
                prepared,
                &key,
                workspace_id,
                checkout_id,
                path,
                preview,
                false,
            );
            return;
        }
        let tab_id = match prepared {
            PreparedFileTab::Open(tab_id) => {
                if !preview {
                    self.promote_editor_tab(&tab_id);
                }
                tab_id
            }
            PreparedFileTab::Reading { root, channel } => {
                self.start_document_open(
                    root,
                    channel,
                    documents::OpenRequestFields {
                        workspace_id: workspace_id.to_owned(),
                        checkout_id: checkout_id.to_owned(),
                        path: path.to_owned(),
                        preview,
                        reload: false,
                        reveal: None,
                        restore: false,
                        placement: None,
                    },
                );
                self.snapshot.ui_state.selected_path = Some(path.to_owned());
                return;
            }
            read @ PreparedFileTab::Read { .. } => {
                match self.insert_file_tab(read, workspace_id, checkout_id, path, preview) {
                    Some(tab_id) => tab_id,
                    None => return,
                }
            }
        };
        if let Err(message) = self.activate_editor_tab(&tab_id) {
            self.set_error("file.focus_failed", message, false);
        }
        self.snapshot.ui_state.selected_path = Some(path.to_owned());
    }

    /// Adds a read document as a tab in its checkout's strip without showing
    /// it. Returns the tab id, or `None` for a preparation that holds no
    /// document.
    pub(super) fn insert_file_tab(
        &mut self,
        prepared: PreparedFileTab,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        preview: bool,
    ) -> Option<String> {
        let PreparedFileTab::Read {
            tab_id,
            document,
            place,
        } = prepared
        else {
            return None;
        };
        self.editor_documents
            .insert(tab_id.clone(), Edited::new(*document));
        self.document_places.insert(tab_id.clone(), place);
        let tab = EditorTabSnapshot {
            id: tab_id.clone(),
            workspace_id: workspace_id.to_owned(),
            checkout_id: checkout_id.to_owned(),
            path: path.to_owned(),
            label: Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(path)
                .to_owned(),
            kind: EditorTabKind::File,
            diff_committed: None,
            markdown_live: true,
            wrap: true,
            dirty: false,
            preview,
            unavailable_reason: None,
        };
        self.place_editor_tab(tab);
        Some(tab_id)
    }

    /// Puts a new editor tab into the strip.
    ///
    /// An ordinary tab takes a slot at the end. A preview tab takes the
    /// checkout's preview slot: when the checkout already has a preview tab
    /// that is clean, the new tab replaces it in place, inheriting its strip
    /// slot, and the replaced tab's document, Markdown mode and wrap state
    /// are dropped without a Recent Closed entry (D-04). A dirty preview tab
    /// is never replaced: it is promoted where it sits and the new preview
    /// opens beside it (D-05). File and diff tabs share the one slot (D-02).
    ///
    /// With View areas the preview slots are the areas' (one preview display
    /// each, PRD S7 D-02), so a new tab only joins the editor, and the area
    /// retires the document its preview display showed (`view_areas.rs`).
    pub(super) fn place_editor_tab(&mut self, tab: EditorTabSnapshot) {
        if self.separate_view_areas() {
            self.snapshot.editor.tabs.push(tab);
            self.rebuild_tab_strips();
            return;
        }
        let replaced = tab.preview.then(|| {
            self.snapshot
                .editor
                .tabs
                .iter()
                .position(|held| {
                    held.preview
                        && held.workspace_id == tab.workspace_id
                        && held.checkout_id == tab.checkout_id
                })
                .map(|index| (index, self.snapshot.editor.tabs[index].dirty))
        });
        match replaced.flatten() {
            Some((index, false)) => {
                let old = self.retire_editor_tab(index);
                let old_entry = StripTabSnapshot::editor(&old).id;
                let new_entry = StripTabSnapshot::editor(&tab).id;
                for order in self.checkout_tab_order.values_mut() {
                    for entry in order.iter_mut().filter(|entry| **entry == old_entry) {
                        *entry = new_entry.clone();
                    }
                }
                for pending in self.pending_tab_move.values_mut() {
                    for entry in pending
                        .desired
                        .iter_mut()
                        .filter(|entry| **entry == old_entry)
                    {
                        *entry = new_entry.clone();
                    }
                }
                self.push_diagnostic(
                    "editor.preview_replaced",
                    format!("Preview tab {} replaced by {}", old.id, tab.id),
                );
                self.snapshot.editor.tabs.insert(index, tab);
            }
            Some((index, true)) => {
                let dirty = &mut self.snapshot.editor.tabs[index];
                dirty.preview = false;
                let kept = dirty.id.clone();
                self.push_diagnostic(
                    "editor.preview_kept_dirty",
                    format!("Dirty preview tab {kept} kept and promoted"),
                );
                self.snapshot.editor.tabs.push(tab);
            }
            None => self.snapshot.editor.tabs.push(tab),
        }
        self.rebuild_tab_strips();
    }

    /// Takes an editor tab out of the runtime: its snapshot entry, its
    /// document, its place in the focus history, and, when it was the active
    /// diff, the Changes selection it was showing. The caller decides what
    /// the removal means: a close records it for reopening, a preview
    /// replacement does not.
    pub(super) fn retire_editor_tab(&mut self, index: usize) -> EditorTabSnapshot {
        let tab = self.snapshot.editor.tabs.remove(index);
        let was_active = self.snapshot.editor.active_tab_id.as_deref() == Some(tab.id.as_str());
        self.editor_documents.remove(&tab.id);
        self.forget_document(&tab.id);
        self.archive_documents.remove(&tab.id);
        self.editor_tab_history.retain(|known| known != &tab.id);
        if was_active {
            self.snapshot.editor.active_tab_id = None;
            self.snapshot.editor.document = None;
            self.snapshot.editor.archive_detail = None;
            if tab.kind == EditorTabKind::Diff {
                let changes = self.snapshot.changes.edit();
                changes.selected_path = None;
                changes.diff = None;
            }
        }
        tab
    }

    /// Closes every file and diff tab of a removed device without saving and
    /// drops its closed items. A dirty tab's draft stays in the shell's own
    /// store, where a draft no tab stands for is offered for export (B26).
    pub(super) fn retire_device_editor_tabs(&mut self, device_id: &str) {
        let scope = format!("remote:{device_id}:");
        while let Some(index) = self
            .snapshot
            .editor
            .tabs
            .iter()
            .position(|tab| tab.checkout_id.starts_with(&scope))
        {
            self.retire_editor_tab(index);
        }
        let node = self.node.as_str();
        self.recent_closed
            .retain(|item| item.device_id(node) != device_id);
        self.sync_recent_closed_snapshot();
        self.forget_device_opens(&scope);
    }

    /// Makes a preview tab an ordinary tab in the same slot. Returns whether
    /// anything changed: promoting a tab that is already ordinary is a
    /// no-op, and a tab that is not open is refused with a reason. With View
    /// areas every preview display of the document is kept open instead,
    /// and the tab's flag follows them.
    pub(super) fn promote_editor_tab(&mut self, tab_id: &str) -> bool {
        if !self.snapshot.editor.tabs.iter().any(|tab| tab.id == tab_id) {
            self.set_error(
                "editor.keep_open_unknown_tab",
                format!("Editor tab {tab_id} is not open"),
                false,
            );
            return true;
        }
        if self.separate_view_areas() {
            return self.promote_view_displays(tab_id);
        }
        let Some(tab) = self
            .snapshot
            .editor
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
        else {
            return false;
        };
        if !tab.preview {
            return false;
        }
        tab.preview = false;
        self.push_diagnostic("editor.preview_promoted", format!("Tab {tab_id} kept open"));
        self.rebuild_tab_strips();
        true
    }

    pub(super) fn focus_editor_tab_context(&mut self, tab_id: &str) -> Result<(), String> {
        let tab = self
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .cloned()
            .ok_or_else(|| format!("File tab {tab_id} is not open"))?;
        let (device_id, checkout) = self
            .catalog_checkout(&tab.workspace_id, &tab.checkout_id)
            .map(|(workspace, checkout)| (workspace.device_id.clone(), checkout.clone()))
            .ok_or_else(|| {
                "The editor tab's project or checkout is no longer available".to_owned()
            })?;
        // A device's focus is its Herdr session's, which the core follows;
        // showing one of its files moves nothing on this machine.
        if device_id != self.node.as_str() {
            return self.activate_editor_tab(tab_id);
        }
        self.activate_editor_tab(tab_id)?;
        let pane_id = checkout.active_tab_id.as_deref().and_then(|id| {
            let first = checkout
                .tabs
                .iter()
                .find(|tab| tab.id.as_deref() == Some(id))
                .and_then(|tab| tab.panes.first())
                .map(|pane| pane.id.clone());
            self.tab_focus_pane_id(id, first)
        });
        self.snapshot.navigator.focused_workspace_id = Some(tab.workspace_id);
        self.snapshot.navigator.focused_checkout_id = Some(tab.checkout_id);
        self.snapshot.navigator.root_path = Some(checkout.path);
        self.sync_changes_root_path();
        self.select_terminal_pane(pane_id);
        self.operator_focused_pane_id = None;
        self.refresh_pane_read_state();
        self.persist_current_ui_state();
        Ok(())
    }

    pub(super) fn open_file_tab(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        preview: bool,
    ) {
        match self.prepare_file_tab(workspace_id, checkout_id, path) {
            Ok(prepared) => self.show_file_tab(prepared, workspace_id, checkout_id, path, preview),
            // The read failed before anything moved, so an existing preview
            // tab keeps its slot (B15).
            Err(message) => self.set_error("file.open_failed", message, true),
        }
    }

    pub(super) fn diff_tab_id(
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        committed: bool,
    ) -> String {
        let scope = if committed { "committed" } else { "working" };
        format!("diff:{workspace_id}:{checkout_id}:{scope}:{path}")
    }

    pub(super) fn show_diff_tab(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        committed: bool,
        preview: bool,
    ) {
        let tab_id = self.insert_diff_tab(workspace_id, checkout_id, path, committed, preview);
        if let Err(message) = self.activate_editor_tab(&tab_id) {
            self.set_error("diff.focus_failed", message, false);
        }
    }

    /// Puts a diff tab in its checkout's strip without showing it, or keeps
    /// the one already there (promoted unless a preview was asked for).
    pub(super) fn insert_diff_tab(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
        committed: bool,
        preview: bool,
    ) -> String {
        let tab_id = Self::diff_tab_id(workspace_id, checkout_id, path, committed);
        if self.snapshot.editor.tabs.iter().any(|tab| tab.id == tab_id) {
            if !preview {
                self.promote_editor_tab(&tab_id);
            }
            return tab_id;
        }
        self.place_editor_tab(EditorTabSnapshot {
            id: tab_id.clone(),
            workspace_id: workspace_id.to_owned(),
            checkout_id: checkout_id.to_owned(),
            path: path.to_owned(),
            label: diff_label(path, committed),
            kind: EditorTabKind::Diff,
            diff_committed: Some(committed),
            markdown_live: true,
            wrap: true,
            dirty: false,
            preview,
            unavailable_reason: None,
        });
        tab_id
    }

    pub(super) fn show_archive_tab(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        detail: ArchiveDetailSnapshot,
        preview: bool,
    ) {
        let kind = EditorTabKind::Session;
        let tab_id = format!(
            "{}:{}:{}:{}",
            detail.kind, workspace_id, checkout_id, detail.id
        );
        self.archive_documents
            .insert(tab_id.clone(), detail.clone());
        if self.snapshot.editor.tabs.iter().any(|tab| tab.id == tab_id) {
            if !preview {
                self.promote_editor_tab(&tab_id);
            }
        } else {
            self.place_editor_tab(EditorTabSnapshot {
                id: tab_id.clone(),
                workspace_id: workspace_id.to_owned(),
                checkout_id: checkout_id.to_owned(),
                path: detail.id,
                label: detail.title,
                kind,
                diff_committed: None,
                markdown_live: false,
                wrap: true,
                dirty: false,
                preview,
                unavailable_reason: None,
            });
        }
        if let Err(message) = self.activate_editor_tab(&tab_id) {
            self.set_error("archive.focus_failed", message, false);
        }
    }

    pub(super) fn next_recent_closed_key(&mut self) -> String {
        self.recent_closed_sequence += 1;
        format!(
            "reopen-{}-{}",
            unix_milliseconds(),
            self.recent_closed_sequence
        )
    }

    /// The device whose closed items the reopen command and the snapshot's
    /// `recent_closed` speak for: the one in front.
    fn reopen_device(&self) -> &str {
        self.snapshot
            .navigator
            .focused_device_id
            .as_deref()
            .unwrap_or(self.node.as_str())
    }

    /// The newest closed item the device in front can reopen. One stack keeps
    /// the twenty most recent closes of every device, and reopen never takes
    /// another device's item, so a reopen on one device cannot restore work
    /// on another.
    fn reopenable(&self) -> Option<&ClosedItem> {
        let device = self.reopen_device();
        self.recent_closed
            .iter()
            .rev()
            .find(|item| item.device_id(self.node.as_str()) == device)
    }

    pub(super) fn sync_recent_closed_snapshot(&mut self) {
        // This machine's close reservations block only this machine's reopen.
        let local = self.reopen_device() == self.node.as_str();
        let device = self.reopen_device().to_owned();
        self.snapshot.recent_closed.count = self
            .recent_closed
            .iter()
            .filter(|item| item.device_id(self.node.as_str()) == device)
            .count();
        self.snapshot.recent_closed.top_label =
            self.reopenable().map(|item| item.label().to_owned());
        self.snapshot.recent_closed.restoring = self.reopen_in_flight.is_some();
        self.snapshot.recent_closed.pending = self
            .close_capture_order
            .iter()
            .filter_map(|key| {
                self.close_operations
                    .get(key)
                    .filter(|operation| operation.request.retain_for_reopen)
                    .map(|operation| (key, operation))
            })
            .map(
                |(key, operation)| crate::model::RecentClosedPendingSnapshot {
                    key: operation.request.key.clone(),
                    target_id: operation.target_id.clone(),
                    label: operation.label.clone(),
                    phase: operation.phase.clone(),
                    checking: self.close_status_checks_in_flight.contains(key),
                    message: operation.message.clone(),
                    retryable: operation.retryable
                        && (!matches!(operation.phase.as_str(), "failed" | "refused")
                            || operation.request.context.replacement_shell),
                },
            )
            .collect();
        let newest_close = self
            .close_capture_order
            .back()
            .filter(|_| local)
            .and_then(|key| self.close_operations.get(key))
            .filter(|operation| operation.request.retain_for_reopen);
        let queueable =
            newest_close.is_some_and(PendingClose::settling) && self.reopen_after_close.is_none();
        self.snapshot.recent_closed.can_reopen = queueable
            || (self.reopenable().is_some()
                && (!local || self.close_capture_order.is_empty())
                && self.reopen_in_flight.is_none());
        self.snapshot.recent_closed.reopen_blocked_reason =
            newest_close.filter(|_| !queueable).map(|operation| {
                if operation.phase == "unknown" {
                    "The latest close result needs checking before it can be reopened.".to_owned()
                } else {
                    "The latest close is still being confirmed.".to_owned()
                }
            });
        self.sync_async_operations();
    }

    pub(super) fn push_recent_closed(&mut self, item: ClosedItem) {
        push_bounded(&mut self.recent_closed, item);
        self.sync_recent_closed_snapshot();
    }

    pub(super) fn consume_recent_closed(&mut self, key: &str) {
        if let Some(index) = self.recent_closed.iter().position(|item| item.key() == key) {
            self.recent_closed.remove(index);
        }
    }

    pub(super) fn close_file_tab_now(&mut self, tab_id: &str) -> bool {
        let Some(index) = self
            .snapshot
            .editor
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
        else {
            self.set_error(
                "file.close_unknown_tab",
                format!("File tab {tab_id} is not open"),
                false,
            );
            return true;
        };
        let was_active = self.snapshot.editor.active_tab_id.as_deref() == Some(tab_id);
        let closed_tab = self.retire_editor_tab(index);
        if closed_tab.kind == EditorTabKind::File {
            // A checkout the catalog no longer lists keeps the item; its
            // reopen then reports that the checkout is unavailable.
            let (device_id, checkout_path) = self
                .catalog_checkout(&closed_tab.workspace_id, &closed_tab.checkout_id)
                .map(|(workspace, checkout)| (workspace.device_id.clone(), checkout.path.clone()))
                .unwrap_or_else(|| (self.node.as_str().to_owned(), String::new()));
            let key = self.next_recent_closed_key();
            self.push_recent_closed(ClosedItem::File {
                key,
                device_id,
                workspace_id: closed_tab.workspace_id.clone(),
                checkout_id: closed_tab.checkout_id.clone(),
                checkout_path,
                path: closed_tab.path.clone(),
                label: closed_tab.label.clone(),
            });
        }
        self.rebuild_tab_strips();
        // With View areas the area the closed display was in shows its next
        // display (`view_areas.rs`); the focus history is the one canvas's.
        if was_active && !self.separate_view_areas() {
            while let Some(previous_id) = self.editor_tab_history.pop() {
                if self
                    .snapshot
                    .editor
                    .tabs
                    .iter()
                    .any(|tab| tab.id == previous_id)
                {
                    if let Err(message) = self.activate_editor_tab(&previous_id) {
                        self.set_error("editor.focus_failed", message, false);
                    }
                    break;
                }
            }
        }
        self.persist_current_ui_state();
        true
    }

    pub(super) fn set_reopen_notices(&mut self, notices: Vec<live::ReopenNotice>) {
        self.snapshot.recent_closed.notices = notices
            .into_iter()
            .map(|notice| crate::model::RecentClosedNoticeSnapshot {
                pane_id: notice.pane_id,
                message: notice.message,
            })
            .collect();
    }

    pub(super) fn close_context(&self, tab: &TabSnapshot) -> Option<ClosedContext> {
        let tab_id = tab.id.as_ref()?;
        let project_workspace_id = tab.workspace_id.as_ref()?;
        let checkout_id = tab.checkout_id.as_ref()?;
        let (session_workspace_id, tab_order) = self
            .herdr_workspace_tab_order
            .iter()
            .find(|(_, ids)| ids.iter().any(|id| id == tab_id))?;
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.id == *project_workspace_id)
            .find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.id == *checkout_id)
                    .map(|checkout| ClosedContext {
                        workspace_id: session_workspace_id.clone(),
                        workspace_label: workspace.label.clone(),
                        workspace_ids_before_close: self
                            .snapshot
                            .navigator
                            .workspaces
                            .iter()
                            .flat_map(|workspace| workspace.session_workspace_ids.iter().cloned())
                            .collect(),
                        tab_ids_before_close: tab_order.clone(),
                        pane_ids_before_close: tab
                            .panes
                            .iter()
                            .map(|pane| pane.id.clone())
                            .collect(),
                        checkout_id: checkout_id.clone(),
                        checkout_path: checkout.path.clone(),
                        tab_id: tab_id.clone(),
                        tab_label: tab.label.clone().unwrap_or_else(|| "Tab".into()),
                        tab_index: tab_order.iter().position(|id| id == tab_id).unwrap_or(0),
                        agent_group: self
                            .workspace_views
                            .as_ref()
                            .and_then(|s| s.views.get(&workspace.device_id, &checkout.path))
                            .and_then(|v| v.agent_tab_group(tab_id))
                            .flatten()
                            .map(str::to_owned),
                        agent_area: self
                            .workspace_views
                            .as_ref()
                            .and_then(|s| s.views.get(&workspace.device_id, &checkout.path))
                            .and_then(|v| v.group_layout(v.agent_tab_group(tab_id)?))
                            .and_then(|layout| {
                                layout.tree.areas().into_iter().find_map(|area| {
                                    area.displays
                                        .iter()
                                        .position(|item| &item.id == tab_id)
                                        .map(|index| (area.id.clone(), index))
                                })
                            }),
                        replacement_shell: self.primary_needs_shell(session_workspace_id, tab_id),
                        replacement_in_tab: false,
                        tab_name: match (tab.naming.operator_name(), tab.naming.manual_name()) {
                            (Some(name), _) => ClosedTabName::Named(name.to_owned()),
                            (None, Some(name)) => ClosedTabName::Labeled(name.to_owned()),
                            (None, None) => ClosedTabName::Unnamed,
                        },
                    })
            })
    }

    pub(super) fn closed_panes(&self, tab: &TabSnapshot) -> Vec<ClosedPane> {
        tab.panes
            .iter()
            .map(|pane| {
                let agent = self
                    .snapshot
                    .navigator
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == pane.id)
                    .map(|agent| ClosedAgent {
                        kind: agent.agent_kind.clone(),
                        session_id: agent.session_id.clone(),
                        launch: self.launch_choice_of(agent),
                    });
                ClosedPane {
                    pane_id: pane.id.clone(),
                    label: crate::start_claim::public_label(pane.herdr_label.as_deref()).or_else(
                        || crate::start_claim::public_label(pane.terminal_title.as_deref()),
                    ),
                    cwd: pane.cwd.clone(),
                    agent,
                }
            })
            .collect()
    }

    pub(super) fn close_operation_kind(target: &live::CloseCaptureTarget) -> &'static str {
        match target {
            live::CloseCaptureTarget::Pane { .. } => "pane.close",
            live::CloseCaptureTarget::Tab { .. } => "tab.close",
        }
    }

    pub(super) fn close_operation_pane_ids(
        target: &live::CloseCaptureTarget,
        panes: &[ClosedPane],
    ) -> Vec<String> {
        match target {
            live::CloseCaptureTarget::Pane { pane_id } => vec![pane_id.clone()],
            live::CloseCaptureTarget::Tab { .. } => {
                panes.iter().map(|pane| pane.pane_id.clone()).collect()
            }
        }
    }

    pub(super) fn close_operation_target_present(
        payload: &SessionSnapshotPayload,
        target: &live::CloseCaptureTarget,
        scope_id: &str,
    ) -> bool {
        match target {
            live::CloseCaptureTarget::Pane { pane_id } => {
                if let Some(layout) = payload
                    .layouts
                    .iter()
                    .find(|layout| layout.tab_id == scope_id)
                {
                    return layout.panes.iter().any(|pane| pane.pane_id == *pane_id);
                }
                if payload.panes.iter().any(|pane| pane.pane_id == *pane_id) {
                    return true;
                }
                // Without the target tab's layout, a tab that is still in the
                // snapshot is an incomplete answer, not proof that its pane
                // disappeared. A removed tab is enough to prove the pane is
                // gone because pane closes never move a pane to another tab.
                payload.tabs.iter().any(|tab| tab.tab_id == scope_id)
            }
            live::CloseCaptureTarget::Tab { tab_id } => {
                payload.tabs.iter().any(|tab| tab.tab_id == *tab_id)
            }
        }
    }

    pub(super) fn close_target_present_in_projection(&self, operation: &PendingClose) -> bool {
        match &operation.request.target {
            live::CloseCaptureTarget::Pane { pane_id } => self
                .snapshot
                .navigator
                .workspaces
                .iter()
                .flat_map(|workspace| workspace.checkouts.iter())
                .flat_map(|checkout| checkout.tabs.iter())
                .find(|tab| tab.id.as_deref() == Some(operation.scope_id.as_str()))
                .is_some_and(|tab| tab.panes.iter().any(|pane| pane.id == *pane_id)),
            live::CloseCaptureTarget::Tab { tab_id } => self
                .snapshot
                .navigator
                .workspaces
                .iter()
                .flat_map(|workspace| workspace.checkouts.iter())
                .flat_map(|checkout| checkout.tabs.iter())
                .any(|tab| tab.id.as_deref() == Some(tab_id.as_str())),
        }
    }

    /// Returns a pre-send close failure when the user-approved target has
    /// changed since the request was captured. The caller must select the new
    /// target again, so an approval for one pane range never silently covers a
    /// later pane or a newly started agent.
    pub(super) fn close_precondition_failure(&self, operation: &PendingClose) -> Option<String> {
        if let Some(reason) = self.dormant_close_precondition(operation) {
            return Some(reason);
        }
        let current_pane_ids = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .find(|tab| tab.id.as_deref() == Some(operation.scope_id.as_str()))
            .map(|tab| {
                let mut ids = tab
                    .panes
                    .iter()
                    .map(|pane| pane.id.clone())
                    .collect::<Vec<_>>();
                ids.sort();
                ids
            });
        let target_is_known =
            current_pane_ids.is_some() || self.close_target_present_in_projection(operation);
        if !target_is_known {
            // A few transport-facing callers can deliver a capture result
            // before the first authoritative projection exists. There is no
            // target identity to compare in that state, so keep the capture
            // path alive and let the later topology confirmation decide.
            return None;
        }
        let activity_status_unknown = self.snapshot.navigator.agents.iter().any(|agent| {
            let in_target = match &operation.request.target {
                live::CloseCaptureTarget::Pane { pane_id } => agent.pane_id == *pane_id,
                live::CloseCaptureTarget::Tab { .. } => operation
                    .scope_pane_ids
                    .iter()
                    .any(|pane_id| pane_id == &agent.pane_id),
            };
            in_target
                && (agent.requires_close_status_check
                    && self.failed_terminal_id(&agent.pane_id).is_none())
        });
        if activity_status_unknown {
            return Some(
                "Activity status became unknown while preparing this close; check status before closing"
                    .to_owned(),
            );
        }
        if let Some(mut current_pane_ids) = current_pane_ids {
            if !operation.scope_pane_ids.is_empty() {
                current_pane_ids.sort();
                if current_pane_ids != operation.scope_pane_ids {
                    return Some(
                        "The close target changed before it was sent; select the updated tab or pane again"
                            .to_owned(),
                    );
                }
            }
        } else if !self.close_target_present_in_projection(operation) {
            return Some(
                "The close target disappeared before it was sent; select the current target again"
                    .to_owned(),
            );
        }
        let current_protected_agent_pane_ids = self
            .snapshot
            .navigator
            .agents
            .iter()
            .filter(|agent| {
                let in_target = match &operation.request.target {
                    live::CloseCaptureTarget::Pane { pane_id } => agent.pane_id == *pane_id,
                    live::CloseCaptureTarget::Tab { .. } => operation
                        .scope_pane_ids
                        .iter()
                        .any(|pane_id| pane_id == &agent.pane_id),
                };
                in_target
                    && (agent.requires_close_confirmation
                        && self.failed_terminal_id(&agent.pane_id).is_none())
            })
            .map(|agent| agent.pane_id.as_str())
            .collect::<Vec<_>>();
        let newly_protected = current_protected_agent_pane_ids.iter().any(|pane_id| {
            !operation
                .protected_agent_pane_ids
                .iter()
                .any(|known| known == pane_id)
        });
        if newly_protected {
            return Some(
                "A new agent needs attention in this tab; review the close confirmation again"
                    .to_owned(),
            );
        }
        None
    }

    pub(super) fn cancel_close_before_effect(&mut self, key: &str, message: String) {
        let Some(operation) = self.close_operations.get_mut(key) else {
            return;
        };
        operation.phase = "refused".to_owned();
        operation.stage = "capture".to_owned();
        operation.message = Some(format!("The close was canceled: {message}"));
        operation.retryable = true;
        operation.deadline_at_unix_ms = None;
        let operation = operation.clone();
        if !operation.replacement_effect_started {
            self.finish_agent_effect(
                &operation.request.context.checkout_path,
                &format!("close:{key}"),
            );
        }
        self.clear_close_guards(&operation);
        self.restore_close_selection(operation.selection_restore.as_ref());
        self.set_reopen_notices(vec![live::ReopenNotice {
            pane_id: matches!(
                &operation.request.target,
                live::CloseCaptureTarget::Pane { .. }
            )
            .then_some(operation.target_id.clone()),
            message: operation.message.clone().unwrap_or_else(|| message.clone()),
        }]);
        self.push_diagnostic(
            "recent_closed.close_canceled",
            format!(
                "{}: target changed before close effect",
                operation.request.key
            ),
        );
        self.settle_dormant_close_result(key);
        self.promote_close_reservations();
        self.sync_recent_closed_snapshot();
    }

    pub(super) fn close_operation_holds_pane(&self, pane_id: &str) -> bool {
        self.close_operations
            .values()
            .any(|operation| operation.pane_ids.iter().any(|known| known == pane_id))
    }

    pub(super) fn close_selection_state(&self, checkout_id: Option<&str>) -> CloseSelectionState {
        let (active_tab_id, visible_tab_id) = checkout_id
            .map(|checkout_id| {
                let active_tab_id = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .find(|checkout| checkout.id == checkout_id)
                    .and_then(|checkout| checkout.active_tab_id.clone());
                let visible_tab_id = self.visible_tab_ids.get(checkout_id).cloned();
                (active_tab_id, visible_tab_id)
            })
            .unwrap_or((None, None));
        CloseSelectionState {
            focused_checkout_id: self.snapshot.navigator.focused_checkout_id.clone(),
            terminal_pane_id: self.snapshot.terminal.pane_id.clone(),
            focused_pane_id: self.snapshot.focused.pane_id.clone(),
            selected_pane_id: self.snapshot.ui_state.selected_pane_id.clone(),
            active_tab_id,
            visible_tab_id,
        }
    }

    pub(super) fn restore_close_selection(&mut self, restore: Option<&CloseSelectionRestore>) {
        let Some(restore) = restore else {
            return;
        };
        if self.close_selection_state(restore.checkout_id.as_deref()) != restore.after {
            return;
        }
        // The close took the operator to the checkout they were in before
        // (`leave_emptied_checkout`); its tab is back, so they go back too.
        if restore.before.focused_checkout_id != restore.after.focused_checkout_id
            && let Some(checkout_id) = restore.before.focused_checkout_id.as_deref()
            && let Some(workspace_id) = self
                .snapshot
                .navigator
                .workspaces
                .iter()
                .find(|workspace| {
                    workspace
                        .checkouts
                        .iter()
                        .any(|checkout| checkout.id == checkout_id)
                })
                .map(|workspace| workspace.id.clone())
        {
            self.bring_checkout_forward_for_close(&workspace_id, checkout_id);
        }
        if let Some(checkout_id) = restore.checkout_id.as_deref() {
            match restore.before.visible_tab_id.clone() {
                // The close moved the operator, and Herdr with them, to
                // another tab; going back tells Herdr too.
                Some(tab_id) if restore.after.visible_tab_id.as_ref() != Some(&tab_id) => {
                    self.focus_close_alternative(checkout_id, tab_id);
                }
                visible => {
                    if let Some(checkout) = self
                        .snapshot
                        .navigator
                        .workspaces
                        .iter_mut()
                        .flat_map(|workspace| workspace.checkouts.iter_mut())
                        .find(|checkout| checkout.id == checkout_id)
                    {
                        checkout.active_tab_id = restore.before.active_tab_id.clone();
                    }
                    match visible {
                        Some(tab_id) => {
                            self.visible_tab_ids.insert(checkout_id.to_owned(), tab_id);
                        }
                        None => {
                            self.visible_tab_ids.remove(checkout_id);
                        }
                    }
                    self.sync_active_tab_projection();
                }
            }
        }
        self.select_terminal_pane(restore.before.terminal_pane_id.clone());
        self.snapshot.focused.pane_id = restore.before.focused_pane_id.clone();
        self.snapshot.ui_state.selected_pane_id = restore.before.selected_pane_id.clone();
    }

    pub(super) fn clear_close_guards(&mut self, operation: &PendingClose) {
        for pane_id in &operation.pane_ids {
            self.panes_closing.remove(pane_id);
        }
        self.sync_terminal_intents();
    }

    pub(super) fn start_close_capture(
        &mut self,
        target: live::CloseCaptureTarget,
        tab: TabSnapshot,
    ) -> bool {
        let target_id = close_target_id(&target).to_owned();
        // A tree close already admitted keeps room for the local closes it
        // has not started yet, so its all-or-nothing admission holds.
        if self.close_operations.len() + self.tree_close_reserved_slots()
            >= crate::recent_closed::RECENT_CLOSED_LIMIT
        {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: None,
                message: "Resolve or dismiss an earlier close before closing another item".into(),
            }]);
            return true;
        }
        let Some(scope_id) = tab.id.clone() else {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: matches!(&target, live::CloseCaptureTarget::Pane { .. })
                    .then_some(target_id.clone()),
                message: "The close target has no tab identity, so it was not closed".to_owned(),
            }]);
            return true;
        };
        if self
            .close_operations
            .values()
            .any(|operation| operation.scope_id == scope_id || operation.target_id == target_id)
            || self.pane_operations.values().any(|operation| {
                operation.scope_id == scope_id
                    && (matches!(
                        operation.phase.as_str(),
                        "transmitting" | "awaiting_topology"
                    ) || (operation.phase == "queued"
                        && matches!(operation.request, super::operations::GeometryRequest::Task { .. })
                        // The pump has removed the earliest queued Close and
                        // is synchronously starting its turn. A Task still
                        // waiting is behind that Close, so must not veto it.
                        && !self.pumping_geometry))
            })
        {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: matches!(&target, live::CloseCaptureTarget::Pane { .. })
                    .then_some(target_id.clone()),
                message: "This tab already has a close in progress; no second close was sent"
                    .to_owned(),
            }]);
            self.sync_recent_closed_snapshot();
            return false;
        }
        let Some(mut context) = self.close_context(&tab) else {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: matches!(&target, live::CloseCaptureTarget::Pane { .. })
                    .then_some(target_id.clone()),
                message:
                    "The closed item could not be recorded because its local context is incomplete"
                        .into(),
            }]);
            return true;
        };
        context.replacement_shell &=
            matches!(target, live::CloseCaptureTarget::Tab { .. }) || tab.panes.len() == 1;
        // A failed last terminal must remain explicitly closable even when
        // its folder cannot launch a replacement shell. The project registration stays.
        context.replacement_shell &= !tab
            .panes
            .iter()
            .all(|pane| pane.failed_terminal_id.is_some());
        let retain_for_reopen = !self.archive_closes_pane(&target_id)
            && !self
                .snapshot
                .ui_state
                .agent_sleep
                .dormant
                .values()
                .any(|record| {
                    record.old_pane_id == target_id
                        && record.connection_generation == self.live_generation
                        && record.phase == crate::agent_sleep::DormantPhase::Closing
                        && record.close_key.is_none()
                });
        // A shell inside the tab keeps the tab, so the primary workspace keeps
        // its tab too and needs no new one.
        context.replacement_in_tab = retain_for_reopen && self.keeps_named_tab(&target, &tab);
        context.replacement_shell |= context.replacement_in_tab;
        let new_tab_shell = context.replacement_shell && !context.replacement_in_tab;
        if new_tab_shell && !self.admit_agent_tab(&context.checkout_path) {
            return true;
        }
        let Some(live_context) = self.live.as_ref().cloned() else {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: matches!(&target, live::CloseCaptureTarget::Pane { .. })
                    .then_some(target_id.clone()),
                message: "Closing this item requires the local Herdr connection".into(),
            }]);
            return true;
        };
        let request = live::CloseCaptureRequest {
            failed_terminals: tab
                .panes
                .iter()
                .filter(|pane| match &target {
                    live::CloseCaptureTarget::Pane { pane_id } => &pane.id == pane_id,
                    live::CloseCaptureTarget::Tab { .. } => true,
                })
                .filter_map(|pane| {
                    pane.failed_terminal_id
                        .as_ref()
                        .map(|id| (pane.id.clone(), id.clone()))
                })
                .collect(),
            retain_for_reopen,
            key: self.next_recent_closed_key(),
            connection_generation: self.live_generation,
            context,
            panes: self.closed_panes(&tab),
            target,
        };
        if new_tab_shell
            && !self.reserve_agent_effect(
                &request.context.checkout_path,
                &format!("close:{}", request.key),
            )
        {
            return true;
        }
        let pane_ids = Self::close_operation_pane_ids(&request.target, &request.panes);
        let mut scope_pane_ids = tab
            .panes
            .iter()
            .map(|pane| pane.id.clone())
            .collect::<Vec<_>>();
        scope_pane_ids.sort();
        let protected_agent_pane_ids = self
            .snapshot
            .navigator
            .agents
            .iter()
            .filter(|agent| {
                let in_target = match &request.target {
                    live::CloseCaptureTarget::Pane { pane_id } => agent.pane_id == *pane_id,
                    live::CloseCaptureTarget::Tab { .. } => scope_pane_ids
                        .iter()
                        .any(|pane_id| pane_id == &agent.pane_id),
                };
                in_target
                    && (agent.requires_close_confirmation
                        && self.failed_terminal_id(&agent.pane_id).is_none())
            })
            .map(|agent| agent.pane_id.clone())
            .collect::<Vec<_>>();
        let approved_peer_terminals = tab
            .panes
            .iter()
            .filter(|pane| protected_agent_pane_ids.contains(&pane.id))
            .filter_map(|pane| {
                pane.terminal_id.as_ref().map(|terminal| live::ClosePeer {
                    pane_id: pane.id.clone(),
                    terminal_id: terminal.clone(),
                    session: self
                        .snapshot
                        .navigator
                        .agents
                        .iter()
                        .find(|agent| agent.pane_id == pane.id)
                        .and_then(|agent| agent.close_session.clone()),
                })
            })
            .collect();
        let restore_checkout_id = tab.checkout_id.clone();
        let selection_before = self.close_selection_state(restore_checkout_id.as_deref());
        let label = match &request.target {
            live::CloseCaptureTarget::Pane { pane_id } => tab
                .panes
                .iter()
                .find(|pane| pane.id == *pane_id)
                .and_then(|pane| pane.herdr_label.clone().or(pane.terminal_title.clone()))
                .unwrap_or_else(|| pane_id.clone()),
            live::CloseCaptureTarget::Tab { .. } => {
                tab.label.clone().unwrap_or_else(|| "Tab".to_owned())
            }
        };
        let now = unix_milliseconds();
        self.close_operations.insert(
            request.key.clone(),
            PendingClose {
                replacement_effect_started: false,
                replacement_id: None,
                allow_replacement_create: true,
                target_id: target_id.clone(),
                scope_id,
                scope_pane_ids,
                pane_ids: pane_ids.clone(),
                protected_agent_pane_ids,
                approved_peer_terminals,
                label,
                item: None,
                phase: "preparing".to_owned(),
                stage: "capture".to_owned(),
                started_at_unix_ms: now,
                deadline_at_unix_ms: Some(now.saturating_add(CLOSE_STAGE_TIMEOUT_MS)),
                connection_generation: request.connection_generation,
                message: None,
                retryable: true,
                selection_restore: None,
                request: request.clone(),
            },
        );
        self.close_capture_order.push_back(request.key.clone());
        self.bind_dormant_close(&target_id, &request.key);
        // A pane closing beside others is drawn gone at once (D-07).
        self.request_republish();
        if let live::CloseCaptureTarget::Pane { pane_id } = &request.target {
            self.begin_op_timing(
                &super::op_timing::pane_close_op_id(&request.key),
                "pane.close",
                Some(&request.context.tab_id),
                &[pane_id],
            );
        }
        // A reopen waiting for an earlier close meant that item, not this one.
        if let Some(key) = self.reopen_after_close.take() {
            crate::diagnostic!(serde_json::json!({
                "component": "recent_closed",
                "kind": "recent_closed.reopen_dropped",
                "key": key,
            }));
        }
        self.panes_closing.extend(pane_ids);
        // The node refuses keys for a closing pane from now on.
        self.sync_terminal_intents();
        if request.failed_terminals.is_empty() {
            // A sleep's own close (not kept for reopen) is Hide's, not the
            // operator's, so it never takes the screen elsewhere. One that
            // opens a replacement shell keeps its tab (`leaving_tab`), so its
            // checkout is never empty; a shell made inside the tab takes the
            // keyboard once it exists (`ingest_close_replacement`).
            self.move_from_closing_target(&request.target, &tab, request.retain_for_reopen);
        }
        let selection_after = self.close_selection_state(restore_checkout_id.as_deref());
        if selection_before != selection_after
            && let Some(operation) = self.close_operations.get_mut(&request.key)
        {
            operation.selection_restore = Some(CloseSelectionRestore {
                checkout_id: restore_checkout_id,
                before: selection_before,
                after: selection_after,
            });
        }
        self.sync_recent_closed_snapshot();
        if let Err(message) = live::spawn_close_capture(live_context, request.clone()) {
            self.fail_close_operation(
                &request.key,
                format!("The close worker could not start: {message}"),
            );
        }
        true
    }

    /// Keeps the operator on a confirmed alternative as soon as a close is
    /// approved, and only when the closing item is the one in use. The
    /// alternative is chosen from the existing projection, so this never
    /// invents geometry or follows a global Herdr focus change. When the
    /// close leaves the checkout with no tab at all and `may_leave_checkout`
    /// says it is the operator's own, the alternative is the checkout they
    /// were in before (`leave_emptied_checkout`).
    pub(super) fn move_from_closing_target(
        &mut self,
        target: &live::CloseCaptureTarget,
        tab: &TabSnapshot,
        may_leave_checkout: bool,
    ) {
        let Some(checkout_id) = tab.checkout_id.as_deref() else {
            return;
        };
        if self.snapshot.navigator.focused_checkout_id.as_deref() != Some(checkout_id) {
            return;
        }
        if let live::CloseCaptureTarget::Pane { pane_id } = target
            && tab.panes.iter().any(|pane| pane.id != *pane_id)
        {
            let replacement = tab
                .panes
                .iter()
                .map(|pane| pane.id.as_str())
                .filter(|candidate| *candidate != pane_id)
                .find(|candidate| !self.panes_closing.contains(*candidate))
                .map(str::to_owned);
            if self.snapshot.terminal.pane_id.as_deref() == Some(pane_id.as_str()) {
                self.select_terminal_pane(replacement);
            }
            return;
        }
        // The tab's own close, or its only pane's, which takes the tab.
        let Some(tab_id) = tab.id.as_deref() else {
            return;
        };
        let in_use = self
            .visible_tab_ids
            .get(checkout_id)
            .is_some_and(|id| id == tab_id)
            || self
                .snapshot
                .terminal
                .pane_id
                .as_ref()
                .is_some_and(|pane_id| tab.panes.iter().any(|pane| &pane.id == pane_id));
        if !in_use {
            return;
        }
        if let Some(replacement) = self.closing_tab_replacement(checkout_id, tab_id) {
            self.focus_close_alternative(checkout_id, replacement);
        } else if may_leave_checkout && self.checkout_emptied_by_closes(checkout_id) {
            self.leave_emptied_checkout(checkout_id);
        }
    }

    /// Whether every tab of the checkout is one a running close is taking
    /// away, so nothing of it is left to show. A tab off the strip (a
    /// delegated child's) is still a tab the sidebar reaches, so a checkout
    /// holding one is not empty.
    fn checkout_emptied_by_closes(&self, checkout_id: &str) -> bool {
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .find(|checkout| checkout.id == checkout_id)
            .is_some_and(|checkout| !self.has_tab_left(checkout))
    }

    /// Whether the checkout holds a tab no running close is taking away.
    fn has_tab_left(&self, checkout: &CheckoutSnapshot) -> bool {
        checkout.tabs.iter().any(|tab| {
            !tab.id.as_deref().is_some_and(|id| {
                self.close_operations
                    .values()
                    .any(|operation| operation.leaving_tab() == Some(id))
            })
        })
    }

    /// Brings back the checkout of this machine the operator was in before
    /// the one their close just emptied, the way choosing it from the
    /// sidebar does, so the screen never stands on a checkout with nothing
    /// left in it (docs/UI_BEHAVIOR.md, Agent areas). A checkout with no tab
    /// left, or one whose worktree or project is being removed, is no place
    /// to go. With no such checkout, or with another device in front, the
    /// screen stays and the Agent area's empty state shows.
    fn leave_emptied_checkout(&mut self, checkout_id: &str) {
        if !self.device_in_front(self.node.as_str()) {
            return;
        }
        let node = self.node.as_str();
        let previous = self
            .snapshot
            .ui_state
            .recent_checkouts
            .iter()
            .filter(|record| record.device_id == node && record.checkout_id != checkout_id)
            .find_map(|record| {
                self.snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .filter(|workspace| workspace.device_id == node)
                    .find_map(|workspace| {
                        workspace
                            .checkouts
                            .iter()
                            .find(|checkout| checkout.id == record.checkout_id)
                            .filter(|checkout| {
                                self.has_tab_left(checkout)
                                    && !self.removal_in_flight(&workspace.id, &checkout.path)
                            })
                            .map(|checkout| (workspace.id.clone(), checkout.id.clone()))
                    })
            });
        let Some((workspace_id, previous)) = previous else {
            return;
        };
        self.bring_checkout_forward_for_close(&workspace_id, &previous);
    }

    /// Whether this machine's project `workspace_id`, or its worktree at
    /// `checkout_path`, is being removed.
    fn removal_in_flight(&self, workspace_id: &str, checkout_path: &str) -> bool {
        self.workspace_removals_in_flight.contains(workspace_id)
            || self
                .snapshot
                .worktree_removal
                .as_ref()
                .is_some_and(|removal| {
                    removal.device_id.is_none()
                        && removal.checkout_path == checkout_path
                        && matches!(removal.phase.as_str(), "checking" | "closing" | "removing")
                })
    }

    /// A checkout a close moves the operator to, or back from, comes forward
    /// as a sidebar choice does: on its own tab, as the Workspace a reload
    /// opens on, waking what sleeps there.
    fn bring_checkout_forward_for_close(&mut self, workspace_id: &str, checkout_id: &str) {
        let prior = self.focused_visible_tab_id();
        self.focus_checkout(workspace_id, checkout_id);
        self.mark_front_chosen();
        self.agent_sleep_visit(prior.as_deref());
    }

    /// Puts the operator on a tab the way a click on its chip does, so Herdr
    /// is told: its own choice of tab after the close then reads as the
    /// answer to this focus rather than as a move to follow.
    fn focus_close_alternative(&mut self, checkout_id: &str, tab_id: String) {
        let Some(workspace_id) = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .find(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .any(|checkout| checkout.id == checkout_id)
            })
            .map(|workspace| workspace.id.clone())
        else {
            return;
        };
        let prior = self.focused_visible_tab_id();
        self.apply(Event::FocusTab(FocusTabPayload {
            workspace_id,
            checkout_id: checkout_id.to_owned(),
            tab_id,
            focus_device: false,
        }));
        self.agent_sleep_visit(prior.as_deref());
    }

    /// The tab the operator lands on when the tab in use closes: the one its
    /// Workspace's Agent areas show once it has left (`shown_agent_layout`),
    /// so the keyboard lands where the screen does; with no Agent layout, the
    /// checkout's first tab that no close is taking.
    fn closing_tab_replacement(&self, checkout_id: &str, tab_id: &str) -> Option<String> {
        let (device_id, checkout) = self
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
            .find(|(_, checkout)| checkout.id == checkout_id)?;
        let key = (device_id.clone(), checkout.path.clone());
        if let Some(layout) = self.agent_layout_of(&key) {
            return self
                .shown_agent_layout(&key, layout)
                .active()
                .filter(|id| *id != tab_id)
                .map(str::to_owned);
        }
        checkout
            .tabs
            .iter()
            .filter_map(|candidate| candidate.id.as_deref())
            .filter(|candidate| *candidate != tab_id)
            .find(|candidate| {
                !self
                    .close_operations
                    .values()
                    .any(|operation| operation.target_id == *candidate)
            })
            .map(str::to_owned)
    }

    pub(super) fn fail_close_operation(&mut self, key: &str, message: String) {
        self.op_timings
            .finish(&super::op_timing::pane_close_op_id(key), "failed");
        let Some(operation) = self.close_operations.get_mut(key) else {
            return;
        };
        operation.phase = "failed".to_owned();
        operation.stage = "capture".to_owned();
        operation.message = Some(if operation.request.context.replacement_shell {
            "The close could not be prepared. Nothing was closed; retry when the connection is ready.".to_owned()
        } else {
            format!("Restore information was not prepared; the item was not closed: {message}")
        });
        operation.retryable = true;
        operation.deadline_at_unix_ms = None;
        let operation = operation.clone();
        if !operation.replacement_effect_started {
            self.finish_agent_effect(
                &operation.request.context.checkout_path,
                &format!("close:{key}"),
            );
        }
        self.clear_close_guards(&operation);
        // Nothing was closed, so the tab returns to its place and so does the
        // operator, as after a refusal.
        self.restore_close_selection(operation.selection_restore.as_ref());
        self.set_reopen_notices(vec![live::ReopenNotice {
            pane_id: matches!(
                &operation.request.target,
                live::CloseCaptureTarget::Pane { .. }
            )
            .then_some(operation.target_id.clone()),
            message: operation.message.clone().unwrap_or_else(|| message.clone()),
        }]);
        self.push_diagnostic(
            "recent_closed.capture_failed",
            format!("{}: {}", operation.request.key, message),
        );
        self.settle_dormant_close_result(key);
        self.promote_close_reservations();
        self.sync_recent_closed_snapshot();
    }

    pub(super) fn ensure_pending_close_from_request(
        &mut self,
        request: &live::CloseCaptureRequest,
    ) {
        if self.close_operations.contains_key(&request.key) {
            return;
        }
        let target_id = close_target_id(&request.target).to_owned();
        let pane_ids = Self::close_operation_pane_ids(&request.target, &request.panes);
        let now = unix_milliseconds();
        self.close_operations.insert(
            request.key.clone(),
            PendingClose {
                replacement_effect_started: false,
                replacement_id: None,
                allow_replacement_create: true,
                request: request.clone(),
                target_id,
                scope_id: request.context.tab_id.clone(),
                scope_pane_ids: {
                    let mut ids = request.context.pane_ids_before_close.clone();
                    ids.sort();
                    ids
                },
                pane_ids,
                protected_agent_pane_ids: Vec::new(),
                approved_peer_terminals: Vec::new(),
                label: request.context.tab_label.clone(),
                item: None,
                phase: "preparing".to_owned(),
                stage: "capture".to_owned(),
                started_at_unix_ms: now,
                deadline_at_unix_ms: Some(now.saturating_add(CLOSE_STAGE_TIMEOUT_MS)),
                connection_generation: request.connection_generation,
                message: None,
                retryable: true,
                selection_restore: None,
            },
        );
        if !self
            .close_capture_order
            .iter()
            .any(|key| key == &request.key)
        {
            self.close_capture_order.push_back(request.key.clone());
        }
    }

    pub(crate) fn ingest_close_capture_result(
        &mut self,
        request: &live::CloseCaptureRequest,
        result: Result<live::CloseCaptureOutcome, String>,
    ) -> (bool, Vec<live::CloseEffectRequest>) {
        self.with_close_geometry(|runtime| runtime.apply_close_capture_result(request, result))
    }

    fn apply_close_capture_result(
        &mut self,
        request: &live::CloseCaptureRequest,
        result: Result<live::CloseCaptureOutcome, String>,
    ) -> (bool, Vec<live::CloseEffectRequest>) {
        if !request.retain_for_reopen && !self.close_operations.contains_key(&request.key) {
            return (false, Vec::new());
        }
        self.ensure_pending_close_from_request(request);
        let Some(operation) = self.close_operations.get(&request.key).cloned() else {
            return (false, Vec::new());
        };
        if operation.connection_generation != request.connection_generation
            || request.connection_generation != self.live_generation
            || !matches!(operation.phase.as_str(), "preparing" | "capturing")
        {
            return (false, Vec::new());
        }
        if operation
            .deadline_at_unix_ms
            .is_some_and(|deadline| deadline <= unix_milliseconds())
        {
            self.fail_close_operation(
                &request.key,
                "the restore capture result arrived after its deadline".to_owned(),
            );
            return (true, Vec::new());
        }
        if let Some(message) = self.close_precondition_failure(&operation) {
            self.cancel_close_before_effect(&request.key, message);
            return (true, Vec::new());
        }
        let mut effects = Vec::new();
        match result {
            Ok(outcome) => {
                let Some(operation) = self.close_operations.get_mut(&request.key) else {
                    return (false, Vec::new());
                };
                operation.item = outcome.item;
                if self.defer_dormant_close(&request.key) {
                    if let Some(operation) = self.close_operations.get_mut(&request.key) {
                        // Keep the existing preparing reservation while the
                        // one state writer establishes the durable sleep intent.
                        operation.phase = "preparing".into();
                        operation.stage = "durable_sleep".into();
                    }
                    self.persist_ui_state();
                } else if let Some(effect) = self.prepare_captured_close_effect(&request.key) {
                    effects.push(effect);
                }
            }
            Err(message) => self.fail_close_operation(&request.key, message),
        }
        self.sync_recent_closed_snapshot();
        (true, effects)
    }

    /// Both ordinary capture and a sleep save receipt enter the existing
    /// close-effect worker through this one reservation transition.
    pub(super) fn prepare_captured_close_effect(
        &mut self,
        key: &str,
    ) -> Option<live::CloseEffectRequest> {
        let operation = self.close_operations.get_mut(key)?;
        let request = &operation.request;
        operation.replacement_effect_started |= request.context.replacement_shell;
        operation.phase = "transmitting".into();
        operation.stage = "close_request".into();
        operation.message = None;
        operation.retryable = false;
        operation.deadline_at_unix_ms = Some(unix_milliseconds().saturating_add(
            if request.context.replacement_shell {
                3 * CLOSE_STAGE_TIMEOUT_MS
            } else {
                CLOSE_STAGE_TIMEOUT_MS
            },
        ));
        let effect = live::CloseEffectRequest {
            safety: (!request.failed_terminals.is_empty()).then(|| live::CloseSafety {
                approved_peers: operation.approved_peer_terminals.clone(),
                pane_ids: operation.scope_pane_ids.clone(),
                deadline: std::time::Instant::now()
                    + std::time::Duration::from_millis(
                        operation
                            .deadline_at_unix_ms
                            .unwrap_or_default()
                            .saturating_sub(unix_milliseconds())
                            .min(CLOSE_STAGE_TIMEOUT_MS),
                    ),
            }),
            failed_terminals: request.failed_terminals.clone(),
            retain_for_reopen: request.retain_for_reopen,
            allow_replacement_create: operation.allow_replacement_create,
            key: request.key.clone(),
            connection_generation: request.connection_generation,
            target: request.target.clone(),
            replacement: (request.context.replacement_shell
                && (matches!(request.target, live::CloseCaptureTarget::Tab { .. })
                    || request.panes.len() == 1))
                .then(|| request.context.clone()),
        };
        self.push_diagnostic(
            "recent_closed.reserved",
            format!("Reserved user close {key} before its external effect"),
        );
        Some(effect)
    }

    pub(crate) fn ingest_close_effect_result(
        &mut self,
        request: &live::CloseEffectRequest,
        result: Result<(), hide_herdr_client::ApiError>,
    ) -> bool {
        self.with_close_geometry(|runtime| runtime.apply_close_effect_result(request, result))
    }

    fn apply_close_effect_result(
        &mut self,
        request: &live::CloseEffectRequest,
        result: Result<(), hide_herdr_client::ApiError>,
    ) -> bool {
        if !request.retain_for_reopen && !self.close_operations.contains_key(&request.key) {
            return false;
        }
        if !self.close_operations.contains_key(&request.key) {
            // A result can outlive the runtime that created its reservation.
            // Rehydrate that reservation without putting it back on the
            // confirmed stack, then let the same identity checks and
            // topology gate handle the result.
            let tab_id = match &request.target {
                live::CloseCaptureTarget::Pane { pane_id } => pane_id.clone(),
                live::CloseCaptureTarget::Tab { tab_id } => tab_id.clone(),
            };
            self.ensure_pending_close_from_request(&live::CloseCaptureRequest {
                failed_terminals: request.failed_terminals.clone(),
                retain_for_reopen: request.retain_for_reopen,
                key: request.key.clone(),
                connection_generation: request.connection_generation,
                context: ClosedContext {
                    workspace_id: String::new(),
                    workspace_label: String::new(),
                    workspace_ids_before_close: Vec::new(),
                    tab_ids_before_close: Vec::new(),
                    pane_ids_before_close: Vec::new(),
                    checkout_id: String::new(),
                    checkout_path: String::new(),
                    tab_id,
                    tab_label: request.key.clone(),
                    tab_index: 0,
                    agent_area: None,
                    agent_group: None,
                    replacement_shell: false,
                    replacement_in_tab: false,
                    tab_name: ClosedTabName::Unnamed,
                },
                panes: Vec::new(),
                target: request.target.clone(),
            });
            let item = self
                .recent_closed
                .iter()
                .position(|item| item.key() == request.key)
                .and_then(|index| self.recent_closed.remove(index));
            if let Some(operation) = self.close_operations.get_mut(&request.key) {
                operation.item = item;
                operation.phase = "transmitting".to_owned();
                operation.stage = "close_request".to_owned();
                operation.retryable = false;
                operation.deadline_at_unix_ms =
                    Some(unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS));
            }
        }
        let Some(operation) = self.close_operations.get(&request.key).cloned() else {
            return false;
        };
        if operation.connection_generation != request.connection_generation
            || request.connection_generation != self.live_generation
            || close_target_id(&operation.request.target) != close_target_id(&request.target)
            || operation.phase != "transmitting"
        {
            return false;
        }
        let result = if operation
            .deadline_at_unix_ms
            .is_some_and(|deadline| deadline <= unix_milliseconds())
        {
            Err(hide_herdr_client::ApiError::Transport(
                "close result arrived after its deadline".to_owned(),
            ))
        } else {
            result
        };
        let mut schedule_status_check = false;
        let timing_id = super::op_timing::pane_close_op_id(&request.key);
        match &result {
            Ok(()) => self.op_timings.stamp(
                &timing_id,
                super::op_timing::Stage::HerdrAck,
                std::time::Instant::now(),
            ),
            Err(hide_herdr_client::ApiError::Remote { .. }) => {
                self.op_timings.finish(&timing_id, "refused")
            }
            Err(_) => self.op_timings.finish(&timing_id, "unknown"),
        }
        match result {
            Ok(()) => {
                if let Some(operation) = self.close_operations.get_mut(&request.key) {
                    operation.phase = "awaiting_topology".to_owned();
                    operation.stage = "topology".to_owned();
                    operation.message = Some(
                        "Close was accepted; waiting for Herdr to confirm the target is gone"
                            .to_owned(),
                    );
                    operation.retryable = false;
                    operation.deadline_at_unix_ms =
                        Some(unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS));
                }
                self.push_diagnostic(
                    "recent_closed.captured",
                    format!("Captured user close {}", request.key),
                );
            }
            Err(hide_herdr_client::ApiError::Remote { code, message }) => {
                if let Some(operation) = self.close_operations.get_mut(&request.key) {
                    operation.phase = "refused".to_owned();
                    operation.stage = if code == "replacement_failed" {
                        "replacement"
                    } else {
                        "close_request"
                    }
                    .to_owned();
                    operation.message = Some(if !operation.request.context.replacement_shell {
                        "The item was not closed. Check its current state before closing it again."
                    } else if code == "replacement_failed" {
                        "The replacement shell is unconfirmed; its place stays reserved. Nothing was closed. Retry when the connection is ready."
                    } else {
                        "The item was not closed. The replacement shell remains available; retry the close."
                    }.to_owned());
                    operation.retryable = operation.request.context.replacement_shell;
                    operation.deadline_at_unix_ms = None;
                }
                let operation = self.close_operations.get(&request.key).cloned();
                if let Some(operation) = operation.as_ref() {
                    self.clear_close_guards(operation);
                    self.restore_close_selection(operation.selection_restore.as_ref());
                }
                self.set_reopen_notices(vec![live::ReopenNotice {
                    pane_id: matches!(&request.target, live::CloseCaptureTarget::Pane { .. })
                        .then_some(close_target_id(&request.target).to_owned()),
                    message: operation
                        .and_then(|operation| operation.message)
                        .unwrap_or_else(|| "The item was not closed. Check its current state before closing it again.".to_owned()),
                }]);
                self.push_diagnostic(
                    "recent_closed.close_failed",
                    format!("{}: {code}: {message}", request.key),
                );
            }
            Err(error) => {
                if let Some(operation) = self.close_operations.get_mut(&request.key) {
                    operation.phase = "unknown".to_owned();
                    operation.stage = "status_check".to_owned();
                    operation.message =
                        Some("Close result is unknown; check status before retrying".to_owned());
                    operation.retryable = true;
                    operation.deadline_at_unix_ms =
                        Some(unix_milliseconds().saturating_add(CLOSE_STAGE_TIMEOUT_MS));
                }
                self.set_reopen_notices(vec![live::ReopenNotice {
                    pane_id: matches!(&request.target, live::CloseCaptureTarget::Pane { .. })
                        .then_some(close_target_id(&request.target).to_owned()),
                    message: format!(
                        "Close result is unknown; the reopen reservation was kept: {error}"
                    ),
                }]);
                self.push_diagnostic(
                    "recent_closed.close_unconfirmed",
                    format!("{}: {error}", request.key),
                );
                schedule_status_check = true;
            }
        }
        self.settle_dormant_close_result(&request.key);
        if schedule_status_check {
            self.start_close_status_check(&request.key);
        }
        self.promote_close_reservations();
        self.sync_recent_closed_snapshot();
        true
    }

    pub(super) fn promote_close_reservations(&mut self) -> bool {
        let mut changed = false;
        let mut queued_close_completed = false;
        while let Some(key) = self.close_capture_order.front().cloned() {
            let Some(operation) = self.close_operations.get(&key).cloned() else {
                self.close_capture_order.pop_front();
                changed = true;
                continue;
            };
            if !matches!(operation.phase.as_str(), "completed" | "failed" | "refused") {
                break;
            }
            if operation.request.context.replacement_shell
                && matches!(operation.phase.as_str(), "failed" | "refused")
            {
                break;
            }
            self.close_capture_order.pop_front();
            self.close_operations.remove(&key);
            self.clear_close_guards(&operation);
            // Without a captured item the reopen would reach an older close.
            queued_close_completed |= self.reopen_after_close.as_deref() == Some(key.as_str())
                && operation.request.retain_for_reopen
                && operation.phase == "completed"
                && operation.item.is_some();
            if operation.phase == "completed"
                && operation.request.retain_for_reopen
                && let Some(item) = operation.item
            {
                push_bounded(&mut self.recent_closed, item);
            }
            changed = true;
        }
        if changed {
            self.sync_recent_closed_snapshot();
            // A close waiting behind one that just left its tab takes its turn.
            self.pump_geometry_queues();
        }
        self.release_queued_reopen(queued_close_completed);
        changed
    }

    /// Runs the reopen that waited for a close once that close is confirmed
    /// and nothing newer stands in front of it; a close that ended any other
    /// way, or a device now in front, drops it.
    fn release_queued_reopen(&mut self, close_completed: bool) {
        let Some(key) = self.reopen_after_close.clone() else {
            return;
        };
        if self.close_operations.contains_key(&key) {
            return;
        }
        self.reopen_after_close = None;
        let runs = close_completed && self.reopen_device() == self.node.as_str();
        crate::diagnostic!(serde_json::json!({
            "component": "recent_closed",
            "kind": if runs { "recent_closed.reopen_released" } else { "recent_closed.reopen_dropped" },
            "key": key,
        }));
        if runs {
            self.reopen_closed();
        } else {
            self.sync_recent_closed_snapshot();
        }
    }

    /// Completes a close only after a fresh topology says the requested
    /// target is absent. A capture failure with an absent target is kept as a
    /// completed, no-item operation, so Hide does not invent restore data for
    /// an external close.
    pub(super) fn mark_close_topology_confirmed(&mut self, key: &str) -> bool {
        let Some(operation) = self.close_operations.get(key).cloned() else {
            return false;
        };
        if matches!(operation.phase.as_str(), "completed" | "failed" | "refused") {
            return false;
        }
        self.confirm_dormant_close(&operation);
        self.op_timings.stamp(
            &super::op_timing::pane_close_op_id(key),
            super::op_timing::Stage::Applied,
            std::time::Instant::now(),
        );
        if let Some(current) = self.close_operations.get_mut(key) {
            let had_restore = current.item.is_some();
            current.phase = "completed".to_owned();
            current.stage = "topology".to_owned();
            current.message = (!had_restore).then(|| {
                "The item was already closed; Hide synchronized this view without creating a reopen entry"
                    .to_owned()
            });
            current.retryable = false;
            current.deadline_at_unix_ms = None;
        }
        self.clear_close_guards(&operation);
        if operation.item.is_none() {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: matches!(
                    &operation.request.target,
                    live::CloseCaptureTarget::Pane { .. }
                )
                .then_some(operation.target_id),
                message: "The item was already closed; Hide synchronized this view without creating a reopen entry".to_owned(),
            }]);
        } else {
            self.snapshot.recent_closed.notices.clear();
        }
        self.push_diagnostic(
            "recent_closed.topology_confirmed",
            format!(
                "Confirmed close {} by fresh topology; restore data {}",
                key,
                if operation.item.is_some() {
                    "is reserved"
                } else {
                    "was unavailable"
                }
            ),
        );
        true
    }

    pub(super) fn observe_close_topology(&mut self, payload: &SessionSnapshotPayload) -> bool {
        let mut changed = false;
        for key in self.close_capture_order.clone() {
            let Some(operation) = self.close_operations.get(&key).cloned() else {
                continue;
            };
            let target_present = Self::close_operation_target_present(
                payload,
                &operation.request.target,
                &operation.scope_id,
            );
            let scope_changed = !operation.scope_pane_ids.is_empty()
                && payload
                    .layouts
                    .iter()
                    .find(|layout| layout.tab_id == operation.scope_id)
                    .is_some_and(|layout| {
                        let mut pane_ids = layout
                            .panes
                            .iter()
                            .map(|pane| pane.pane_id.clone())
                            .collect::<Vec<_>>();
                        pane_ids.sort();
                        pane_ids != operation.scope_pane_ids
                    });
            if matches!(operation.phase.as_str(), "preparing" | "capturing")
                && (!target_present || scope_changed)
            {
                self.cancel_close_before_effect(
                    &key,
                    if target_present {
                        "the tab or pane set changed while restore information was being prepared"
                            .to_owned()
                    } else {
                        "the target disappeared while restore information was being prepared"
                            .to_owned()
                    },
                );
                changed = true;
            } else if !target_present {
                changed |= self.mark_close_topology_confirmed(&key);
            }
        }
        changed |= self.promote_close_reservations();
        if changed {
            self.sync_recent_closed_snapshot();
        }
        changed
    }

    pub(super) fn start_close_status_check(&mut self, key: &str) -> bool {
        self.start_close_status_checks(&[key.to_owned()])
    }

    /// Starts one read for all currently unknown local closes. All of those
    /// reservations share the same Herdr session, so a status check is a
    /// single authoritative snapshot with a fan-out result rather than one
    /// request per pane or tab.
    pub(super) fn start_close_status_checks(&mut self, requested_keys: &[String]) -> bool {
        if requested_keys.is_empty()
            || requested_keys
                .iter()
                .any(|key| self.close_status_checks_in_flight.contains(key))
        {
            return false;
        }
        let requested_unknown = requested_keys.iter().any(|key| {
            self.close_operations
                .get(key)
                .is_some_and(|operation| operation.phase == "unknown")
        });
        if !requested_unknown {
            return false;
        }

        let requests = self
            .close_capture_order
            .iter()
            .filter_map(|key| {
                let operation = self.close_operations.get(key)?;
                (operation.phase == "unknown" && !self.close_status_checks_in_flight.contains(key))
                    .then(|| live::CloseStatusCheckRequest {
                        key: key.clone(),
                        target: operation.request.target.clone(),
                    })
            })
            .collect::<Vec<_>>();
        if requests.is_empty() {
            return false;
        }
        let request_keys = requests
            .iter()
            .map(|request| request.key.clone())
            .collect::<Vec<_>>();
        let Some(context) = self.live.as_ref().cloned() else {
            for key in &request_keys {
                if let Some(operation) = self.close_operations.get_mut(key) {
                    operation.message = Some(
                        "Close result is unknown; reconnect Herdr before checking status"
                            .to_owned(),
                    );
                    operation.retryable = true;
                    operation.deadline_at_unix_ms = None;
                }
            }
            self.sync_recent_closed_snapshot();
            return true;
        };

        for key in &request_keys {
            self.close_status_checks_in_flight.insert(key.clone());
            if let Some(operation) = self.close_operations.get_mut(key) {
                operation.message = Some("Checking the current topology".to_owned());
                operation.stage = "status_check".to_owned();
                operation.retryable = true;
            }
        }
        if let Err(message) =
            live::spawn_close_status_checks(context, self.live_generation, requests)
        {
            for key in &request_keys {
                self.close_status_checks_in_flight.remove(key);
                if let Some(operation) = self.close_operations.get_mut(key) {
                    operation.message = Some(format!(
                        "Close result is unknown; status check could not start: {message}"
                    ));
                    operation.retryable = true;
                    operation.deadline_at_unix_ms = None;
                }
            }
            let notices = request_keys
                .iter()
                .filter_map(|key| {
                    let operation = self.close_operations.get(key)?;
                    Some(live::ReopenNotice {
                        pane_id: matches!(
                            &operation.request.target,
                            live::CloseCaptureTarget::Pane { .. }
                        )
                        .then_some(operation.target_id.clone()),
                        message: operation.message.clone().unwrap_or_default(),
                    })
                })
                .collect();
            self.set_reopen_notices(notices);
            self.sync_recent_closed_snapshot();
            return true;
        }
        self.sync_recent_closed_snapshot();
        true
    }

    pub(crate) fn ingest_close_status_results(
        &mut self,
        connection_generation: u64,
        requests: &[live::CloseStatusCheckRequest],
        result: Result<SessionSnapshotPayload, SessionFetchError>,
    ) -> bool {
        self.with_close_geometry(|runtime| {
            runtime.apply_close_status_results(connection_generation, requests, result)
        })
    }

    fn apply_close_status_results(
        &mut self,
        connection_generation: u64,
        requests: &[live::CloseStatusCheckRequest],
        result: Result<SessionSnapshotPayload, SessionFetchError>,
    ) -> bool {
        let valid_requests = requests
            .iter()
            .filter_map(|request| {
                let operation = self.close_operations.get(&request.key)?;
                if connection_generation != self.live_generation
                    || operation.connection_generation != connection_generation
                    || close_target_id(&operation.request.target)
                        != close_target_id(&request.target)
                {
                    return None;
                }
                Some((
                    request.key.clone(),
                    request.target.clone(),
                    operation.scope_id.clone(),
                ))
            })
            .collect::<Vec<_>>();
        if valid_requests.is_empty() {
            for request in requests {
                self.close_status_checks_in_flight.remove(&request.key);
            }
            return false;
        }

        let mut changed = false;
        match result {
            Ok(payload) => {
                let target_presence = valid_requests
                    .iter()
                    .map(|(key, target, scope_id)| {
                        (
                            key.clone(),
                            Self::close_operation_target_present(&payload, target, scope_id),
                        )
                    })
                    .collect::<Vec<_>>();
                // The status response is itself a fresh session projection.
                // Apply it before classifying the reservation so the sidebar
                // cannot retain a closed target until an unrelated event.
                changed |= self.ingest_session_with_catalog(Ok(payload), None);
                let mut notices = Vec::new();
                for (key, target_present) in target_presence {
                    if target_present && let Some(operation) = self.close_operations.get_mut(&key) {
                        operation.phase = "unknown".to_owned();
                        operation.stage = "status_check".to_owned();
                        operation.message = Some(
                            "The target is still present; no close was resent. Check status again after resolving the current work"
                                .to_owned(),
                        );
                        operation.retryable = true;
                        operation.deadline_at_unix_ms = None;
                        notices.push(live::ReopenNotice {
                            pane_id: matches!(
                                &operation.request.target,
                                live::CloseCaptureTarget::Pane { .. }
                            )
                            .then_some(operation.target_id.clone()),
                            message: operation.message.clone().unwrap_or_default(),
                        });
                        changed = true;
                    }
                    self.push_diagnostic(
                        "recent_closed.status_checked",
                        format!("Checked close {key}; target_present={target_present}"),
                    );
                }
                if !notices.is_empty() {
                    self.set_reopen_notices(notices);
                }
            }
            Err(error) => {
                let mut notices = Vec::new();
                for (key, _, _) in &valid_requests {
                    if let Some(operation) = self.close_operations.get_mut(key) {
                        operation.phase = "unknown".to_owned();
                        operation.stage = "status_check".to_owned();
                        operation.message = Some(format!(
                            "Close result is unknown; status check failed: {}",
                            error.message()
                        ));
                        operation.retryable = true;
                        operation.deadline_at_unix_ms = None;
                        notices.push(live::ReopenNotice {
                            pane_id: matches!(
                                &operation.request.target,
                                live::CloseCaptureTarget::Pane { .. }
                            )
                            .then_some(operation.target_id.clone()),
                            message: operation.message.clone().unwrap_or_default(),
                        });
                        changed = true;
                    }
                    self.push_diagnostic(
                        "recent_closed.status_check_failed",
                        format!("{key}: {}", error.message()),
                    );
                }
                self.set_reopen_notices(notices);
            }
        }
        for request in requests {
            self.close_status_checks_in_flight.remove(&request.key);
        }
        changed |= self.promote_close_reservations();
        self.sync_recent_closed_snapshot();
        changed || !valid_requests.is_empty()
    }

    pub(crate) fn check_close_status(&mut self, key: &str) -> bool {
        let Some(operation) = self.close_operations.get(key) else {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: None,
                message: "That close is no longer pending".to_owned(),
            }]);
            return true;
        };
        if operation.phase != "unknown" {
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: matches!(
                    &operation.request.target,
                    live::CloseCaptureTarget::Pane { .. }
                )
                .then_some(operation.target_id.clone()),
                message: "This close is still awaiting its normal confirmation".to_owned(),
            }]);
            return true;
        }
        self.start_close_status_check(key)
    }

    pub(super) fn expire_close_operations(&mut self, now_unix_ms: u64) -> bool {
        let mut check = Vec::new();
        let mut changed = false;
        for key in self.close_capture_order.clone() {
            let Some(operation) = self.close_operations.get(&key).cloned() else {
                continue;
            };
            if self.close_status_checks_in_flight.contains(&key) {
                continue;
            }
            let Some(deadline) = operation.deadline_at_unix_ms else {
                continue;
            };
            if deadline > now_unix_ms {
                continue;
            }
            match operation.phase.as_str() {
                "preparing" | "capturing" => {
                    self.fail_close_operation(
                        &key,
                        "the restore capture deadline expired".to_owned(),
                    );
                    changed = true;
                }
                "transmitting" | "awaiting_topology" => {
                    if let Some(operation) = self.close_operations.get_mut(&key) {
                        operation.phase = "unknown".to_owned();
                        operation.stage = "status_check".to_owned();
                        operation.message = Some(
                            "Close result is unknown; checking the current topology".to_owned(),
                        );
                        operation.retryable = true;
                        operation.deadline_at_unix_ms =
                            Some(now_unix_ms.saturating_add(CLOSE_STAGE_TIMEOUT_MS));
                    }
                    check.push(key);
                    changed = true;
                }
                "unknown" => {
                    if let Some(operation) = self.close_operations.get_mut(&key) {
                        operation.message = Some(
                            "Result check needed; no mutation was resent. Use Check status to retry the read-only check"
                                .to_owned(),
                        );
                        operation.deadline_at_unix_ms = None;
                    }
                    changed = true;
                }
                _ => {}
            }
        }
        if !check.is_empty() {
            changed |= self.start_close_status_checks(&check);
        }
        changed |= self.promote_close_reservations();
        if changed {
            self.sync_recent_closed_snapshot();
        }
        changed
    }

    pub(super) fn reopen_closed(&mut self) -> bool {
        if let Some(key) = self.reopen_in_flight.as_deref() {
            crate::diagnostic!(serde_json::json!({
                "component": "recent_closed",
                "kind": "recent_closed.reopen_in_flight",
                "key": key,
            }));
            return false;
        }
        if self.reopen_device() == self.node.as_str()
            && let Some(key) = self.close_capture_order.back().cloned()
            && let Some(operation) = self.close_operations.get(&key)
            && operation.request.retain_for_reopen
        {
            // The tab already left the screen, so a reopen pressed right
            // after the close is the operator's intent, not a mistake: it
            // waits for that close and runs when Herdr confirms it.
            if operation.settling() {
                crate::diagnostic!(serde_json::json!({
                    "component": "recent_closed",
                    "kind": "recent_closed.reopen_queued",
                    "key": key,
                    "phase": operation.phase,
                }));
                self.reopen_after_close = Some(key);
                self.sync_recent_closed_snapshot();
                return true;
            }
            crate::diagnostic!(serde_json::json!({
                "component": "recent_closed",
                "kind": "recent_closed.reopen_waits_for_close",
                "key": key,
                "phase": operation.phase,
            }));
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: matches!(
                    &operation.request.target,
                    live::CloseCaptureTarget::Pane { .. }
                )
                .then_some(operation.target_id.clone()),
                message: if operation.phase == "unknown" {
                    "Close result is unknown; check status before reopening".to_owned()
                } else {
                    "Close is still being confirmed; reopening is temporarily unavailable"
                        .to_owned()
                },
            }]);
            self.sync_recent_closed_snapshot();
            return true;
        }
        let Some(item) = self.reopenable().cloned() else {
            crate::diagnostic!(serde_json::json!({
                "component": "recent_closed",
                "kind": "recent_closed.nothing_to_reopen",
                "device_id": self.reopen_device(),
            }));
            return false;
        };
        if matches!(item, ClosedItem::Draft { .. }) {
            return self.reopen_agent_draft(item);
        }
        if let ClosedItem::File {
            workspace_id,
            checkout_id,
            path,
            ..
        } = &item
            && let Some(tab_id) = self.snapshot.editor.tabs.iter().find_map(|tab| {
                (tab.kind == EditorTabKind::File
                    && tab.workspace_id == *workspace_id
                    && tab.checkout_id == *checkout_id
                    && tab.path == *path)
                    .then(|| tab.id.clone())
            })
        {
            self.consume_recent_closed(item.key());
            match self.focus_editor_tab_context(&tab_id) {
                Ok(()) => self.snapshot.recent_closed.notices.clear(),
                Err(message) => self.set_reopen_notices(vec![live::ReopenNotice {
                    pane_id: None,
                    message: format!(
                        "The file reopened, but its project context was unavailable: {message}"
                    ),
                }]),
            }
            self.sync_recent_closed_snapshot();
            return true;
        }
        let (workspace_exists, tab_exists, fallback_pane_id) = match &item {
            ClosedItem::Pane {
                context, placement, ..
            } => {
                let tab = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .flat_map(|checkout| checkout.tabs.iter())
                    .find(|tab| tab.id.as_deref() == Some(context.tab_id.as_str()));
                let fallback = tab.and_then(|tab| {
                    placement
                        .neighbor_pane_id
                        .as_ref()
                        .filter(|neighbor| tab.panes.iter().any(|pane| pane.id == **neighbor))
                        .cloned()
                        .or_else(|| tab.panes.first().map(|pane| pane.id.clone()))
                });
                (
                    self.snapshot.navigator.workspaces.iter().any(|workspace| {
                        workspace
                            .session_workspace_ids
                            .contains(&context.workspace_id)
                    }),
                    tab.is_some(),
                    fallback,
                )
            }
            ClosedItem::Tab { context, .. } => (
                self.snapshot.navigator.workspaces.iter().any(|workspace| {
                    workspace
                        .session_workspace_ids
                        .contains(&context.workspace_id)
                }),
                false,
                None,
            ),
            ClosedItem::File { .. } => (true, true, None),
            ClosedItem::Draft { .. } => unreachable!("draft reopen is local"),
        };
        // A tab made again lands in its checkout's owner (D-16), found by the
        // checkout the item was closed from, else by its folder.
        let owner = match &item {
            ClosedItem::Pane { context, .. } | ClosedItem::Tab { context, .. } => {
                let located = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .filter(|workspace| workspace.remote_target_id.is_none())
                    .find_map(|workspace| {
                        workspace
                            .checkouts
                            .iter()
                            .find(|checkout| checkout.id == context.checkout_id)
                            .or_else(|| {
                                workspace
                                    .checkouts
                                    .iter()
                                    .find(|checkout| checkout.path == context.checkout_path)
                            })
                            .map(|checkout| (workspace, checkout))
                    });
                let Some((workspace, checkout)) = located else {
                    self.set_reopen_notices(vec![live::ReopenNotice {
                        pane_id: None,
                        message: format!(
                            "{} can't reopen: its checkout {} is no longer listed",
                            item.label(),
                            context.checkout_path
                        ),
                    }]);
                    self.sync_recent_closed_snapshot();
                    return true;
                };
                Some(super::projects::owner_open(
                    workspace,
                    checkout,
                    self.node.as_str(),
                    &self.node,
                ))
            }
            ClosedItem::File { .. } | ClosedItem::Draft { .. } => None,
        };
        // With View areas the reopened file takes a display, so a Workspace
        // at its display cap refuses it here and the item stays.
        if self.separate_view_areas()
            && let ClosedItem::File {
                workspace_id,
                checkout_id,
                path,
                ..
            } = &item
            && let Some(workspace) = self.workspace_key(workspace_id, checkout_id)
            && !self.admit_view_open(
                &workspace,
                path,
                crate::view_layout::DisplayKind::File,
                None,
                false,
                false,
            )
        {
            return true;
        }
        let admission = if !tab_exists && self.reopen_device() == self.node.as_str() {
            match &item {
                ClosedItem::Pane { context, .. } | ClosedItem::Tab { context, .. } => Some((
                    context.checkout_path.clone(),
                    format!("reopen:{}", item.key()),
                )),
                ClosedItem::File { .. } | ClosedItem::Draft { .. } => None,
            }
        } else {
            None
        };
        let prior_admission = admission
            .as_ref()
            .is_some_and(|(path, claim)| self.has_agent_effect(path, claim));
        if admission
            .as_ref()
            .is_some_and(|(path, claim)| !self.reserve_agent_effect(path, claim))
        {
            return true;
        }
        let key = item.key().to_owned();
        self.reopen_in_flight = Some(key.clone());
        self.set_reopen_notices(vec![live::ReopenNotice {
            pane_id: fallback_pane_id.clone(),
            message: format!("Reopening {}…", item.label()),
        }]);
        self.sync_recent_closed_snapshot();
        let request = live::ReopenRequest {
            item,
            workspace_exists,
            tab_exists,
            fallback_pane_id,
            owner,
            codex_daemon: self.codex_daemon(self.node.as_str()),
        };
        let spawned = if let ClosedItem::File {
            workspace_id,
            checkout_id,
            ..
        } = &request.item
        {
            let located = self
                .document_root(workspace_id, checkout_id)
                .and_then(|root| Ok((self.node_link(&root.device_id)?, root)));
            located.and_then(|(channel, root)| {
                self.worker_context
                    .as_ref()
                    .cloned()
                    .ok_or_else(|| "the file worker is unavailable".to_owned())
                    .and_then(|worker| {
                        live::spawn_file_reopen(
                            worker.runtime,
                            worker.notifier,
                            request,
                            root,
                            channel,
                        )
                    })
            })
        } else {
            self.live
                .as_ref()
                .cloned()
                .ok_or_else(|| {
                    "the local Herdr connection is unavailable; retry when it returns".to_owned()
                })
                .and_then(|context| live::spawn_reopen(context, request))
        };
        if let Err(message) = spawned {
            if !prior_admission && let Some((path, claim)) = admission {
                self.finish_agent_effect(&path, &claim);
            }
            self.reopen_in_flight = None;
            self.set_reopen_notices(vec![live::ReopenNotice {
                pane_id: None,
                message: format!("Reopen could not start; retry is available: {message}"),
            }]);
            self.sync_recent_closed_snapshot();
        }
        true
    }

    pub fn ingest_reopen_result(
        &mut self,
        request: &live::ReopenRequest,
        result: Result<live::FileReopenResultOrHerdr, String>,
    ) -> bool {
        let key = request.item.key();
        if self.reopen_in_flight.as_deref() != Some(key) {
            return false;
        }
        self.reopen_in_flight = None;
        match result {
            Err(message) => {
                let reserved = match &request.item {
                    ClosedItem::Pane { context, .. } | ClosedItem::Tab { context, .. } => {
                        self.has_agent_effect(&context.checkout_path, &format!("reopen:{key}"))
                    }
                    ClosedItem::File { .. } | ClosedItem::Draft { .. } => false,
                };
                let reservation = if reserved {
                    " Its tab place stays reserved."
                } else {
                    ""
                };
                self.set_reopen_notices(vec![live::ReopenNotice {
                    pane_id: request.fallback_pane_id.clone(),
                    message: format!("Reopen failed; retry is available: {message}.{reservation}"),
                }]);
            }
            Ok(live::FileReopenResultOrHerdr::File(file)) => {
                match file {
                    live::FileReopenResult::Opened(opened) => {
                        let (document, place) = *opened;
                        if let ClosedItem::File {
                            workspace_id,
                            checkout_id,
                            path,
                            ..
                        } = &request.item
                        {
                            let tab_id = self.new_file_tab_id(workspace_id, checkout_id, path);
                            let prepared = PreparedFileTab::Read {
                                tab_id: tab_id.clone(),
                                document: Box::new(document),
                                place,
                            };
                            self.show_file_tab(prepared, workspace_id, checkout_id, path, false);
                            // With View areas the open can still be refused as
                            // it lands, when the Workspace filled its views
                            // meanwhile: the item stays and says why.
                            if !self.snapshot.editor.tabs.iter().any(|tab| tab.id == tab_id) {
                                let reason = self
                                    .snapshot
                                    .status
                                    .last_error
                                    .as_ref()
                                    .map_or_else(String::new, |error| error.message.clone());
                                self.set_reopen_notices(vec![live::ReopenNotice {
                                    pane_id: None,
                                    message: format!(
                                        "The file could not be reopened; retry is available: {reason}"
                                    ),
                                }]);
                                self.sync_recent_closed_snapshot();
                                return true;
                            }
                            if let Err(message) = self.focus_editor_tab_context(&tab_id) {
                                self.set_reopen_notices(vec![live::ReopenNotice {
                                pane_id: None,
                                message: format!("The file reopened, but its project context was unavailable: {message}"),
                            }]);
                            } else {
                                self.snapshot.recent_closed.notices.clear();
                            }
                            self.consume_recent_closed(key);
                        }
                    }
                    live::FileReopenResult::Missing => {
                        self.consume_recent_closed(key);
                        self.set_reopen_notices(vec![live::ReopenNotice {
                        pane_id: None,
                        message: "The file was deleted. Restore it from Finder Trash to open it again.".into(),
                    }]);
                    }
                    live::FileReopenResult::Failed(message) => {
                        self.set_reopen_notices(vec![live::ReopenNotice {
                            pane_id: None,
                            message: format!(
                                "The file could not be reopened; retry is available: {message}"
                            ),
                        }]);
                    }
                }
            }
            Ok(live::FileReopenResultOrHerdr::Herdr(outcome)) => {
                // Only this machine's closes reopen, so each pane id is the
                // core's own.
                for agent in &outcome.agents {
                    self.record_pane_launch(
                        &agent.pane_id,
                        &agent.kind,
                        &agent.name,
                        &agent.launch,
                        agent.session_id.as_deref(),
                    );
                }
                if let Some(tab_id) = outcome.tab_id.as_ref() {
                    let context = match &request.item {
                        ClosedItem::Pane { context, .. } | ClosedItem::Tab { context, .. } => {
                            Some(context)
                        }
                        _ => None,
                    };
                    if let Some(name) = context.and_then(ClosedContext::operator_tab_name) {
                        self.remember_operator_tab_name(tab_id, Some(name));
                    }
                    if let Some(context) = context {
                        match self.queue_restored_agent_tab(
                            context,
                            tab_id,
                            &format!("reopen:{key}"),
                        ) {
                            Ok(()) => self.finish_agent_effect(
                                &context.checkout_path,
                                &format!("reopen:{key}"),
                            ),
                            Err(message) => self
                                .push_diagnostic("agent_layout.reopen_placement_failed", message),
                        }
                    }
                }
                if outcome.consumed {
                    self.consume_recent_closed(key);
                }
                let can_show = match (&request.item, outcome.tab_id.as_deref()) {
                    (
                        ClosedItem::Pane { context, .. } | ClosedItem::Tab { context, .. },
                        Some(id),
                    ) => self.agent_can_show_created(&context.checkout_path, id),
                    _ => true,
                };
                if can_show && let Some(pane_id) = outcome.focused_pane_id {
                    self.snapshot.terminal.pane_id = Some(pane_id.clone());
                    self.snapshot.focused.surface = Surface::Terminal;
                    self.snapshot.focused.pane_id = Some(pane_id.clone());
                    self.snapshot.ui_state.selected_pane_id = Some(pane_id);
                    self.yield_surface_to_terminal();
                }
                self.set_reopen_notices(outcome.notices);
            }
        }
        self.sync_recent_closed_snapshot();
        true
    }
}

/// A diff tab's name: the file and the comparison it shows.
pub(super) fn diff_label(path: &str, committed: bool) -> String {
    let scope = if committed {
        "branch diff"
    } else {
        "working diff"
    };
    format!("{} ({scope})", super::workspace_view::file_label(path))
}

impl Runtime {
    /// `close_tab` on this machine, and the same step a tree close takes for
    /// a local tab, so both refuse and start exactly alike.
    pub(super) fn close_local_tab(&mut self, tab_id: String, confirmed: bool) -> bool {
        let tab = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .find(|tab| tab.id.as_deref() == Some(tab_id.as_str()))
            .cloned();
        let Some(tab) = tab else {
            self.set_error(
                "tab.unknown",
                format!("Tab {tab_id} is not available"),
                false,
            );
            return true;
        };
        let pane_ids = tab
            .panes
            .iter()
            .map(|pane| pane.id.as_str())
            .collect::<HashSet<_>>();
        let status_unknown = self.snapshot.navigator.agents.iter().any(|agent| {
            pane_ids.contains(agent.pane_id.as_str())
                && (agent.requires_close_status_check
                    && self.failed_terminal_id(&agent.pane_id).is_none())
        });
        if status_unknown {
            self.set_error(
                "tab.close_status_unknown",
                format!(
                    "Tab {tab_id} has a pane whose activity status is unknown; refresh status before closing"
                ),
                true,
            );
            return true;
        }
        let requires_confirmation = self.snapshot.navigator.agents.iter().any(|agent| {
            pane_ids.contains(agent.pane_id.as_str())
                && (agent.requires_close_confirmation
                    && self.failed_terminal_id(&agent.pane_id).is_none())
        });
        if requires_confirmation && !confirmed {
            self.set_error(
                "tab.close_confirmation_required",
                format!(
                    "Tab {tab_id} contains an agent that is working or needs attention; close_tab requires confirmed=true"
                ),
                false,
            );
            return true;
        }
        self.push_diagnostic("tab.close.requested", format!("Closing tab {tab_id}"));
        self.start_close_capture(live::CloseCaptureTarget::Tab { tab_id }, tab)
    }

    /// `close_pane` on this machine, and the same step a tree close takes for
    /// a local pane: a pane that is the last of its project keeps the project
    /// registered through the close exactly as an ordinary close does.
    ///
    /// The operator's own close (`may_queue`) of one pane among several waits
    /// in its tab's line while an operation there is still with Herdr
    /// (PRD instant-pane-topology D-10); a tree close and the line's own turn
    /// start at once.
    pub(super) fn close_local_pane(
        &mut self,
        pane_id: String,
        confirmed: bool,
        may_queue: bool,
    ) -> bool {
        let status_unknown = self.snapshot.navigator.agents.iter().any(|agent| {
            agent.pane_id == pane_id
                && (agent.requires_close_status_check
                    && self.failed_terminal_id(&agent.pane_id).is_none())
        });
        if status_unknown {
            self.set_error(
                "pane.close_status_unknown",
                format!(
                    "Pane {pane_id} has an unknown activity status; refresh status before closing"
                ),
                true,
            );
            return true;
        }
        let requires_confirmation = self.snapshot.navigator.agents.iter().any(|agent| {
            agent.pane_id == pane_id
                && (agent.requires_close_confirmation
                    && self.failed_terminal_id(&agent.pane_id).is_none())
        });
        if requires_confirmation && !confirmed {
            self.set_error(
                "pane.close_confirmation_required",
                format!(
                    "Pane {pane_id} is working or needs attention; close_pane requires confirmed=true"
                ),
                false,
            );
            return true;
        }
        if self.live.is_none() {
            self.set_error(
                "pane.control_unavailable",
                "Pane close requires a live Herdr connection",
                true,
            );
            return true;
        }
        let tab = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| workspace.checkouts.iter())
            .flat_map(|checkout| checkout.tabs.iter())
            .find(|tab| tab.panes.iter().any(|pane| pane.id == pane_id))
            .cloned();
        let Some(tab) = tab else {
            self.set_error(
                "pane.unknown",
                format!("Pane {pane_id} is not available"),
                false,
            );
            return true;
        };
        // A close waiting its turn is drawn gone at once, so the keyboard
        // leaves the pane now, as it does when the close starts.
        if may_queue && self.queue_pane_close(&pane_id, confirmed) {
            let queued = self.pane_operations.values().any(|operation| {
                operation.phase == "queued"
                    && matches!(
                        &operation.request,
                        super::operations::GeometryRequest::Close { pane_id: closing, .. }
                            if *closing == pane_id
                    )
            });
            if queued && self.failed_terminal_id(&pane_id).is_none() {
                // One pane among several: its tab stays, so it empties no
                // checkout until its turn starts the close.
                self.move_from_closing_target(
                    &live::CloseCaptureTarget::Pane { pane_id },
                    &tab,
                    false,
                );
            }
            return true;
        }
        self.retain_project_before_last_pane_closes(&pane_id);
        self.push_diagnostic("pane.close.requested", format!("Closing pane {pane_id}"));
        self.start_close_capture(live::CloseCaptureTarget::Pane { pane_id }, tab)
    }
}
