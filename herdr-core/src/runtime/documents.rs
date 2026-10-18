//! File tabs on any device: where a document is read from, and how its
//! saves run and settle (PRD S5.5 B8-B15, B33, B34).
//!
//! A tab's document names the device, the checkout root and the root
//! identity it was read under ([`DocumentPlace`]). This machine answers in
//! process, as the local open always has; a device answers through its
//! helper on a worker, and the tab appears when the answer does, in the
//! checkout that asked even if the operator has moved on.
//!
//! One save per document runs at a time, and the newest draft that arrives
//! meanwhile waits behind it (D-15); an older waiting draft is replaced, not
//! sent. A save whose answer is lost is `unknown`: the draft stays, no other
//! save is sent for that document, and the file is read back (after any save
//! still running in its folder has finished) to judge it saved, not saved,
//! or overtaken by another change. Nothing is ever resent on its own.

use std::sync::Arc;

use super::editor::PreparedFileTab;
use super::view_areas::ViewPlacement;
use super::workspace_view::WorkspaceKey;
use super::*;
use crate::files::{DocumentPlace, DocumentRoot, OpenFailure, SaveOutcome};
use crate::model::{EditorConflictSnapshot, EditorOpeningSnapshot, EditorSaveSnapshot};
use crate::node_access::{LinkError, NodeLink};

#[derive(Default)]
pub(super) struct SaveSlot {
    running: bool,
    queued: Option<PendingSave>,
    unsettled: Option<UnsettledSave>,
    checking: bool,
    /// A read-back was asked for while one was running, for instance because
    /// the helper came back; the running one may have used the old
    /// connection, so a failed answer starts another instead of waiting.
    recheck: bool,
    /// Why the unknown save is still unknown, for the tab to show.
    waiting: Option<String>,
    /// The queued save was never sent because the device's helper was still
    /// connecting; it goes out when the helper is ready, and is dropped
    /// (draft kept) if the connection fails. The reason is what the tab shows.
    held: Option<String>,
    /// Why the last save of this tab was refused or not sent, until the next
    /// save starts: the tab shows it with Export and Retry at the save's own
    /// place rather than only in the diagnostic log (S5.5 B15, B45).
    refused: Option<Unsaved>,
}

/// Why the tab's last save has not landed.
enum Unsaved {
    /// It was refused, or never sent.
    Refused(String),
    /// Its answer was lost and the file was read back unchanged: it has not
    /// reached the file yet, and a helper whose connection ended may still
    /// finish it, so it is "not yet" rather than "not saved" (B14).
    NotYet(String),
}

struct PendingSave {
    contents: String,
    close_after: CloseAfter,
}

/// What closes once a save lands with nothing left unsaved: nothing, or the
/// document, and with it the lane of the Workspace whose view the operator
/// closed when that leaves the lane nothing to show (`close_emptied_lane`).
#[derive(Clone)]
pub(super) enum CloseAfter {
    Nothing,
    Document { lane: Option<WorkspaceKey> },
}

#[derive(Clone, PartialEq)]
pub(super) struct UnsettledSave {
    contents: String,
    expected: String,
}

pub(super) struct SaveRequest {
    contents: String,
    expected: String,
    close_after: CloseAfter,
    place: DocumentPlace,
}

pub(super) struct OpenRequest {
    generation: u64,
    workspace_id: String,
    checkout_id: String,
    path: String,
    preview: bool,
    /// Replaces the open tab's document (Reload) instead of adding a tab.
    reload: bool,
    reveal: Option<PendingReveal>,
    /// A View display's file read back after a restart, or retried
    /// (`view_areas.rs`): it joins the editor without taking the screen, the
    /// display binds to it by what it shows, and a failed read makes an
    /// unavailable tab rather than an error.
    restore: bool,
    /// Where the document shows once it is read, in a shell with View areas
    /// (PRD S7 B1): nothing moves until the read lands, and a failed read
    /// moves nothing.
    placements: Vec<ViewPlacement>,
}

/// A reveal waiting on its file. The screen moves when the read lands, all
/// at once, and only while the checkout the operator had in front when they
/// asked is still in front; a failed read moves nothing.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct PendingReveal {
    pub(super) front: Option<(String, String)>,
}

type OpenResult = Result<(EditorDocumentSnapshot, DocumentPlace), OpenFailure>;

impl Runtime {
    /// Every project the catalog carries: this machine's navigator, then each
    /// device's Herdr session.
    pub(super) fn catalog_workspaces(&self) -> impl Iterator<Item = &WorkspaceSnapshot> {
        let sessions = self
            .snapshot
            .status
            .remote
            .iter()
            .filter_map(|remote| remote.session.as_ref())
            .flat_map(|session| session.workspaces.iter());
        self.snapshot.navigator.workspaces.iter().chain(sessions)
    }

