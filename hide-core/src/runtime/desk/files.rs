//! The files Desk's tab panel holds open (PRD desk-2-ui D-22, B19, B36).
//!
//! Each is the file document of its own checkout, read, edited and saved on
//! the same path the Workspace's file view uses (`prepare_file_tab`, the
//! document read on a worker, `file_save` by its tab id), but no Workspace's
//! View shows it because Desk asked, and opening it brings no checkout, device
//! or window forward. A Workspace that shows the same file shares the one
//! document; closing that Workspace's last display of it leaves it open while
//! Desk holds it, and Desk letting it go closes it unless a Workspace shows it
//! or it holds unsaved work. Which tab is in front, the preview tab and a
//! file shown twice side by side (B37) are the window's: the core holds each
//! file's document once.
//!
//! `hided` owns each hold: a window sends the whole set of files its room
//! shows (`desk_files`), at most 32 per connection, refused past that; hided
//! keeps which connections hold each file and lets a connection's files go
//! when it closes. The core hears one `desk_file` per window message, the
//! files no connection held before and the files no connection holds any
//! more, so `desk.files` is the union over the live connections, bounded by
//! their count times 32.

use serde::Deserialize;

use crate::model::EditorTabKind;
use crate::runtime::documents::OpenRequestFields;
use crate::runtime::editor::PreparedFileTab;
use crate::runtime::{ExplorerTarget, Runtime};

/// A file of a registered checkout, by the ids the catalog names it with.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime) struct DeskFile {
    workspace_id: String,
    checkout_id: String,
    path: String,
}

/// `desk_file`, sent by `hided` alone: the files a window's message brought
/// into Desk's hold and the ones it let go, applied as one event.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime) struct FilePayload {
    #[serde(default)]
    open: Vec<DeskFile>,
    #[serde(default)]
    close: Vec<DeskFile>,
}

/// `desk_file_retry`, from a window: the file and the device hided routed
/// it by, which the core reads from its catalog instead.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime) struct RetryPayload {
    workspace_id: String,
    checkout_id: String,
    path: String,
    #[serde(default)]
    #[allow(dead_code)]
    device_id: Option<String>,
}

impl From<RetryPayload> for DeskFile {
    fn from(payload: RetryPayload) -> Self {
        Self {
            workspace_id: payload.workspace_id,
            checkout_id: payload.checkout_id,
            path: payload.path,
        }
    }
}

const NO_CHECKOUT: &str = "The file's project or checkout is no longer available";

impl Runtime {
    pub(in crate::runtime) fn desk_file(&mut self, payload: FilePayload) -> bool {
        let on = self.desk.worker.is_some() && self.desk.stored.settings.enabled;
        if !on && !payload.open.is_empty() {
            crate::diagnostic!(serde_json::json!({
                "component": "desk",
                "kind": "desk.file_open_refused",
                "files": payload.open.len(),
                "reason": "desk_off",
            }));
        }
        let open = if on { payload.open } else { Vec::new() };
        let opened = open
            .into_iter()
            .fold(false, |changed, file| self.desk_file_open(file) | changed);
        payload
            .close
            .iter()
            .fold(opened, |changed, file| self.desk_file_close(file) | changed)
    }

    /// Opens `file`'s document without showing it in any View: an open
    /// document is shared, an unread one is read on a worker and joins the
    /// editor as a restored one does, and one that cannot be opened is a tab
    /// that says why (B28), which `desk_file_retry` reads again.
    fn desk_file_open(&mut self, file: DeskFile) -> bool {
        if !self.desk.files.contains(&file) {
            self.desk.files.push(file.clone());
        }
        self.desk_file_read(&file);
        crate::diagnostic!(serde_json::json!({
            "component": "desk",
            "kind": "desk.file_open",
            "checkout": file.checkout_id,
            "held": self.desk.files.len(),
        }));
        true
    }

    /// `desk_file_retry`: reads a held file that could not be opened again,
    /// as a View's Retry does (B16, B28).
    pub(in crate::runtime) fn desk_file_retry(&mut self, file: DeskFile) -> bool {
        let unavailable = self.desk.files.contains(&file)
            && self.snapshot.editor.tabs.iter().any(|tab| {
                tab.kind == EditorTabKind::File
                    && tab.workspace_id == file.workspace_id
                    && tab.checkout_id == file.checkout_id
                    && tab.path == file.path
                    && tab.unavailable_reason.is_some()
            });
        if !unavailable {
            return false;
        }
        self.desk_file_read(&file);
        true
    }

