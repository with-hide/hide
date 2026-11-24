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

use serde::Deserialize;

use crate::model::EditorTabKind;
use crate::runtime::documents::OpenRequestFields;
use crate::runtime::editor::PreparedFileTab;
use crate::runtime::{ExplorerTarget, Runtime};

/// The most files Desk holds at once; one more lets the oldest go first, as
/// the tab panel drops its oldest file tab.
pub(crate) const MAX_DESK_FILES: usize = 32;

/// A file of a registered checkout, by the ids the catalog names it with.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime) struct DeskFile {
    workspace_id: String,
    checkout_id: String,
    path: String,
}

/// `desk_file`: a file pressed in Desk opens, a file tab closed lets go, and
/// leaving Desk lets every file of the window's room go in one event.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, tag = "action", rename_all = "snake_case")]
pub(in crate::runtime) enum FilePayload {
    Open {
        workspace_id: String,
        checkout_id: String,
        path: String,
        /// The device the checkout is on, which `hided` routes a device's
        /// file by; the core reads the checkout's device from its catalog.
        #[serde(default)]
        #[allow(dead_code)]
        device_id: Option<String>,
        /// The preview file this open replaces in its tab (B36), let go in
        /// the same event.
        #[serde(default)]
        replace: Option<DeskFile>,
    },
    Close {
        files: Vec<DeskFile>,
    },
}

impl Runtime {
    pub(in crate::runtime) fn desk_file(&mut self, payload: FilePayload) -> bool {
        match payload {
            FilePayload::Open {
                workspace_id,
                checkout_id,
                path,
                device_id: _,
                replace,
            } => {
                let file = DeskFile {
                    workspace_id,
                    checkout_id,
                    path,
                };
                let changed = self.desk_file_open(file.clone());
                match replace {
                    Some(old) if old != file => self.desk_file_close(&old) | changed,
                    _ => changed,
                }
            }
            FilePayload::Close { files } => files
                .iter()
                .fold(false, |changed, file| self.desk_file_close(file) | changed),
        }
    }

    /// Opens `file`'s document without showing it in any View: an open
    /// document is shared, an unread one is read on a worker and joins the
    /// editor as a restored one does, its tab saying why when it cannot be
    /// read.
    fn desk_file_open(&mut self, file: DeskFile) -> bool {
        if self
            .catalog_checkout(&file.workspace_id, &file.checkout_id)
            .is_none()
        {
            self.set_error(
                "file.invalid_context",
                "The file's project or checkout is no longer available",
                false,
            );
            return true;
        }
        if !self.desk.files.contains(&file) {
            if self.desk.files.len() >= MAX_DESK_FILES {
                let oldest = self.desk.files[0].clone();
                self.desk_file_close(&oldest);
            }
            self.desk.files.push(file.clone());
        }
        match self.prepare_file_tab(&file.workspace_id, &file.checkout_id, &file.path) {
            // A preview tab Desk holds is kept: no later preview replaces it.
            Ok(PreparedFileTab::Open(tab_id)) => {
                self.promote_editor_tab(&tab_id);
            }
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
            Err(message) => {
                self.desk.files.retain(|held| held != &file);
                self.set_error("file.open_failed", message, true);
            }
        }
        crate::diagnostic!(serde_json::json!({
            "component": "desk",
            "kind": "desk.file_open",
            "checkout": file.checkout_id,
            "held": self.desk.files.len(),
        }));
        true
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

    /// A file Desk's Explorer created opens as a file Desk holds, from the
    /// same result, as the Workspace's created file opens in its View.
    pub(in crate::runtime) fn desk_open_created(&mut self, target: &ExplorerTarget, path: &str) {
        self.desk_file_open(DeskFile {
            workspace_id: target.workspace_id.clone(),
            checkout_id: target.checkout_id.clone(),
            path: path.to_owned(),
        });
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