    /// A checkout by its ids wherever the catalog carries it: this machine's
    /// and registered projects in the navigator, and each device's Herdr
    /// session. Device ids are scoped (`remote:<device>:…`), so one id never
    /// names checkouts on two machines.
    pub(super) fn catalog_checkout(
        &self,
        workspace_id: &str,
        checkout_id: &str,
    ) -> Option<(&WorkspaceSnapshot, &CheckoutSnapshot)> {
        self.catalog_workspaces()
            .find(|workspace| workspace.id == workspace_id)
            .and_then(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.id == checkout_id)
                    .map(|checkout| (workspace, checkout))
            })
    }

    /// Whether a read of `path` in this checkout is still running.
    pub(super) fn document_opens_path(
        &self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
    ) -> bool {
        self.document_opens.values().any(|open| {
            open.workspace_id == workspace_id
                && open.checkout_id == checkout_id
                && open.path == path
        })
    }

    /// Opens still being read that will add a View display to the
    /// Workspace `key`, so the display cap counts them before they land.
    pub(super) fn pending_view_placements(&self, key: &WorkspaceKey) -> usize {
        self.document_opens
            .values()
            .flat_map(|open| &open.placements)
            .filter(|placement| &placement.key == key)
            .count()
    }

    /// Authoritative tab removal retires only that owner's GUI destinations.
    /// A shared read still serves every surviving explicit request.
    pub(super) fn prune_gui_reads(&mut self, key: &WorkspaceKey, tabs: &[&str]) -> bool {
        let before = self.document_opens.len();
        let mut changed = false;
        self.document_opens.retain(|_, open| {
            let count = open.placements.len();
            open.placements.retain(|placement| {
                &placement.key != key
                    || !placement
                        .owner
                        .as_deref()
                        .is_some_and(|owner| !tabs.contains(&owner))
            });
            changed |= count != open.placements.len();
            count == 0 || !open.placements.is_empty() || open.reload || open.restore
        });
        if self.document_opens.len() != before {
            self.sync_opening_snapshot();
        }
        changed
    }

    /// Stops waiting for a read of `path` that no View display needs any
    /// more: its answer then lands nowhere, and no tab appears for it.
    pub(super) fn cancel_document_read(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
        path: &str,
    ) {
        let before = self.document_opens.len();
        self.document_opens.retain(|_, open| {
            open.reload
                || !open.placements.is_empty()
                || open.workspace_id != workspace_id
                || open.checkout_id != checkout_id
                || open.path != path
        });
        if self.document_opens.len() != before {
            self.sync_opening_snapshot();
        }
    }

    /// The checkout the operator is looking at: this machine's focus while
    /// it is the selected device, otherwise the core's explicit device/path
    /// choice. Herdr focus supplies the initial choice only.
    pub(super) fn front_checkout(&self) -> Option<(&str, &str)> {
        let device = self
            .snapshot
            .navigator
            .focused_device_id
            .as_deref()
            .unwrap_or(self.node.as_str());
        if device == self.node.as_str() {
            return self
                .snapshot
                .navigator
                .focused_workspace_id
                .as_deref()
                .zip(self.snapshot.navigator.focused_checkout_id.as_deref());
        }
        if let Some(path) = self
            .workspace_views
            .as_ref()
            .and_then(|store| store.remote_front.get(device))
            && let Some((workspace, checkout)) = self
                .catalog_workspaces()
                .filter(|workspace| workspace.device_id == device)
                .find_map(|workspace| {
                    workspace
                        .checkouts
                        .iter()
                        .find(|checkout| &checkout.path == path)
                        .map(|checkout| (workspace, checkout))
                })
        {
            return Some((&workspace.id, &checkout.id));
        }
        let session = self
            .snapshot
            .status
            .remote
            .iter()
            .find(|remote| remote.target_id == device)?
            .session
            .as_ref()?;
        session
            .focused_workspace_id
            .as_deref()
            .zip(session.focused_checkout_id.as_deref())
    }

    /// Whether the editor is showing a tab of a checkout on `device`.
    pub(super) fn active_editor_tab_on_device(&self, device: &str) -> bool {
        let Some(tab) = self
            .snapshot
            .editor
            .active_tab_id
            .as_deref()
            .and_then(|id| self.snapshot.editor.tabs.iter().find(|tab| tab.id == id))
        else {
            return false;
        };
        self.catalog_checkout(&tab.workspace_id, &tab.checkout_id)
            .is_some_and(|(workspace, _)| workspace.device_id == device)
    }

    pub(super) fn front_checkout_owned(&self) -> Option<(String, String)> {
        self.front_checkout()
            .map(|(workspace, checkout)| (workspace.to_owned(), checkout.to_owned()))
    }

    /// The checkout a file belongs to, as its device reaches it. This
    /// machine's checkouts resolve through the roots hided pinned when it
    /// opened them, so a replaced checkout refuses the read.
    pub(super) fn document_root(
        &self,
        workspace_id: &str,
        checkout_id: &str,
    ) -> Result<DocumentRoot, String> {
        let (workspace, checkout) = self
            .catalog_checkout(workspace_id, checkout_id)
            .ok_or_else(|| "The file's project or checkout is no longer available".to_owned())?;
        let device_id = workspace.device_id.clone();
        if device_id == self.node.as_str()
            && let Some(roots) = self.file_roots.as_ref()
        {
            let not_opened =
                || "The checkout is not one this daemon has opened, so nothing was read".to_owned();
            let native =
                hide_platform::path::from_wire(&checkout.path).map_err(|_| not_opened())?;
            let (path, identity) = roots.pinned_root(&native).ok_or_else(not_opened)?;
            return Ok(DocumentRoot {
                device_id,
                path: hide_platform::path::to_wire(&path).map_err(|_| not_opened())?,
                identity: Some(identity),
            });
        }
        Ok(DocumentRoot {
            device_id,
            path: checkout.path.clone(),
            identity: None,
        })
    }

    /// Where a file of the checkout is read from: its root and the channel
    /// of the device that holds it. Nothing is read here; the read runs on a
    /// worker (`start_document_open`), on this machine as on a device.
    pub(super) fn document_source(
        &mut self,
        workspace_id: &str,
        checkout_id: &str,
    ) -> Result<(DocumentRoot, Arc<dyn NodeLink>), String> {
        let root = self.document_root(workspace_id, checkout_id)?;
        let channel = self.node_link(&root.device_id)?;
        Ok((root, channel))
    }

    pub(super) fn start_document_open(
        &mut self,
        root: DocumentRoot,
        channel: Arc<dyn NodeLink>,
        request: OpenRequestFields,
    ) {
        let tab_id =
            self.new_file_tab_id(&request.workspace_id, &request.checkout_id, &request.path);
        // Replacing previews/New tabs need no extra display slot, but churn
        // while a read is held must not grow its destination queue forever.
        if let Some(placement) = &request.placement {
            let repeated = self.document_opens.get(&tab_id).is_some_and(|running| {
                running
                    .placements
                    .iter()
                    .any(|held| held.same_destination(placement))
            });
            if !repeated
                && self.pending_view_placements(&placement.key)
                    >= crate::view_layout::MAX_VIEW_DISPLAYS
            {
                let error = crate::view_layout::LayoutError::DisplayLimit;
                self.set_error(error.kind(), error.message(), false);
                return;
            }
        }
        if !request.reload
            && let Some(running) = self.document_opens.get_mut(&tab_id)
        {
            // Share only the file read. Each asking tab retains its GUI
            // destination; repeated clicks at the same destination converge,
            // with a later pin keeping that display open.
            if !request.preview && !running.reload {
                running.preview = false;
            }
            if let Some(placement) = request.placement {
                running.restore = false;
                if let Some(held) = running
                    .placements
                    .iter_mut()
                    .find(|held| held.same_destination(&placement))
                {
                    held.preview &= placement.preview;
                } else {
                    running.placements.push(placement);
                }
            }
            return;
        }
        // Reload supersedes the I/O generation, not any admitted GUI
        // destinations waiting for this same shared buffer.
        let mut placements = self
            .document_opens
            .remove(&tab_id)
            .map(|open| open.placements)
            .unwrap_or_default();
        placements.extend(request.placement);
        self.next_document_generation += 1;
        let generation = self.next_document_generation;
        self.document_opens.insert(
            tab_id.clone(),
            OpenRequest {
                generation,
                workspace_id: request.workspace_id,
                checkout_id: request.checkout_id,
                path: request.path.clone(),
                preview: request.preview,
                reload: request.reload,
                reveal: request.reveal,
                restore: request.restore,
                placements,
            },
        );
        self.sync_opening_snapshot();
        crate::diagnostic!(serde_json::json!({
            "component": "documents",
            "kind": "file.open_requested",
            "device": root.device_id,
            "generation": generation,
            "reload": request.reload,
        }));
        let path = request.path;
        // A runtime without a worker context has no shared mutex to hold, so
        // the read runs in place; every shared runtime reads on a worker, the
        // local disk included, because a slow volume must not stall input.
        let Some(context) = self.worker_context.clone() else {
            let result = files::open_document(channel.as_ref(), &root, &path);
            self.ingest_document_open(&tab_id, generation, result);
            return;
        };
        let spawned = thread::Builder::new()
            .name("herdr-core-file-open".to_owned())
            .spawn(move || {
                let result = files::open_document(channel.as_ref(), &root, &path);
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.ingest_document_open(&tab_id, generation, result),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            let tab_id = self
                .document_opens
                .iter()
                .find(|(_, open)| open.generation == generation)
                .map(|(tab_id, _)| tab_id.clone());
            if let Some(tab_id) = tab_id {
                self.document_opens.remove(&tab_id);
            }
            self.sync_opening_snapshot();
            self.set_error(
                "file.open_worker_failed",
                format!("The file reader could not start: {error}"),
                true,
            );
        }
    }

    /// Drops reads still running for checkouts under `scope` (a removed
    /// device's `remote:<id>:`), so a late answer opens no tab for it.
    pub(super) fn forget_device_opens(&mut self, scope: &str) {
        let before = self.document_opens.len();
        self.document_opens
            .retain(|_, open| !open.checkout_id.starts_with(scope));
        if self.document_opens.len() != before {
            self.sync_opening_snapshot();
        }
    }

    fn sync_opening_snapshot(&mut self) {
        let mut opening: Vec<_> = self.document_opens.values().collect();
        opening.sort_by_key(|open| open.generation);
        self.snapshot.editor.opening = opening
            .into_iter()
            .filter(|open| !open.reload)
            .map(|open| EditorOpeningSnapshot {
                workspace_id: open.workspace_id.clone(),
                checkout_id: open.checkout_id.clone(),
                path: open.path.clone(),
            })
            .collect();
    }

    /// A device read came back. An answer for a request that was replaced,
    /// or forgotten with its device, is dropped, and the log says so under
    /// the generation `file.open_requested` named. The tab is shown where it
    /// was asked for; it takes the screen only while that checkout is still
    /// the one in front (B34).
    pub(super) fn ingest_document_open(
        &mut self,
        tab_id: &str,
        generation: u64,
        result: OpenResult,
    ) -> bool {
        if self.document_opens.get(tab_id).map(|open| open.generation) != Some(generation) {
            self.push_diagnostic(
                "file.open_dropped",
                format!(
                    "The read for open request {generation} came back after the request was dropped"
                ),
            );
            return false;
        }
        let request = self
            .document_opens
            .remove(tab_id)
            .expect("the request was just found");
        self.sync_opening_snapshot();
        let (document, place) = match result {
            Ok(opened) => opened,
            // A View display's file read back after a restart, or retried:
            // its tab says why, and nothing else moves (S6 B20).
            Err(failure) if request.restore => {
                self.insert_unavailable_file_tab(
                    &request.workspace_id,
                    &request.checkout_id,
                    &request.path,
                    request.preview,
                    failure.message(),
                );
                return true;
            }
            Err(failure) => {
                self.set_error(
                    if request.reload {
                        "file.reload_failed"
                    } else {
                        "file.open_failed"
                    },
                    failure.message(),
                    true,
                );
                return true;
            }
        };
        // A tab that said its file was unavailable takes the file now that
        // it could be read (B20). A retry shows it where it is; an open shows
        // it like any open would.
        if !request.reload
            && let Some(tab) = self
                .snapshot
                .editor
                .tabs
                .iter_mut()
                .find(|tab| tab.id == tab_id && tab.unavailable_reason.is_some())
        {
            tab.unavailable_reason = None;
            self.replace_document(tab_id, document, place);
            if request.restore {
                return true;
            }
            let prepared = PreparedFileTab::Open(tab_id.to_owned());
            let in_front = self.front_checkout()
                == Some((request.workspace_id.as_str(), request.checkout_id.as_str()));
            if !request.placements.is_empty() {
                self.complete_file_in_views(
                    prepared,
                    &request.workspace_id,
                    &request.checkout_id,
                    &request.path,
                    request.placements,
                );
            } else if let Some(reveal) = &request.reveal
                && self.front_checkout_owned() == reveal.front
            {
                self.settle_reveal(
                    &request.workspace_id,
                    &request.checkout_id,
                    &request.path,
                    Some(prepared),
                );
            } else if in_front {
                self.show_file_tab(
                    prepared,
                    &request.workspace_id,
                    &request.checkout_id,
                    &request.path,
                    request.preview,
                );
            }
            self.persist_current_ui_state();
            return true;
        }
        let open = self.snapshot.editor.tabs.iter().any(|tab| tab.id == tab_id);
        if let Some(reveal) = &request.reveal
            && !open
            && request.placements.is_empty()
            && self.front_checkout_owned() == reveal.front
        {
            self.settle_reveal(
                &request.workspace_id,
                &request.checkout_id,
                &request.path,
                Some(PreparedFileTab::Read {
                    tab_id: tab_id.to_owned(),
                    document: Box::new(document),
                    place,
                }),
            );
            return true;
        }
        if request.reload && open {
            self.replace_document(tab_id, document.clone(), place.clone());
        }
        if !request.placements.is_empty() {
            // A CLI open may already have supplied this shared buffer. Honor
            // the GUI destinations using it, without replacing dirty/save state.
            let prepared = if open {
                PreparedFileTab::Open(tab_id.to_owned())
            } else {
                PreparedFileTab::Read {
                    tab_id: tab_id.to_owned(),
                    document: Box::new(document),
                    place,
                }
            };
            self.complete_file_in_views(
                prepared,
                &request.workspace_id,
                &request.checkout_id,
                &request.path,
                request.placements,
            );
            self.persist_current_ui_state();
            return true;
        }
        if request.reload || open {
            return true;
        }
        let in_front = self.front_checkout()
            == Some((request.workspace_id.as_str(), request.checkout_id.as_str()));
        let prepared = PreparedFileTab::Read {
            tab_id: tab_id.to_owned(),
            document: Box::new(document),
            place,
        };
        // A View display's file read back after a restart joins the editor
        // without taking the screen; the display binds to it by what it
        // shows (`view_areas.rs`).
        if request.restore {
            self.insert_file_tab(
                prepared,
                &request.workspace_id,
                &request.checkout_id,
                &request.path,
                request.preview,
            );
            return true;
        }
        if in_front {
            self.show_file_tab(
                prepared,
                &request.workspace_id,
                &request.checkout_id,
                &request.path,
                request.preview,
            );
        } else {
            self.insert_file_tab(
                prepared,
                &request.workspace_id,
                &request.checkout_id,
                &request.path,
                request.preview,
            );
        }
        self.persist_current_ui_state();
        true
    }

    /// Puts a freshly read document in place of the tab's current one: the
    /// operator chose Reload, so the draft and any save state go with it.
    pub(super) fn replace_document(
        &mut self,
        tab_id: &str,
        document: EditorDocumentSnapshot,
        place: DocumentPlace,
    ) {
        self.editor_documents
            .insert(tab_id.to_owned(), Edited::new(document));
        self.document_places.insert(tab_id.to_owned(), place);
        self.document_saves.remove(tab_id);
        self.sync_file_tab_dirty(tab_id);
        self.sync_active_editor_document();
    }

    /// Reload: reads the file again under the checkout's current root, so a
    /// checkout that was replaced is adopted only on this explicit request.
    pub(super) fn reload_document(&mut self, tab_id: &str) -> bool {
        let Some(tab) = self
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .cloned()
        else {
            self.set_error(
                "file.reload_failed",
                "The file tab is no longer open",
                false,
            );
            return true;
        };
        // A save in flight owns the tab's revision until its answer is
        // settled; a reload landing first would take the slot with it and
        // let the save's revision describe the reloaded text (PRD S5.5 B13).
        if self.document_saves.get(tab_id).is_some_and(|slot| {
            slot.running || slot.checking || slot.unsettled.is_some() || slot.queued.is_some()
        }) {
            self.set_error(
                "file.reload_busy",
                format!(
                    "{} is still being saved; reload once the save has finished",
                    tab.path
                ),
                true,
            );
            return true;
        }
        match self.document_source(&tab.workspace_id, &tab.checkout_id) {
            Ok((root, channel)) => self.start_document_open(
                root,
                channel,
                OpenRequestFields {
                    workspace_id: tab.workspace_id,
                    checkout_id: tab.checkout_id,
                    path: tab.path,
                    preview: false,
                    reload: true,
                    reveal: None,
                    restore: false,
                    placement: None,
                },
            ),
            Err(message) => self.set_error("file.reload_failed", message, true),
        }
        true
    }

    /// Keep Editing: the draft stays and is now judged against what the file
    /// holds, so the next save replaces that version on purpose.
    pub(super) fn keep_editing_document(&mut self, tab_id: &str) -> bool {
        let Some(document) = self.editor_documents.get_mut(tab_id).map(Edited::edit) else {
            self.set_error(
                "file.conflict_without_document",
                "The active file tab has no document state",
                false,
            );
            return true;
        };
        if let Some(conflict) = document.conflict.take()
            && let Some(disk) = conflict.disk_revision
        {
            document.revision = Some(disk);
        }
        self.sync_active_editor_document();
        true
    }

    /// A save the operator asked for, or the save a close waits on.
    pub(super) fn request_file_save(
        &mut self,
        payload: FileSavePayload,
        close_after: CloseAfter,
    ) -> bool {
        let Some(tab_id) = self
            .snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| tab.id == payload.tab_id && tab.path == payload.path)
            .map(|tab| tab.id.clone())
        else {
            self.set_error("file.save_rejected", "The save target is not open", false);
            return true;
        };
        let Some(document) = self.editor_documents.get_mut(&tab_id).map(Edited::edit) else {
            self.set_error(
                "file.save_rejected",
                "The save target has no document state",
                false,
            );
            return true;
        };
        if let Err(message) = files::check_editable(document, "the draft was preserved") {
            self.set_error("file.save_rejected", message, false);
            return true;
        }
        // The operator asked for the file's text in place of the draft; a
        // save sent now would race the reload for which text the tab keeps.
        if self
            .document_opens
            .get(&tab_id)
            .is_some_and(|open| open.reload)
        {
            self.set_error(
                "file.save_during_reload",
                format!(
                    "{} is being reloaded, so this save was not sent",
                    payload.path
                ),
                true,
            );
            return true;
        }
        let Some(document) = self.editor_documents.get_mut(&tab_id).map(Edited::edit) else {
            return true;
        };
        document.contents_utf8 = Some(payload.contents_utf8.clone());
        document.dirty = true;
        self.sync_file_tab_dirty(&tab_id);
        self.sync_active_editor_document();
        let slot = self.document_saves.entry(tab_id.clone()).or_default();
        if slot.unsettled.is_some() {
            self.set_error(
                "file.save_unsettled",
                "The last save's result is still unknown, so this save was not sent. Hide reads the file back first; the draft is kept.",
                true,
            );
            self.start_save_settle(&tab_id);
            self.sync_save_snapshot(&tab_id);
            return true;
        }
        if slot.running || slot.held.is_some() {
            slot.queued = Some(PendingSave {
                contents: payload.contents_utf8,
                close_after,
            });
            self.sync_save_snapshot(&tab_id);
            return true;
        }
        self.start_document_save(&tab_id, payload.contents_utf8, close_after);
        true
    }

    fn start_document_save(&mut self, tab_id: &str, contents: String, close_after: CloseAfter) {
        if let Some(slot) = self.document_saves.get_mut(tab_id) {
            slot.refused = None;
        }
        let Some(place) = self.document_places.get(tab_id).cloned() else {
            self.set_error(
                "file.save_rejected",
                "The file tab has no location to save to; the draft was preserved",
                false,
            );
            return;
        };
        let Some(expected) = self
            .editor_documents
            .get(tab_id)
            .and_then(|document| document.revision.clone())
        else {
            self.set_error(
                "file.save_rejected",
                "The file's revision was never read; the draft was preserved",
                false,
            );
            return;
        };
        let channel = match self.node_link(&place.device_id) {
            Ok(channel) => channel,
            Err(message) if self.device_host_connecting(&place.device_id) => {
                let slot = self.document_saves.entry(tab_id.to_owned()).or_default();
                slot.queued = Some(PendingSave {
                    contents,
                    close_after,
                });
                slot.held = Some(message);
                self.sync_save_snapshot(tab_id);
                return;
            }
            Err(message) => {
                let message = format!("{message}; nothing was sent and the draft was preserved");
                self.document_saves
                    .entry(tab_id.to_owned())
                    .or_default()
                    .refused = Some(Unsaved::Refused(message.clone()));
                self.sync_save_snapshot(tab_id);
                self.set_error("file.save_unavailable", message, true);
                return;
            }
        };
        let Some(context) = self.worker_context.clone() else {
            self.set_error(
                "file.save_worker_unavailable",
                "The file save worker is unavailable; the draft was preserved",
                true,
            );
            return;
        };
        self.document_saves
            .entry(tab_id.to_owned())
            .or_default()
            .running = true;
        self.sync_save_snapshot(tab_id);
        crate::diagnostic!(serde_json::json!({
            "component": "documents",
            "kind": "file.save_started",
            "device": place.device_id,
            "path": place.relative.as_str(),
        }));
        let request = SaveRequest {
            contents,
            expected,
            close_after,
            place,
        };
        let worker_tab = tab_id.to_owned();
        let spawned = thread::Builder::new()
            .name("herdr-core-file-save".to_owned())
            .spawn(move || {
                let outcome = files::save_document(
                    channel.as_ref(),
                    &request.place,
                    &request.contents,
                    &request.expected,
                );
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.ingest_document_save(&worker_tab, request, outcome),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            });
        if let Err(error) = spawned {
            if let Some(slot) = self.document_saves.get_mut(tab_id) {
                slot.running = false;
            }
            self.sync_save_snapshot(tab_id);
            self.set_error(
                "file.save_worker_failed",
                format!("The file save worker could not start: {error}; the draft was preserved"),
                true,
            );
        }
    }

    pub(super) fn ingest_document_save(
        &mut self,
        tab_id: &str,
        request: SaveRequest,
        outcome: SaveOutcome,
    ) -> bool {
        let queued = match self.document_saves.get_mut(tab_id) {
            Some(slot) => {
                slot.running = false;
                slot.queued.take()
            }
            None => None,
        };
        let path =
            hide_platform::path::wire_join(&request.place.root.path, &request.place.relative);
        let Some(document) = self.editor_documents.get_mut(tab_id).map(Edited::edit) else {
            self.document_saves.remove(tab_id);
            self.push_diagnostic(
                "file.save_after_close",
                format!(
                    "A save of {path} finished after its tab closed: {}",
                    outcome_word(&outcome)
                ),
            );
            return true;
        };
        let mut next = None;
        let mut close = CloseAfter::Nothing;
        // A save that did not land leaves one diagnostic line naming its
        // device, tab and outcome (S5.5 B48); the reason is the host's words,
        // which never carry the draft.
        let failure = match &outcome {
            SaveOutcome::Saved(_) => None,
            SaveOutcome::Conflict { message, .. } | SaveOutcome::Refused(message) => {
                Some(message.clone())
            }
            SaveOutcome::Unknown(reason) => Some(reason.clone()),
        };
        if let Some(reason) = failure {
            crate::diagnostic!(serde_json::json!({
                "component": "documents", "kind": "file.save_not_saved",
                "outcome": outcome_word(&outcome), "device": request.place.device_id,
                "tab": tab_id, "path": path, "reason": reason,
            }));
        }
        match outcome {
            SaveOutcome::Saved(saved) => {
                document.revision = Some(saved.revision);
                document.conflict = None;
                if document.contents_utf8.as_deref() == Some(request.contents.as_str()) {
                    document.dirty = false;
                }
                let dirty = document.dirty;
                self.push_diagnostic("file.save_ready", format!("Saved {path}"));
                match queued {
                    Some(waiting) if waiting.contents != request.contents => next = Some(waiting),
                    _ if dirty => {}
                    Some(waiting) => close = waiting.close_after,
                    None => close = request.close_after,
                }
            }
            SaveOutcome::Conflict {
                disk_revision,
                message,
            } => {
                document.conflict = Some(EditorConflictSnapshot {
                    opened_revision: request.expected,
                    disk_revision,
                });
                document.dirty = true;
                self.set_error("file.save_conflict", message, true);
            }
            SaveOutcome::Refused(message) => {
                self.document_saves
                    .entry(tab_id.to_owned())
                    .or_default()
                    .refused = Some(Unsaved::Refused(message.clone()));
                self.set_error("file.save_failed", message, true);
            }
            SaveOutcome::Unknown(reason) => {
                let slot = self.document_saves.entry(tab_id.to_owned()).or_default();
                slot.unsettled = Some(UnsettledSave {
                    contents: request.contents,
                    expected: request.expected,
                });
                self.set_error(
                    "file.save_unknown",
                    format!(
                        "{reason}. The draft is kept, and Hide reads the file back to learn whether the save landed."
                    ),
                    true,
                );
                self.sync_save_snapshot(tab_id);
                self.start_save_settle(tab_id);
            }
        }
        self.sync_file_tab_dirty(tab_id);
        self.sync_save_snapshot(tab_id);
        if let Some(next) = next {
            self.start_document_save(tab_id, next.contents, next.close_after);
        } else if let CloseAfter::Document { lane } = close {
            let changed = self.close_file_tab_now(tab_id);
            if let Some(key) = lane {
                self.close_emptied_lane(&key);
            }
            return changed;
        }
        true
    }

    /// Reads back the file of a save whose answer was lost. Waits, without
    /// sending anything, while the device cannot be reached.
    pub(super) fn start_save_settle(&mut self, tab_id: &str) {
        let Some(slot) = self.document_saves.get(tab_id) else {
            return;
        };
        let Some(unsettled) = slot.unsettled.clone() else {
            return;
        };
        if slot.checking {
            if let Some(slot) = self.document_saves.get_mut(tab_id) {
                slot.recheck = true;
            }
            return;
        }
        let Some(place) = self.document_places.get(tab_id).cloned() else {
            return;
        };
        let channel = match self.node_link(&place.device_id) {
            Ok(channel) => channel,
            Err(message) => {
                if let Some(slot) = self.document_saves.get_mut(tab_id) {
                    slot.waiting = Some(message);
                }
                self.sync_save_snapshot(tab_id);
                return;
            }
        };
        let Some(context) = self.worker_context.clone() else {
            return;
        };
        if let Some(slot) = self.document_saves.get_mut(tab_id) {
            slot.checking = true;
            slot.recheck = false;
        }
        self.sync_save_snapshot(tab_id);
        let worker_tab = tab_id.to_owned();
        let spawned = thread::Builder::new()
            .name("herdr-core-file-settle".to_owned())
            .spawn(move || {
                let result = files::revision_now(channel.as_ref(), &place);
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.ingest_save_settle(&worker_tab, unsettled, result),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
            });
        if spawned.is_err() {
            if let Some(slot) = self.document_saves.get_mut(tab_id) {
                slot.checking = false;
            }
            self.sync_save_snapshot(tab_id);
        }
    }

    pub(super) fn ingest_save_settle(
        &mut self,
        tab_id: &str,
        unsettled: UnsettledSave,
        result: Result<Option<String>, LinkError>,
    ) -> bool {
        let Some(slot) = self.document_saves.get_mut(tab_id) else {
            return false;
        };
        slot.checking = false;
        if slot.unsettled.as_ref() != Some(&unsettled) {
            return false;
        }
        let Some(document) = self.editor_documents.get_mut(tab_id).map(Edited::edit) else {
            self.document_saves.remove(tab_id);
            return false;
        };
        let saved = hide_node_link::document::revision_of(unsettled.contents.as_bytes());
        match result {
            Ok(Some(revision)) if revision == saved => {
                slot.unsettled = None;
                slot.waiting = None;
                document.revision = Some(revision);
                document.conflict = None;
                if document.contents_utf8.as_deref() == Some(unsettled.contents.as_str()) {
                    document.dirty = false;
                }
                let path = document.path.clone();
                self.push_diagnostic(
                    "file.save_settled_saved",
                    format!("The unanswered save of {path} had reached the file"),
                );
            }
            Ok(Some(revision)) if revision == unsettled.expected => {
                slot.unsettled = None;
                slot.waiting = None;
                let message = "The file was read back unchanged, so the unanswered save has not reached it yet. The device may still finish it; the draft is kept, and a save that finds the file changed will show the conflict.";
                slot.refused = Some(Unsaved::NotYet(message.to_owned()));
                self.set_error("file.save_not_applied", message, true);
            }
            Ok(disk_revision) => {
                slot.unsettled = None;
                slot.waiting = None;
                document.dirty = true;
                document.conflict = Some(EditorConflictSnapshot {
                    opened_revision: unsettled.expected,
                    disk_revision,
                });
                self.set_error(
                    "file.save_conflict",
                    "The file changed while the save's result was unknown; choose Reload or Keep Editing. The draft is kept.",
                    true,
                );
            }
            Err(LinkError::Refused(error)) => {
                slot.unsettled = None;
                slot.waiting = None;
                document.dirty = true;
                document.conflict = Some(EditorConflictSnapshot {
                    opened_revision: unsettled.expected,
                    disk_revision: None,
                });
                self.set_error(
                    "file.save_conflict",
                    format!(
                        "{}; the unanswered save cannot be read back. The draft is kept.",
                        error.message
                    ),
                    true,
                );
            }
            Err(error) => {
                slot.unsettled = Some(unsettled);
                slot.waiting = Some(error.to_string());
                if slot.recheck {
                    self.sync_save_snapshot(tab_id);
                    self.start_save_settle(tab_id);
                    return true;
                }
            }
        }
        self.sync_file_tab_dirty(tab_id);
        self.sync_save_snapshot(tab_id);
        true
    }

    /// A device's helper came back: every save on it whose answer was lost
    /// is read back now, and every save that waited for the helper is sent.
    pub(super) fn settle_device_saves(&mut self, device_id: &str) {
        for tab_id in self.device_save_tabs(device_id) {
            let Some(slot) = self.document_saves.get_mut(&tab_id) else {
                continue;
            };
            if slot.unsettled.is_some() {
                self.start_save_settle(&tab_id);
            } else if slot.held.take().is_some()
                && let Some(queued) = slot.queued.take()
            {
                self.start_document_save(&tab_id, queued.contents, queued.close_after);
            }
        }
    }

    /// A device's helper could not be reached: the saves that waited for it
    /// are not sent, and each tab keeps its draft and says so.
    pub(super) fn release_held_saves(&mut self, device_id: &str, reason: &str) {
        let mut released = false;
        for tab_id in self.device_save_tabs(device_id) {
            let Some(slot) = self.document_saves.get_mut(&tab_id) else {
                continue;
            };
            if slot.held.take().is_some() {
                slot.queued = None;
                slot.refused = Some(Unsaved::Refused(format!(
                    "{reason}; nothing was sent and the draft was preserved"
                )));
                released = true;
                self.sync_save_snapshot(&tab_id);
            }
        }
        if released {
            self.set_error(
                "file.save_unavailable",
                format!("{reason}; nothing was sent and the draft was preserved"),
                true,
            );
        }
    }

    fn device_save_tabs(&self, device_id: &str) -> Vec<String> {
        self.document_saves
            .keys()
            .filter(|tab_id| {
                self.document_places
                    .get(*tab_id)
                    .is_some_and(|place| place.device_id == device_id)
            })
            .cloned()
            .collect()
    }

    fn sync_save_snapshot(&mut self, tab_id: &str) {
        let save = self.document_saves.get(tab_id).and_then(|slot| {
            if slot.unsettled.is_some() {
                Some(EditorSaveSnapshot {
                    state: if slot.checking { "checking" } else { "unknown" }.to_owned(),
                    message: Some(match &slot.waiting {
                        Some(reason) => format!(
                            "The last save's result is unknown; waiting to read the file back: {reason}"
                        ),
                        None => "The last save's result is unknown; reading the file back".to_owned(),
                    }),
                })
            } else if slot.running {
                Some(EditorSaveSnapshot {
                    state: "saving".to_owned(),
                    message: None,
                })
            } else if let Some(reason) = &slot.held {
                Some(EditorSaveSnapshot {
                    state: "waiting".to_owned(),
                    message: Some(format!(
                        "{reason}; the save goes out when the helper is ready"
                    )),
                })
            } else {
                slot.refused.as_ref().map(|unsaved| {
                    let (state, message) = match unsaved {
                        Unsaved::Refused(message) => ("refused", message),
                        Unsaved::NotYet(message) => ("not_applied", message),
                    };
                    EditorSaveSnapshot {
                        state: state.to_owned(),
                        message: Some(message.clone()),
                    }
                })
            }
        });
        if let Some(document) = self.editor_documents.get_mut(tab_id).map(Edited::edit) {
            document.save = save;
        }
        self.sync_active_editor_document();
    }

    /// Forgets a retired buffer's place; a save still running settles into
    /// nothing when it answers. Admitted GUI destinations own their read
    /// independently and remain until completion or their owner is pruned.
    pub(super) fn forget_document(&mut self, tab_id: &str) {
        self.document_places.remove(tab_id);
        if let Some(slot) = self.document_saves.get(tab_id)
            && !slot.running
            && !slot.checking
        {
            self.document_saves.remove(tab_id);
        }
        if self
            .document_opens
            .get(tab_id)
            .is_some_and(|open| open.placements.is_empty())
        {
            self.document_opens.remove(tab_id);
            self.sync_opening_snapshot();
        }
    }
}

/// What a device read is for, as the caller that starts it names it.
pub(super) struct OpenRequestFields {
    pub(super) workspace_id: String,
    pub(super) checkout_id: String,
    pub(super) path: String,
    pub(super) preview: bool,
    pub(super) reload: bool,
    pub(super) reveal: Option<PendingReveal>,
    pub(super) restore: bool,
    pub(super) placement: Option<ViewPlacement>,
}

fn outcome_word(outcome: &SaveOutcome) -> &'static str {
    match outcome {
        SaveOutcome::Saved(_) => "saved",
        SaveOutcome::Conflict { .. } => "conflict",
        SaveOutcome::Refused(_) => "refused",
        SaveOutcome::Unknown(_) => "unknown",
    }
}