    fn desk_file_read(&mut self, file: &DeskFile) {
        if self
            .catalog_checkout(&file.workspace_id, &file.checkout_id)
            .is_none()
        {
            self.insert_unavailable_file_tab(
                &file.workspace_id,
                &file.checkout_id,
                &file.path,
                false,
                NO_CHECKOUT.to_owned(),
            );
            return;
        }
        match self.prepare_file_tab(&file.workspace_id, &file.checkout_id, &file.path) {
            // An open document is shared as it is; a Workspace's preview of
            // it stays a preview, since its retire skips what Desk holds.
            Ok(PreparedFileTab::Open(_)) => {}
            Ok(read @ PreparedFileTab::Read { .. }) => {
                self.insert_file_tab(
                    read,
                    &file.workspace_id,
                    &file.checkout_id,
                    &file.path,
                    false,
                );
            }
            Ok(PreparedFileTab::Reading { root, channel }) => self.start_document_open(
                root,
                channel,
                OpenRequestFields {
                    follow: false,
                    workspace_id: file.workspace_id.clone(),
                    checkout_id: file.checkout_id.clone(),
                    path: file.path.clone(),
                    preview: false,
                    reload: false,
                    reveal: None,
                    // It joins the editor without taking the screen, and a
                    // read that fails leaves a tab that says why.
                    restore: true,
                    placement: None,
                },
            ),
            Err(message) => self.insert_unavailable_file_tab(
                &file.workspace_id,
                &file.checkout_id,
                &file.path,
                false,
                message,
            ),
        }
    }

    /// Lets `file` go: its document closes unless a Workspace's View shows
    /// it or it holds unsaved work, which a View then shows as any open
    /// document with no display does.
    fn desk_file_close(&mut self, file: &DeskFile) -> bool {
        let Some(index) = self.desk.files.iter().position(|held| held == file) else {
            return false;
        };
        self.desk.files.remove(index);
        let Some(tab_id) = self.desk_file_tab(file) else {
            self.cancel_document_read(&file.workspace_id, &file.checkout_id, &file.path);
            return true;
        };
        let shown = self
            .every_view()
            .any(|(_, display)| display.tab_id.as_deref() == Some(tab_id.as_str()));
        if shown || self.document_kept(&tab_id) {
            return true;
        }
        self.close_file_tab_now(&tab_id)
    }

    /// The editor tab of a file Desk holds, once its document is open.
    fn desk_file_tab(&self, file: &DeskFile) -> Option<String> {
        self.snapshot
            .editor
            .tabs
            .iter()
            .find(|tab| {
                tab.kind == EditorTabKind::File
                    && tab.workspace_id == file.workspace_id
                    && tab.checkout_id == file.checkout_id
                    && tab.path == file.path
            })
            .map(|tab| tab.id.clone())
    }

    /// Whether Desk holds the document of editor tab `tab_id`: no View takes
    /// it in for that, and a Workspace closing its last display of it leaves
    /// it open.
    pub(in crate::runtime) fn desk_holds_tab(&self, tab_id: &str) -> bool {
        self.snapshot.editor.tabs.iter().any(|tab| {
            tab.id == tab_id
                && tab.kind == EditorTabKind::File
                && self.desk.files.iter().any(|file| {
                    tab.workspace_id == file.workspace_id
                        && tab.checkout_id == file.checkout_id
                        && tab.path == file.path
                })
        })
    }

    /// The checkout Desk's Explorer changes: the registered checkout at
    /// `root` on `device`, which Desk's session shows whatever is in front
    /// (B35). Desk switched off owns no Explorer.
    pub(in crate::runtime) fn desk_explorer_target(
        &self,
        device: &str,
        root: &str,
    ) -> Option<ExplorerTarget> {
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
                    .map(|checkout| ExplorerTarget {
                        workspace_id: workspace.id.clone(),
                        checkout_id: checkout.id.clone(),
                        device_id: workspace.device_id.clone(),
                        root: checkout.path.clone(),
                        desk: true,
                    })
            })
    }

    /// The editor tabs of every file Desk holds whose document is open.
    pub(in crate::runtime) fn desk_file_tabs(&self) -> Vec<String> {
        self.desk
            .files
            .iter()
            .filter_map(|file| self.desk_file_tab(file))
            .collect()
    }
}
