use serde::Deserialize;
use serde_json::Value;

use super::documents::CloseAfter;
use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct EventEnvelope {
    pub(super) schema_version: u32,
    pub(super) kind: String,
    pub(super) payload: Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct AttachmentPayload {
    pub request_id: String,
    pub pane_id: String,
    pub bracketed_paste: bool,
    #[serde(default)]
    pub clipboard: bool,
    #[serde(default)]
    pub paths: Vec<String>,
    /// The node whose own screen staged `paths` on its machine, for a pane
    /// of that machine: they are pasted as they are, never read or copied
    /// (PRD core-host-node-remote-core D-06). Only that node's relay may
    /// send it (hided `relay_attachment`).
    #[serde(default)]
    pub staged_on: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AttachmentCompletionPayload {
    pub request_id: String,
    pub pane_id: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AttachmentActionPayload {
    pub request_id: String,
    pub pane_id: String,
    pub action: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ClickPayload {
    pub(super) surface: Surface,
    pub(super) x: f64,
    pub(super) y: f64,
    pub(super) button: MouseButton,
    pub(super) click_count: u8,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum MouseButton {
    Left,
    Right,
}

#[derive(Debug, Deserialize)]
pub(super) struct FocusPanePayload {
    pub(super) pane_id: String,
}

/// Why the shell asked for a pane focus.
///
/// The read axis needs the two apart. An operator focus is the act the whole
/// read record rests on; a launch restore reinstates the selection the last
/// session ended on, which says nothing about whether the operator has looked
/// at what changed while the app was closed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum PaneFocusOrigin {
    Operator,
    Restore,
}

#[derive(Debug, Deserialize)]
pub(super) struct FocusPaneRequestPayload {
    pub(super) pane_id: String,
    pub(super) origin: PaneFocusOrigin,
    #[serde(default)]
    pub(super) request_id: Option<String>,
    /// Also makes this machine the device in front, in the same event, when
    /// the focus is accepted: an agent chosen from Main or the Agents list
    /// while another device is in front is one action (S6 B12, B21).
    #[serde(default)]
    pub(super) focus_device: bool,
    /// The page that sent an operator focus and the number it gave this one,
    /// so the snapshot can say how much of what the page sent it includes
    /// (`OperatorFocusAck`). Both or neither.
    #[serde(default)]
    pub(super) client_id: Option<String>,
    #[serde(default)]
    pub(super) sequence: Option<u64>,
    /// The press came from a list of what the operator has to do: a row
    /// with a View to look at brings that View forward too, in the same
    /// event (PRD unseen-agent-views B3, D-14).
    #[serde(default)]
    pub(super) reveal_unseen: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GithubRequestPayload {
    pub(super) workspace_id: String,
    #[serde(default)]
    pub(super) refresh: bool,
    /// The operator is looking at this project (its window came forward, or
    /// the project did): read it again if its answer is old
    /// (`Runtime::read_when_looked_at`). A refresh outranks it.
    #[serde(default)]
    pub(super) look: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OverviewRefreshPayload {
    pub(super) workspace_id: String,
}

/// Whether any window shows the request view; hided aggregates its
/// connections and sends the change (PRD overview-request-view D-32).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RequestViewPayload {
    pub(super) observing: bool,
}

/// Whether any window holding the keyboard draws the Workspace screen, as
/// the daemon sums every window's report; sent only when that changes (PRD
/// unseen-agent-views D-07).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WindowFocusPayload {
    pub(super) focused: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateWorkspacePayload {
    /// The device that holds the folder; this machine when absent. A
    /// device's own helper judges the folder (`Call::Registrable`).
    #[serde(default)]
    pub(super) device_id: Option<String>,
    pub(super) path: String,
    pub(super) label: String,
    pub(super) initialize_git: bool,
    /// Add a project's Create new project: the folder is made and a Git
    /// repository started in it before it is registered. hided has checked
    /// the path on its `$HOME` line; the worker judges what stands there
    /// again, because the check and the make are not one step.
    #[serde(default)]
    pub(super) new_folder: bool,
    /// The folder outside home the operator confirmed adding, as the request
    /// spells it (PRD wsl-device D-10). hided has judged this machine's; a
    /// device's helper judges its own with it (`Call::Registrable`).
    #[serde(default)]
    pub(super) confirmed_outside_home: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CloneRepositoryPayload {
    pub(super) url: String,
    /// The canonical folder hided's `$HOME` line checked.
    pub(super) parent: String,
    /// The folder the URL names, which hided found free under `parent`.
    pub(super) name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CancelRepositoryClonePayload {
    pub(super) id: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateTabPayload {
    #[serde(default)]
    pub(super) area_id: Option<String>,
    pub(super) workspace_id: String,
    #[serde(default)]
    pub(super) checkout_id: Option<String>,
    pub(super) label: String,
    /// The shell's id for keys typed while this tab is being created.
    #[serde(default)]
    pub(super) request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ProjectMemoryNavigatePayload {
    pub(super) device_id: String,
    pub(super) workspace_id: String,
    pub(super) checkout_id: String,
    pub(super) request_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct FocusCheckoutPayload {
    /// Sidebar row disclosure, committed with checkout focus. Omitted by other open entrypoints.
    #[serde(default)]
    pub(super) expanded: Option<bool>,
    /// A project row's fold belongs to the same admitted checkout intent.
    #[serde(default)]
    pub(super) project_expanded: Option<bool>,
    pub(super) workspace_id: String,
    pub(super) checkout_id: String,
    /// Also makes this machine the device in front when the checkout is
    /// accepted, as `FocusPaneRequestPayload::focus_device` does.
    #[serde(default)]
    pub(super) focus_device: bool,
    /// The checkout comes forward on this View display rather than on its
    /// Agent tab: its Workspace's View area shows and the display takes it,
    /// in the same event (Recent navigation). A display the Workspace no
    /// longer holds refuses the whole event, so nothing moves.
    #[serde(default)]
    pub(super) display_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FocusTabPayload {
    pub(super) workspace_id: String,
    pub(super) checkout_id: String,
    pub(super) tab_id: String,
    /// Also makes this machine the device in front when the tab is accepted,
    /// as `FocusCheckoutPayload::focus_device` does.
    #[serde(default)]
    pub(super) focus_device: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct RenameTabPayload {
    pub(super) request_id: String,
    pub(super) tab_id: String,
    pub(super) label: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReorderTabPayload {
    pub(super) workspace_id: String,
    pub(super) checkout_id: String,
    /// The strip entry being moved, by its strip id, not the Herdr tab id:
    /// the strip is what the operator dragged in and it holds both kinds.
    pub(super) tab_id: String,
    /// Where the entry ends up, as its index in the resulting strip.
    pub(super) to_index: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct FocusDevicePayload {
    pub(super) device_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SessionFoldPayload {
    pub(super) key: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct InactiveCheckoutsTogglePayload {
    pub(super) project_path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct InactiveProjectsTogglePayload {
    pub(super) device_id: String,
}

/// Folds or unfolds a project's checkouts in the Projects list. Absent
/// `expanded` flips the fold; present, it sets it, so a retry of the same
/// intent lands on the same state.
#[derive(Debug, Deserialize)]
pub(super) struct ProjectCheckoutsFoldPayload {
    pub(super) workspace_id: String,
    #[serde(default)]
    pub(super) expanded: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct CheckoutAgentsTogglePayload {
    pub(super) checkout_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoveWorkspacePayload {
    pub(super) workspace_id: String,
    /// The agents outside the project, spawned from its agents, that the
    /// operator chose to close first (PRD close-agent-subtree D-10).
    #[serde(default)]
    pub(super) close_descendant_pane_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetPrimaryCheckoutPayload {
    pub workspace_id: String,
    pub checkout_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct WorkspacePinSetPayload {
    pub(super) workspace_id: String,
    pub(super) pinned: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct RegisterDevicePayload {
    /// Empty for a WSL distribution, whose id and name the core gives it.
    #[serde(default)]
    pub(super) id: String,
    #[serde(default)]
    pub(super) label: String,
    #[serde(default)]
    pub(super) ssh_alias: String,
    /// The WSL distribution of this PC to register instead of an SSH alias
    /// (PRD wsl-device B4, D-20).
    #[serde(default)]
    pub(super) wsl_distro: Option<String>,
    /// The device's Herdr socket when its server is not at the default path.
    #[serde(default)]
    pub(super) herdr_socket_path: Option<String>,
    #[serde(default)]
    pub(super) herdr_session: Option<String>,
    /// The operator allowed Hide's helper on the device in the same form
    /// (PRD S5.5 B50); absent or false registers it without file access.
    #[serde(default)]
    pub(super) host_consent: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct DeviceHostConsentPayload {
    pub(super) device_id: String,
    pub(super) allow: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct DeviceHostRetryPayload {
    pub(super) device_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoveDevicePayload {
    pub(super) device_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TestDevicePayload {
    pub(super) device_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreatePanePayload {
    pub(super) tab_id: String,
    pub(super) cwd: String,
    pub(super) command: Option<String>,
    pub(super) direction: PaneSplitDirection,
    /// The shell's id for keys typed while this split is being made.
    #[serde(default)]
    pub(super) request_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ResizePanePayload {
    pub(super) pane_id: String,
    pub(super) direction: PaneResizeDirection,
    pub(super) amount: f32,
}

#[derive(Debug, Deserialize)]
pub(super) struct ToggleZoomPayload {
    pub(super) pane_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfirmedWorkspacePayload {
    pub(super) workspace_id: String,
    pub(super) confirmed: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfirmedTabPayload {
    pub(super) tab_id: String,
    pub(super) confirmed: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ConfirmedPanePayload {
    pub(super) pane_id: String,
    pub(super) confirmed: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct CheckCloseStatusPayload {
    pub(super) key: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PaneTargetPayload {
    pub(super) pane_id: String,
}

/// Fork agent from the pane menu: where the fork goes, chosen at the press,
/// who pressed it, and the shell's key request, which holds what is typed
/// until the fork's agent has started.
#[derive(Debug, Deserialize)]
pub(super) struct ForkPanePayload {
    pub(super) pane_id: String,
    pub(super) placement: crate::fork::ForkPlacement,
    pub(super) origin: ForkOrigin,
    #[serde(default)]
    pub(super) request_id: Option<String>,
}

/// Who asked for a fork. Only the operator's own press moves the screen and
/// the keyboard to it; any other caller's fork is made and waits where it
/// was put, as an agent's spawn does (#1271).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum ForkOrigin {
    Operator,
    Agent,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoteControlPayload {
    pub(super) target_id: String,
    pub(super) request_id: String,
    /// The request id always belongs to the remote transport and its dedupe
    /// receipt. Only relationship Open/Return additionally projects that
    /// receipt into B24's visible pane-focus outcome.
    #[serde(default)]
    pub(super) report_pane_focus_outcome: bool,
    /// Also makes the target the device in front once the request is sent,
    /// so choosing a device's Workspace or agent is one event that moves both
    /// or, refused, neither (S6 B21).
    #[serde(default)]
    pub(super) focus_device: bool,
    #[serde(flatten)]
    pub(super) request: RemoteControlRequest,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub(super) enum RemoteControlRequest {
    FocusPane {
        pane_id: String,
    },
    SplitPane {
        pane_id: String,
        direction: PaneSplitDirection,
        #[serde(default)]
        cwd: Option<String>,
    },
    TogglePaneZoom {
        pane_id: String,
    },
    ClosePane {
        pane_id: String,
        confirmed: bool,
    },
    /// `checkout_id` names the Herdr workspace on the host: a device's
    /// project can hold several (`device_catalog`). Without it,
    /// `workspace_id` must itself be a Herdr workspace's row.
    FocusWorkspace {
        workspace_id: String,
        #[serde(default)]
        checkout_id: Option<String>,
    },
    FocusTab {
        tab_id: String,
    },
    CreateTab {
        workspace_id: String,
        #[serde(default)]
        checkout_id: Option<String>,
        cwd: String,
        label: String,
        #[serde(default)]
        area_id: Option<String>,
    },
    CloseTab {
        tab_id: String,
        confirmed: bool,
    },
}

impl RemoteControlRequest {
    pub(super) fn pane_id(&self) -> Option<&str> {
        match self {
            Self::FocusPane { pane_id }
            | Self::SplitPane { pane_id, .. }
            | Self::TogglePaneZoom { pane_id }
            | Self::ClosePane { pane_id, .. } => Some(pane_id),
            Self::FocusWorkspace { .. }
            | Self::FocusTab { .. }
            | Self::CreateTab { .. }
            | Self::CloseTab { .. } => None,
        }
    }

    pub(super) fn confirmed(&self) -> bool {
        matches!(
            self,
            Self::ClosePane {
                confirmed: true,
                ..
            } | Self::CloseTab {
                confirmed: true,
                ..
            }
        )
    }

    pub(super) fn mutation_kind(&self) -> Option<&'static str> {
        match self {
            Self::SplitPane { .. } => Some("pane.split"),
            Self::TogglePaneZoom { .. } => Some("pane.zoom"),
            Self::ClosePane { .. } => Some("pane.close"),
            Self::CloseTab { .. } => Some("tab.close"),
            Self::FocusPane { .. }
            | Self::FocusWorkspace { .. }
            | Self::FocusTab { .. }
            | Self::CreateTab { .. } => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct FileOpenPayload {
    pub(super) path: String,
    pub(super) workspace_id: String,
    pub(super) checkout_id: String,
    /// Whether the open wants the checkout's preview slot: an Explorer single
    /// click does, a double-click, Cmd+P and every other entry point do not.
    #[serde(default)]
    pub(super) preview: bool,
    /// Open to the side: the View area next to the one in use, or a new one
    /// to its right (PRD S7 B4). Only a shell with View areas reads it.
    #[serde(default)]
    pub(super) beside: bool,
    /// The Workspace in front, named when the file is another checkout's: a
    /// terminal link opens it there as an ordinary tab and moves no checkout
    /// (`open_file_in_front_view`). Only a shell with View areas sends it.
    #[serde(default)]
    pub(super) view: Option<super::view_areas::ViewWorkspace>,
    /// The agent pane whose terminal link the operator clicked: the View
    /// is that agent's (PRD view-opener-feedback D-02).
    #[serde(default)]
    pub(super) opener_pane: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FileTabPayload {
    pub(super) tab_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct FileClosePayload {
    pub(super) tab_id: String,
    #[serde(default)]
    pub(super) pending_save: Option<FileSavePayload>,
}

/// One clicked path, already resolved on the filesystem by the shell.
///
/// The shell owns filesystem resolution because reading directory entries
/// under the runtime mutex is exactly what the performance guide forbids. The
/// core owns every screen effect the click has, and it owns them together:
/// the checkout switch, the panel, the tree and the editor tab land in one
/// event or the dispatch's fire-and-forget ordering would let the operator
/// see a half-applied reveal.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct RevealPathPayload {
    pub(super) path: String,
    pub(super) workspace_id: String,
    pub(super) checkout_id: String,
    pub(super) is_directory: bool,
    /// The agent pane whose terminal link named the path (D-02).
    #[serde(default)]
    pub(super) opener_pane: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoteFileListPayload {
    pub(super) target_id: String,
    pub(super) root_path: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct FileSavePayload {
    pub(super) tab_id: String,
    pub(super) path: String,
    pub(super) contents_utf8: String,
}

/// A new file or folder: the folder it goes in and the name it takes. The
/// root is the tree the shell drew it in, which the core checks against the
/// focused checkout before it trusts the parent to be inside it.
#[derive(Debug, Deserialize)]
pub(super) struct ExplorerCreatePayload {
    /// The device whose tree asked; absent for this machine.
    #[serde(default)]
    pub(super) device_id: Option<String>,
    pub(super) root: String,
    pub(super) parent: String,
    pub(super) name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PathRenamePayload {
    /// The device whose tree asked; absent for this machine.
    #[serde(default)]
    pub(super) device_id: Option<String>,
    pub(super) root: String,
    pub(super) path: String,
    pub(super) name: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PathMovePayload {
    /// The device whose tree asked; absent for this machine.
    #[serde(default)]
    pub(super) device_id: Option<String>,
    pub(super) root: String,
    pub(super) path: String,
    /// The folder the item lands in; the item keeps its name.
    pub(super) destination: String,
}

/// An item copied into a folder of the same checkout. The host names the
/// copy; the destination is the folder.
#[derive(Debug, Deserialize)]
pub(super) struct PathCopyPayload {
    /// The device whose tree asked; absent for this machine.
    #[serde(default)]
    pub(super) device_id: Option<String>,
    pub(super) root: String,
    pub(super) path: String,
    /// The folder the copy lands in.
    pub(super) destination: String,
}

/// The files and folders of a drop from the operator's file manager. Only
/// hided makes this event: a window sends `path_import` with the ids of the
/// files it uploaded, and hided turns each id into the staged path named
/// here and never forwards the window's own event.
#[derive(Debug, Deserialize)]
pub(super) struct PathImportPayload {
    /// The device whose tree asked; absent for this machine.
    #[serde(default)]
    pub(super) device_id: Option<String>,
    pub(super) root: String,
    /// The folder the items land in.
    pub(super) destination: String,
    pub(super) items: Vec<PathImportItem>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PathImportItem {
    /// Below the destination folder, `/` between names.
    pub(super) path: String,
    pub(super) directory: bool,
    /// Where hided staged a file's bytes on this machine; absent for a folder.
    #[serde(default)]
    pub(super) staged: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PathTrashPayload {
    /// The device whose tree asked; absent for this machine.
    #[serde(default)]
    pub(super) device_id: Option<String>,
    pub(super) root: String,
    pub(super) path: String,
    /// The row the tree selects once the item is gone: its next sibling,
    /// else its previous sibling, else its parent. The tree decides it
    /// because only the tree knows its own row order; the core still
    /// refuses one outside the checkout or inside the item.
    pub(super) select_after: String,
    /// The inode the tree read when it built the prompt, so the core moves
    /// the item the modal named and refuses one that replaced it while the
    /// modal was open. Absent when the tree could not read one; decimal
    /// text, as the listing sent it (`inode_text`).
    #[serde(default, deserialize_with = "crate::files::inode_text::deserialize")]
    pub(super) inode: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct FileViewPayload {
    pub(super) tab_id: String,
    pub(super) markdown_live: bool,
    pub(super) wrap: bool,
}

/// `tab_id` names the document the draft is for; the web shell always
/// sends it, because several documents show at once (PRD S7 A4). Absent,
/// the draft is the active tab's, as an older client sends it.
#[derive(Debug, Deserialize)]
pub(super) struct FileDraftPayload {
    #[serde(default)]
    pub(super) tab_id: Option<String>,
    pub(super) contents_utf8: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct FileConflictPayload {
    #[serde(default)]
    pub(super) tab_id: Option<String>,
    pub(super) action: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct UiStateUpdatePayload {
    #[serde(default)]
    pub(super) left_sidebar_visible: Option<bool>,
    /// Absent keeps the rail as it is: an older client does not carry it.
    #[serde(default)]
    pub(super) device_rail_visible: Option<bool>,
    #[serde(default)]
    pub(super) right_panel_visible: Option<bool>,
    #[serde(default)]
    pub(super) right_panel_section: Option<String>,
    pub(super) expanded_paths: Vec<String>,
    /// Absent keeps the devices' expansion: an older client does not carry it.
    #[serde(default)]
    pub(super) device_expanded_paths: Option<BTreeMap<String, Vec<String>>>,
    /// The project, checkout and agent-lineage folds are not here: each has
    /// its own event (`project_checkouts_fold`, `checkout_agents_toggle`,
    /// `agent_tree_toggle`), because a fold computed from the shell's last
    /// snapshot and sent as a set loses a toggle the core has not echoed yet.
    #[serde(default)]
    pub(super) collapsed_checkout_ids: Option<Vec<String>>,
    pub(super) selected_path: Option<String>,
    #[serde(default)]
    pub(super) shortcut_bindings: std::collections::BTreeMap<String, String>,
    /// Absent keeps the web shell's chords; only that shell sends them.
    #[serde(default)]
    pub(super) browser_shortcut_bindings: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    pub(super) workspace_registrations: Option<Vec<crate::model::WorkspaceRegistration>>,
    #[serde(default)]
    pub(super) device_registrations: Option<Vec<crate::model::DeviceRegistration>>,
    #[serde(default)]
    pub(super) accent_hex: Option<String>,
    /// Absent keeps the width: only the web shell's drag and reset send it.
    #[serde(default)]
    pub(super) sidebar_width: Option<u32>,
    #[serde(default)]
    pub(super) font_size: Option<f32>,
    /// Absent keeps the size: only Appearance's Terminal text slider sends it.
    #[serde(default)]
    pub(super) terminal_font_size: Option<f32>,
    /// Ephemeral observation hints for the core-owned provider usage timer.
    /// They ride the existing UI-state event but are never persisted.
    #[serde(default)]
    pub(super) usage_window_visible: Option<bool>,
    #[serde(default)]
    pub(super) usage_popover_open: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct SessionsFilterPayload {
    pub(super) provider: String,
    #[serde(default)]
    pub(super) query: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct ArchiveOpenPayload {
    pub(super) kind: String,
    pub(super) id: String,
    #[serde(default = "default_true")]
    pub(super) preview: bool,
}

fn default_true() -> bool {
    true
}

/// One operator action on a catalog Project's Memory (PRD
/// project-memory-v2 B19-B24), from its Memory tab. Cards are named by
/// number, as agents name them.
#[derive(Clone, Debug, Deserialize)]
pub(super) struct MemoryActionPayload {
    #[serde(default)]
    pub(super) numbers: Vec<u64>,
    pub(super) workspace_id: String,
    pub(super) action: crate::model::MemoryAction,
    /// The card a forget or an undo names.
    #[serde(default)]
    pub(super) number: Option<u64>,
    /// The history row an `undo_change` takes back.
    #[serde(default)]
    pub(super) change: Option<u64>,
}

/// Opens a catalog Project's Memory for a screen; none closes it.
#[derive(Debug, Default, Deserialize)]
pub(super) struct MemoryOpenPayload {
    #[serde(default)]
    pub(super) workspace_id: Option<String>,
}

/// Shows one card of the opened Project in full; none folds it.
#[derive(Debug, Deserialize)]
pub(super) struct MemoryExpandPayload {
    pub(super) workspace_id: String,
    #[serde(default)]
    pub(super) number: Option<u64>,
}

/// The Memory tab said how many older cards were cleared to make room, the
/// newest at `through_unix_ms`.
#[derive(Debug, Deserialize)]
pub(super) struct MemoryEvictionsSeenPayload {
    pub(super) workspace_id: String,
    pub(super) through_unix_ms: u64,
}

/// Searches the opened Project's cards; empty words clear the search.
#[derive(Debug, Deserialize)]
pub(super) struct MemorySearchPayload {
    pub(super) workspace_id: String,
    #[serde(default)]
    pub(super) query: String,
}

/// Which changed file the changes view is showing the diff for. `None`
/// deselects, which is what closing the diff means.
#[derive(Debug, Deserialize)]
pub(super) struct ChangesSelectPayload {
    /// Whether the row is in the committed group, which decides what its diff
    /// is taken against.
    #[serde(default)]
    pub(super) committed: bool,
    #[serde(default)]
    pub(super) path: Option<String>,
    /// Whether the diff opens in the checkout's preview slot, which a single
    /// click on a Changes row asks for.
    #[serde(default)]
    pub(super) preview: bool,
    /// Open to the side, as `FileOpenPayload::beside`.
    #[serde(default)]
    pub(super) beside: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct CleanupReviewPayload {
    pub(super) workspace_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CleanupConfirmPayload {
    pub(super) id: u64,
    /// Worktrees to remove with everything in them.
    #[serde(default)]
    pub(super) paths: Vec<String>,
    /// Build cache and dependency cells to empty.
    #[serde(default)]
    pub(super) cells: Vec<CleanupCellPayload>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct CleanupCellPayload {
    pub(super) path: String,
    pub(super) layer: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct GitWorktreeOpenPayload {
    pub(super) checkout_path: String,
}

/// `overview_open_section`: one checkout brought forward and the right
/// panel moved to `section` in the same event, so the screen never shows
/// the half of the move a refusal would leave (AGENTS.md, "a user action is
/// one event").
#[derive(Debug, Deserialize)]
pub(super) struct OverviewOpenSectionPayload {
    pub(super) checkout_path: String,
    pub(super) section: String,
}

/// `agent_start_in_checkout`: a new tab in the checkout's Herdr workspace
/// with the checkout as its cwd, and the provider the shell then starts in
/// it. `terminal` means the tab alone.
#[derive(Debug, Deserialize)]
pub(super) struct AgentStartInCheckoutPayload {
    #[serde(default)]
    pub(super) confirm_only: bool,
    #[serde(default)]
    pub(super) target: Option<super::group_starts::StartTarget>,
    /// The checkout to start in; absent for a Home start.
    #[serde(default)]
    pub(super) checkout_path: Option<String>,
    /// The device the checkout or Home is on; absent is this machine (PRD
    /// home-device-rail D-22).
    #[serde(default)]
    pub(super) device_id: Option<String>,
    /// Start in the device's Home rather than a checkout (D-04, D-13).
    #[serde(default)]
    pub(super) home: bool,
    pub(super) provider: String,
    /// The agent's first prompt, sent once the agent is ready (an issue
    /// started on a checkout that is not a new worktree).
    #[serde(default)]
    pub(super) prompt: Option<String>,
    /// The model the agent CLI is started with; absent is the CLI's own
    /// default (PRD home-device-rail D-18).
    #[serde(default)]
    pub(super) model: Option<String>,
    /// The start surface's own id for this request, stamped on the task
    /// receipt and on a refusal so that surface reads its own answer.
    #[serde(default)]
    pub(super) request_id: Option<String>,
    /// The session to resume rather than start (PRD link-graph B12): the
    /// provider's own resume arguments, and no prompt.
    #[serde(default)]
    pub(super) resume_session_id: Option<String>,
    /// The archived file selected by the caller, proved again off-lock.
    #[serde(default)]
    pub(super) resume_session_path: Option<String>,
    /// The start comes from a surface that is not the operator's own screen
    /// (the phone): the tab is made and listed, but the desktop's visible
    /// tab, keyboard focus and Herdr's focus stay where they are.
    #[serde(default)]
    pub(super) stay_behind: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct GitWorktreeSetBasePayload {
    pub(super) repository_root: String,
    pub(super) branch: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct CreateWorktreePayload {
    #[serde(default)]
    pub(super) request_id: Option<String>,
    /// The device that holds the repository; this machine when absent.
    #[serde(default)]
    pub(super) device_id: Option<String>,
    pub(super) repository_root: String,
    pub(super) branch: String,
    #[serde(default)]
    pub(super) base_branch: Option<String>,
    #[serde(default)]
    pub(super) agent_kind: Option<String>,
    #[serde(default)]
    pub(super) purpose: Option<String>,
    /// The task the worktree is started from; the worktree is linked to it.
    #[serde(default)]
    pub(super) task_key: Option<String>,
    /// The agent's first prompt, sent once the agent is ready.
    #[serde(default)]
    pub(super) prompt: Option<String>,
    /// The model the agent CLI is started with; absent is its own default.
    #[serde(default)]
    pub(super) model: Option<String>,
}

/// `issue_source_set`: a project's issue source chosen in Settings › Issues;
/// `auto` returns it to the default (`tasks::source_kind`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IssueSourceSetPayload {
    pub(super) project_path: String,
    pub(super) source: String,
}

/// `conductor_settings_set`: the fields of Settings › Conductor one edit
/// changes (PRD software-factory-conductor D-31, B48); an absent field keeps
/// its value, so two edits sent before the first one's echo do not undo each
/// other.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConductorSettingsSetPayload {
    #[serde(default)]
    pub(super) concurrency: Option<u8>,
    #[serde(default)]
    pub(super) default_scope: Option<crate::missions::MissionScope>,
    #[serde(default)]
    pub(super) candidates: Option<Vec<crate::missions::Candidate>>,
    #[serde(default)]
    pub(super) default_strength: Option<crate::missions::Strength>,
    #[serde(default)]
    pub(super) other_vendor: Option<bool>,
}

/// `mission_answer`: the operator's answer from a mission's decision banner
/// (PRD mission-plan-visibility B26, B27): the option it pressed, counted
/// from 1, or what it wrote, for the question with `key`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MissionAnswerPayload {
    pub(super) mission: String,
    pub(super) key: u64,
    #[serde(default)]
    pub(super) option: Option<u8>,
    #[serde(default)]
    pub(super) text: Option<String>,
    /// Types an answer again that was typed without Herdr confirming it.
    #[serde(default)]
    pub(super) resend: bool,
}

/// `issue_settings_set`: how starting work from an issue behaves; an absent
/// field keeps its value.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IssueSettingsSetPayload {
    #[serde(default)]
    pub(super) ai_worktree_name: Option<bool>,
    #[serde(default)]
    pub(super) closes_instruction: Option<bool>,
}

/// `issue_detail_request`: one issue's body, labels, author, assignees and
/// comments, for its panel and the Start dialog.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IssueDetailRequestPayload {
    pub(super) workspace_id: String,
    pub(super) task_key: String,
    #[serde(default)]
    pub(super) comments_after: Option<String>,
}

/// `github_search`: pull requests and issues of this Mac's GitHub projects
/// matching `query`, answered in `issue_work.search` under `request_id`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GithubSearchPayload {
    pub(super) request_id: String,
    pub(super) query: String,
}

/// `local_issue_update`: a Local issue's title and body, edited in its panel.
/// A GitHub issue has no such event; it is edited on GitHub.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LocalIssueUpdatePayload {
    pub(super) request_id: String,
    /// The project the issue is in: a node's project and this machine's may
    /// share a path, and the key names only the path.
    pub(super) workspace_id: String,
    pub(super) task_key: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) body: String,
}

/// `pr_link_issue`: a pull request linked to an issue of its project (PRD
/// overview-lenses-prs D-13, D-31, D-34): an issue the project's source has,
/// by `issue_key`, or a new one made from `new_issue`. A GitHub issue is
/// linked in Hide and closed by `Closes #N` in the pull request's body; a
/// Local one is linked in Hide only. A retry sends the same issue again.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrLinkIssuePayload {
    pub(super) request_id: String,
    pub(super) workspace_id: String,
    pub(super) pr_number: u32,
    #[serde(default)]
    pub(super) issue_key: Option<String>,
    #[serde(default)]
    pub(super) new_issue: Option<PrNewIssuePayload>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrNewIssuePayload {
    pub(super) title: String,
    #[serde(default)]
    pub(super) body: String,
}

/// `pr_feedback_read`: a pull request's failed checks and change requests,
/// read for an agent's first prompt (D-46).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrFeedbackReadPayload {
    pub(super) request_id: String,
    pub(super) workspace_id: String,
    pub(super) pr_number: u32,
}

/// `pr_card_read`: what the card shows beyond the list (D-06), asked when
/// the card opens, when its pull request changed, and by a part's retry.
/// `look_again` is the operator's Look again after a refused write, which
/// also dismisses its notice and reads the list; no automatic read does.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrCardReadPayload {
    pub(super) workspace_id: String,
    pub(super) pr_number: u32,
    #[serde(default)]
    pub(super) look_again: bool,
}

/// `pr_merge`: Merge from the card (D-21, D-22), conditioned on the head
/// commit the operator was shown. A repository with a merge queue takes no
/// `method`; any other takes one of the methods the card offered.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrMergePayload {
    pub(super) workspace_id: String,
    pub(super) pr_number: u32,
    pub(super) head: String,
    #[serde(default)]
    pub(super) method: Option<hide_node_link::gh::PrMergeMethod>,
}

/// `pr_update_branch`: brings a BEHIND pull request's branch up to its base,
/// conditioned on the head commit the operator was shown (D-22).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrUpdateBranchPayload {
    pub(super) workspace_id: String,
    pub(super) pr_number: u32,
    pub(super) head: String,
}

/// `pr_open`: a pull request's decision card (PRD work-tab-pr-lens D-02,
/// D-03, D-28), opened in one event: the Workspace it opens in, the agent
/// brought forward and the card tab, all chosen by the core. `checkout_id`
/// is the Workspace the operator asked from, when it is one of the project's.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrOpenPayload {
    pub(super) workspace_id: String,
    pub(super) pr_number: u32,
    #[serde(default)]
    pub(super) checkout_id: Option<String>,
    /// Asked with no Workspace in front (Home): the pull request's
    /// Workspace opens too, with the card in its lane (PRD
    /// sidebar-task-lens B17). Otherwise the screen stays where it is.
    #[serde(default)]
    pub(super) open_workspace: bool,
}

/// `pr_delegate`: an agent started on a pull request's branch with the
/// operator's first prompt (D-12, D-46): in the checkout on that branch, or
/// in a new worktree of the existing branch when there is none.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrDelegatePayload {
    pub(super) workspace_id: String,
    pub(super) pr_number: u32,
    pub(super) provider: String,
    #[serde(default)]
    pub(super) prompt: String,
    /// The model the agent CLI is started with; absent is its own default.
    #[serde(default)]
    pub(super) model: Option<String>,
}

/// `issue_set_open`: close or reopen a Local issue.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IssueSetOpenPayload {
    /// The project the issue is in, as in `LocalIssueUpdatePayload`.
    pub(super) workspace_id: String,
    pub(super) task_key: String,
    pub(super) open: bool,
}

/// `worktree_name_suggest`: ask the background AI to name a worktree started
/// from an issue. `prefix` is the part the name keeps (`192-`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WorktreeNameSuggestPayload {
    pub(super) request_id: String,
    pub(super) prefix: String,
    pub(super) title: String,
    #[serde(default)]
    pub(super) body: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct SetCheckoutPurposePayload {
    pub(super) checkout_id: String,
    pub(super) text: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct MigrateMainBranchPayload {
    pub(super) repository_root: String,
    pub(super) base_branch: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TaskOperationAckPayload {
    pub(super) id: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct TaskAgentRetryPayload {
    pub(super) id: u64,
}

#[derive(Debug, Deserialize)]
pub(super) struct RemoveWorktreePayload {
    /// The device that holds the worktree; this machine when absent.
    #[serde(default)]
    pub(super) device_id: Option<String>,
    pub(super) checkout_path: String,
    #[serde(default)]
    pub(super) delete_branch: bool,
    /// The operator ticked the discard checkbox the gate offered.
    #[serde(default)]
    pub(super) discard_changes: bool,
    /// The measured repository names the operator actually confirmed.
    #[serde(default)]
    pub(super) expected_ignored_repositories: Vec<String>,
    /// The agents outside the worktree, spawned from its agents, that the
    /// operator chose to close first (PRD close-agent-subtree D-10).
    #[serde(default)]
    pub(super) close_descendant_pane_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct RetryConnectPayload {
    pub(super) target_id: String,
}

/// The operator pressed Reinstall (PRD device-parity B8): on a machine's
/// row, which repairs every part of it that needs it, or on one hook row,
/// which names that part. `device_id` is the core's own node id for this Mac.
#[derive(Debug, Deserialize)]
pub(super) struct KitReinstallPayload {
    pub(super) device_id: String,
    #[serde(default)]
    pub(super) components: Option<Vec<hide_kit::ComponentId>>,
    /// Agents to repair, by adapter id; with `components`, only what is
    /// named is repaired.
    #[serde(default)]
    pub(super) agents: Option<Vec<String>>,
}

/// The operator asked, from a not connected Codex pane's popover, for Codex's
/// shared server to be turned off on one machine (PRD settings-cleanup B27).
/// `device_id` is the core's own node id for this Mac.
#[derive(Debug, Deserialize)]
pub(super) struct CodexDaemonDisablePayload {
    pub(super) device_id: String,
}

/// The operator switched an agent on or off from its row (issue #517); every
/// adapter id has a switch, whether or not the agent is set up there.
#[derive(Debug, Deserialize)]
pub(super) struct KitAgentSetPayload {
    pub(super) device_id: String,
    pub(super) agent: String,
    pub(super) enabled: bool,
}

/// A Settings tab that shows the kit opened: this Mac's parts are read again
/// once, so a part removed by hand shows as removed.
#[derive(Debug, Deserialize)]
pub(super) struct KitCheckPayload {}

/// Add device opened: the account's Host entries are listed once, with where
/// each one leads (`status.ssh_hosts`).
#[derive(Debug, Deserialize)]
pub(super) struct SshHostsListPayload {}

#[derive(Debug, Deserialize)]
pub(super) struct HerdrProfilesListPayload {}

#[derive(Debug, Deserialize)]
pub(super) struct HerdrProfileConnectPayload {
    pub(super) profile_id: String,
    pub(super) host_consent: bool,
}

/// Whether any window draws this daemon's snapshot, sent by hided as its
/// first window arrives and its last one leaves. Without one, the readers
/// that only feed what a window draws (ports, worktrees, pull requests,
/// disk, foreground processes) rest, and label work alone goes on (PRD
/// labels-in-hided B29).
#[derive(Debug, Deserialize)]
pub(super) struct UiAttachedPayload {
    pub(super) attached: bool,
}

/// One Hide AI settings event, carrying whatever it is about.
///
/// It folds what a single screen does into one event, the way
/// `ui_state_update` already folds that screen's other state: the tab
/// appearing or going away, Use Hide AI, a chosen agent, a chosen model (for
/// Runs on or for an agent in the fallback list), and the fallback list's
/// additions and removals. Nothing here is filled in on the core's side, so
/// an event that names a model without its provider is refused rather than
/// guessed at.
#[derive(Deserialize)]
pub(super) struct AiSettingsPayload {
    /// True while the Background AI group is on screen. The provider probe
    /// starts child processes, so it runs only while somebody is looking.
    #[serde(default)]
    pub(super) observing: Option<bool>,
    /// True while a start surface shows its model menu (PRD home-device-rail
    /// D-18): the catalog is read, the Settings-only hook diagnosis is not.
    #[serde(default)]
    pub(super) start_observing: Option<bool>,
    /// The provider the operator chose.
    #[serde(default)]
    pub(super) provider: Option<String>,
    /// The model for `provider`; never for whichever provider happens to be
    /// selected.
    #[serde(default)]
    pub(super) model: Option<String>,
    /// The agent-summary switch (PRD overview-request-view D-11).
    #[serde(default)]
    pub(super) agent_summary: Option<bool>,
    /// Use Hide AI (D-14): off, no model is asked anything.
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    /// An agent to add under "If <Runs on> can't answer" (D-16), at the end
    /// of the list.
    #[serde(default)]
    pub(super) fallback_add: Option<String>,
    /// An agent to take out of that list.
    #[serde(default)]
    pub(super) fallback_remove: Option<String>,
    /// The connected device the agent `provider` or `fallback_add` names
    /// runs on, whichever the event carries; absent or empty is the core's
    /// own machine (PRD wsl-device B11).
    #[serde(default)]
    pub(super) device: Option<String>,
    /// The Usage limit (PRD hide-ai-usage-limit D-01): a percent from 1 to
    /// 100, or `"none"`. A value that is neither is refused with the event.
    #[serde(default)]
    pub(super) usage_limit: Option<hide_ai::UsageLimit>,
}

/// One pane search. An empty `term` clears the search rather than needing its
/// own event, and `step` folds "search this" and "go to the next one" into one
/// path: 0 searches and keeps the current match, +1 and -1 move.
#[derive(Debug, Deserialize)]
pub(super) struct PaneFindPayload {
    pub(super) pane_id: String,
    pub(super) term: String,
    #[serde(default)]
    pub(super) case_sensitive: bool,
    #[serde(default)]
    pub(super) whole_word: bool,
    #[serde(default)]
    pub(super) regex: bool,
    #[serde(default)]
    pub(super) step: i64,
}

/// ⌘F on a pane whose agent has its own find (`own_find` on its row).
#[derive(Debug, Deserialize)]
pub(super) struct PaneFindOpenPayload {
    pub(super) pane_id: String,
    pub(super) request_id: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct PaneTextScalePayload {
    pub(super) pane_id: String,
    pub(super) direction: String,
}

/// The editor is one surface, not one per document, so its zoom carries a
/// direction and nothing to key it by.
#[derive(Debug, Deserialize)]
pub(super) struct EditorTextScalePayload {
    pub(super) direction: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentOnboardingApplyPayload {
    pub(super) agents: Vec<String>,
    /// A browser tab shows only the wizard's first step, so applying it
    /// finishes the wizard there (PRD first-run-onboarding B26).
    #[serde(default)]
    pub(super) finish: bool,
}

/// Leaves the wizard step named `from`; a client that still shows an earlier
/// step moves nothing (engineering rule 11).
#[derive(Debug, Deserialize)]
pub(super) struct FirstRunAdvancePayload {
    pub(super) from: crate::model::FirstRunStep,
    /// The desktop switch the notification step leaves with: on for Continue
    /// on a Mac that already allows Hide (B16), off for Not now (B21), absent
    /// to leave it as it is.
    #[serde(default)]
    pub(super) notifications: Option<bool>,
}

/// The operator's answer to moving their older Herdr server (PRD
/// herdr-takeover): one press, one event.
#[derive(Debug, Deserialize)]
pub(super) struct HerdrTakeoverPayload {
    pub(super) action: super::herdr_takeover::TakeoverAction,
}

#[derive(Debug, Deserialize)]
pub(super) struct FirstRunBackPayload {
    pub(super) from: crate::model::FirstRunStep,
}

#[derive(Debug, Deserialize)]
pub(super) struct FirstRunDraftPayload {
    pub(super) agents: Option<Vec<String>>,
    pub(super) devices: Option<Vec<String>>,
    pub(super) project_device: Option<String>,
    pub(super) project_path: Option<String>,
    pub(super) project_form: Option<crate::model::FirstRunProjectForm>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopNotificationsSetPayload {
    pub(super) enabled: bool,
    /// The reopened wizard's Not now disables both with its one event.
    #[serde(default)]
    pub(super) done_enabled: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub(super) struct DesktopNotificationsDoneSetPayload {
    pub(super) enabled: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ScreenIntroSeenPayload {
    pub(super) screen: crate::model::ScreenIntro,
}

#[derive(Debug, Deserialize)]
pub(super) struct UsagePinSetPayload {
    pub(super) agent: String,
    pub(super) window: hide_agent_adapter::usage::WindowKind,
    pub(super) enabled: bool,
}
#[derive(Debug, Deserialize)]
pub(super) struct UsageHiddenSetPayload {
    pub(super) agent: String,
    pub(super) hidden: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct InterfaceLanguageSetPayload {
    // Required, including an explicit null to return to system language.
    #[serde(deserialize_with = "serde_json::Value::deserialize")]
    pub(super) language: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub(super) struct ProductAnalyticsSetPayload {
    pub(super) enabled: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct ThemeSetPayload {
    pub(super) theme: crate::model::ThemePreference,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentLogoModeSetPayload {
    pub(super) mode: crate::model::AgentLogoMode,
}

#[derive(Debug, Deserialize)]
pub(super) struct PaneTurnRowModeSetPayload {
    pub(super) mode: crate::model::PaneTurnRowMode,
}

#[derive(Debug, Deserialize)]
pub(super) struct ColorThemeSetPayload {
    pub(super) color_theme: crate::model::ColorTheme,
}

#[derive(Debug, Deserialize)]
pub(super) struct PetVisibilityPayload {
    pub(super) visible: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct PetMovePayload {
    /// Already clamped to a visible screen by the shell, which owns the
    /// display geometry; the core only records where the pet ended up.
    pub(super) x: f64,
    pub(super) y: f64,
}

#[derive(Debug, Deserialize)]
pub(super) struct PetDragPayload {
    pub(super) dragging: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct PetShortcutPayload {
    /// `null` or blank means the user cleared the binding: nothing is
    /// registered and no shortcut fires (D-14).
    pub(super) accelerator: Option<String>,
    #[serde(default)]
    pub(super) error: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct TerminalScrollPayload {
    pub(super) pane_id: String,
    pub(super) direction: String,
    pub(super) lines: u16,
    #[serde(default)]
    pub(super) column: Option<u16>,
    #[serde(default)]
    pub(super) row: Option<u16>,
    #[serde(default)]
    pub(super) modifiers: u8,
}

#[derive(Debug, Deserialize)]
pub(super) struct TerminalClickPayload {
    pub(super) pane_id: String,
    pub(super) column: u16,
    pub(super) row: u16,
    pub(super) modifiers: u8,
}

#[derive(Debug, Deserialize)]
pub(super) struct TerminalResizePayload {
    pub(super) pane_id: String,
    pub(super) cols: u16,
    pub(super) rows: u16,
}

pub(super) enum Event {
    WorkspaceView(WorkspaceViewPayload),
    ViewLayout(ViewLayoutPayload),
    AgentLayout(AgentLayoutPayload),
    PaneMove(super::pane_move::Request),
    PaneMoveCheck(super::pane_move::Check),
    PaneMovePreview(super::pane_move_preview::Request),
    PaneMovePreviewCancel(super::pane_move_preview::Cancel),
    AgentGroup(AgentGroupPayload),
    BrowserOpen(BrowserOpenPayload),
    BrowserState(BrowserStatePayload),
    ExternalOpen(ExternalOpenPayload),
    ExternalGrants(ExternalGrantsPayload),
    Attachment(AttachmentPayload),
    AttachmentReady(AttachmentCompletionPayload),
    AttachmentAction(AttachmentActionPayload),
    SessionSnapshot(SessionSnapshotPayload),
    RefreshStatus,
    Click(ClickPayload),
    FocusPane(FocusPaneRequestPayload),
    CreateWorkspace(CreateWorkspacePayload),
    CreateTab(CreateTabPayload),
    FocusCheckout(FocusCheckoutPayload),
    FocusTab(FocusTabPayload),
    ReorderTab(ReorderTabPayload),
    RenameTab(RenameTabPayload),
    FocusDevice(FocusDevicePayload),
    InactiveCheckoutsToggle(InactiveCheckoutsTogglePayload),
    SessionFoldToggle(SessionFoldPayload),
    InactiveProjectsToggle(InactiveProjectsTogglePayload),
    ProjectCheckoutsFold(ProjectCheckoutsFoldPayload),
    CheckoutAgentsToggle(CheckoutAgentsTogglePayload),
    WorkspacePinSet(WorkspacePinSetPayload),
    SetPrimaryCheckout(SetPrimaryCheckoutPayload),
    RemoveWorkspace(RemoveWorkspacePayload),
    RegisterDevice(RegisterDevicePayload),
    DeviceHostConsent(DeviceHostConsentPayload),
    DeviceHostRetry(DeviceHostRetryPayload),
    RemoveDevice(RemoveDevicePayload),
    TestDevice(TestDevicePayload),
    CreatePane(CreatePanePayload),
    ResizePane(ResizePanePayload),
    ToggleZoom(ToggleZoomPayload),
    ToggleConversation(PaneTargetPayload),
    CloseWorkspace(ConfirmedWorkspacePayload),
    CloseTab(ConfirmedTabPayload),
    ClosePane(ConfirmedPanePayload),
    CloseTree(super::tree_close::CloseTreePayload),
    CheckCloseStatus(CheckCloseStatusPayload),
    RetryAgentClose(CheckCloseStatusPayload),
    DismissAgentClose(CheckCloseStatusPayload),
    ReopenClosed,
    ForkPane(ForkPanePayload),
    AgentSleepSet(AgentSleepSetPayload),
    AgentSleep(PaneTargetPayload),
    ArchiveSession(super::session_archive::ArchiveSessionPayload),
    ArchiveResume(super::session_archive::ArchiveTargetPayload),
    ArchiveDelete(super::session_archive::ArchiveTargetPayload),
    ArchiveCheck(super::session_archive::ArchiveTargetPayload),
    PaneInputSubmitted(PaneTargetPayload),
    PaneInputSent(PaneTargetPayload),
    ResolveSession(PaneTargetPayload),
    RevisitStar(PaneTargetPayload),
    RevisitDone(PaneTargetPayload),
    DeliverLettersNow(PaneTargetPayload),
    PaneVisit(PaneTargetPayload),
    AgentWake(AgentWakePayload),
    WakeSleepingSession(SleepingSessionPayload),
    CheckSleepingSession(SleepingSessionPayload),
    PaneReopen(PaneTargetPayload),
    AgentTreeToggle(PaneTargetPayload),
    SessionTreeToggle(PaneTargetPayload),
    SidebarLensSet(super::detail::SidebarLensPayload),
    TaskViewSet(super::detail::TaskViewPayload),
    SidebarProjectSet(super::detail::SidebarProjectPayload),
    PracticeProgressSet(super::detail::PracticeProgressPayload),
    PracticeInviteClose,
    OverviewGraphChange(super::overview_graph::OverviewGraphChangePayload),
    TaskReveal(super::detail::TaskRevealPayload),
    DetailOpen(super::detail::DetailOpenPayload),
    DetailBack,
    DetailClose,
    DetailShow(super::detail::DetailShowPayload),
    RemoteControl(RemoteControlPayload),
    RemoteFileList(RemoteFileListPayload),
    FileOpen(FileOpenPayload),
    RevealPath(RevealPathPayload),
    ViewComment(super::view_feedback::ViewCommentPayload),
    ViewFeedbackSend(super::view_feedback::ViewFeedbackSendPayload),
    FileFocus(FileTabPayload),
    /// Keep Open: the preview tab becomes an ordinary tab in the same slot.
    FileKeepOpen(FileTabPayload),
    FileClose(FileClosePayload),
    FileDraft(FileDraftPayload),
    FileView(FileViewPayload),
    FileSave(FileSavePayload),
    FileConflict(FileConflictPayload),
    FileCreate(ExplorerCreatePayload),
    DirCreate(ExplorerCreatePayload),
    PathRename(PathRenamePayload),
    PathMove(PathMovePayload),
    PathCopy(PathCopyPayload),
    PathImport(PathImportPayload),
    PathTrash(PathTrashPayload),
    UiStateUpdate(Box<UiStateUpdatePayload>),
    SessionsRefresh,
    LinksOpen(super::links::LinksOpenPayload),
    LinksClose,
    SessionRecordOpen(super::links::RecordOpenPayload),
    SessionRecordClose,
    SessionWorkSearch(super::links::RecordSearchPayload),
    SessionMissedStatus(super::links::MissedStatusPayload),
    BacklogAdd(super::backlog::BacklogAddPayload),
    BacklogRetry(super::backlog::BacklogIntentPayload),
    BacklogDismiss(super::backlog::BacklogIntentPayload),
    BacklogUndo(super::backlog::BacklogItemPayload),
    BacklogOrigin(crate::links::ledger::BacklogOriginIdentity),
    BacklogOriginClose(crate::links::ledger::BacklogOriginIdentity),
    SessionsSetFilter(SessionsFilterPayload),
    ArchiveOpen(ArchiveOpenPayload),
    MemoryAction(MemoryActionPayload),
    MemoryOpen(MemoryOpenPayload),
    SuggestionAction(super::suggestions::SuggestionActionPayload),
    SuggestionEvidence(super::suggestion_evidence::SuggestionEvidencePayload),
    SuggestionSettings(super::suggestions::SuggestionSettingsPayload),
    HideAiSwitch(super::suggestions::HideAiSwitchPayload),
    SuggestionsIntroSeen,
    HideAiWake(super::suggestions::HideAiWakePayload),
    ProjectMemoryNavigate(ProjectMemoryNavigatePayload),
    MemoryExpand(MemoryExpandPayload),
    MemoryEvictionsSeen(MemoryEvictionsSeenPayload),
    MemorySearch(MemorySearchPayload),
    RetryConnect(RetryConnectPayload),
    CloneRepository(CloneRepositoryPayload),
    CancelRepositoryClone(CancelRepositoryClonePayload),
    KitReinstall(KitReinstallPayload),
    KitAgentSet(KitAgentSetPayload),
    CodexDaemonDisable(CodexDaemonDisablePayload),
    KitCheck(KitCheckPayload),
    SshHostsList(SshHostsListPayload),
    HerdrProfilesList(HerdrProfilesListPayload),
    HerdrProfileConnect(HerdrProfileConnectPayload),
    UiAttached(UiAttachedPayload),
    AiSettings(AiSettingsPayload),
    TerminalResize(TerminalResizePayload),
    TerminalScroll(TerminalScrollPayload),
    TerminalClick(TerminalClickPayload),
    PaneFind(PaneFindPayload),
    PaneFindOpen(PaneFindOpenPayload),
    PaneTextScale(PaneTextScalePayload),
    EditorTextScale(EditorTextScalePayload),
    ProductAnalyticsSet(ProductAnalyticsSetPayload),
    ProductAnalyticsConnected,
    ThemeSet(ThemeSetPayload),
    AgentLogoModeSet(AgentLogoModeSetPayload),
    PaneTurnRowModeSet(PaneTurnRowModeSetPayload),
    ColorThemeSet(ColorThemeSetPayload),
    InterfaceLanguageSet(InterfaceLanguageSetPayload),
    UsagePinSet(UsagePinSetPayload),
    UsageHiddenSet(UsageHiddenSetPayload),
    AgentOnboardingApply(AgentOnboardingApplyPayload),
    FirstRunAdvance(FirstRunAdvancePayload),
    FirstRunBack(FirstRunBackPayload),
    HerdrTakeover(HerdrTakeoverPayload),
    FirstRunDraft(FirstRunDraftPayload),
    DesktopNotificationsSet(DesktopNotificationsSetPayload),
    DesktopNotificationsDoneSet(DesktopNotificationsDoneSetPayload),
    ScreenIntroSeen(ScreenIntroSeenPayload),
    ChangesSelect(ChangesSelectPayload),
    GitWorktreeOpen(GitWorktreeOpenPayload),
    GitWorktreeSetBase(GitWorktreeSetBasePayload),
    CreateWorktree(CreateWorktreePayload),
    IssueSourceSet(IssueSourceSetPayload),
    ProjectSettingsSet(super::project_settings::SetPayload),
    IssueSettingsSet(IssueSettingsSetPayload),
    ConductorSettingsSet(ConductorSettingsSetPayload),
    MissionAnswer(MissionAnswerPayload),
    IssueDetailRequest(IssueDetailRequestPayload),
    GithubSearch(GithubSearchPayload),
    WorktreeNameSuggest(WorktreeNameSuggestPayload),
    IssueSetOpen(IssueSetOpenPayload),
    LocalIssueUpdate(LocalIssueUpdatePayload),
    PrLinkIssue(PrLinkIssuePayload),
    PrFeedbackRead(PrFeedbackReadPayload),
    PrDelegate(PrDelegatePayload),
    PrOpen(PrOpenPayload),
    PrCardRead(PrCardReadPayload),
    PrMerge(PrMergePayload),
    PrUpdateBranch(PrUpdateBranchPayload),
    SetCheckoutPurpose(SetCheckoutPurposePayload),
    SetCheckoutIssue(SetCheckoutPurposePayload),
    MigrateMainBranch(MigrateMainBranchPayload),
    TaskOperationAck(TaskOperationAckPayload),
    TaskAgentRetry(TaskAgentRetryPayload),
    RemoveWorktree(RemoveWorktreePayload),
    GithubRequest(GithubRequestPayload),
    /// The operator's window came forward (PRD git-freshness D-02).
    FetchLook,
    OverviewRefresh(OverviewRefreshPayload),
    OverviewOpenResult(PaneTargetPayload),
    RequestView(RequestViewPayload),
    WindowFocus(WindowFocusPayload),
    /// The card's refresh button, opening the delete confirmation, and a
    /// completed worktree removal. All three say "read again now" about a
    /// different set of readers, and none needs a target: the card is always
    /// the selected checkout, and a removal changes the whole worktree list.
    OverviewOpenSection(OverviewOpenSectionPayload),
    AgentStartInCheckout(AgentStartInCheckoutPayload),
    CleanupReview(CleanupReviewPayload),
    CleanupConfirm(CleanupConfirmPayload),
    CleanupDismiss,
    CardRefresh,
    DiskWatch(super::disk_sweep::DiskWatchPayload),
    ReconnectPane(FocusPanePayload),
    PetSetVisible(PetVisibilityPayload),
    PetToggleVisible,
    PetMove(PetMovePayload),
    PetDrag(PetDragPayload),
    PetActivity,
    PetShortcutUpdate(PetShortcutPayload),
}

/// The web shell rebinds a handful of pane commands; the bound keeps a
/// malformed client from growing the persisted state without limit.
// The desktop host checks a reported set against the same caps
// (desktop/src/main/menu.ts); change them together. 48 holds every editable
// pane command (34 once the neighbouring tab steps joined) with room to
// spare, and the desktop test fails before a new one would not fit.
const BINDINGS_CAP: usize = 48;
const BINDING_TEXT_CAP: usize = 64;

/// Whether a stored shortcut map is within the caps a host's set may use;
/// both maps hold a handful of pane commands, so anything larger is refused.
pub(super) fn bindings_fit(bindings: &std::collections::BTreeMap<String, String>) -> bool {
    bindings.len() <= BINDINGS_CAP
        && bindings.iter().all(|(command, chord)| {
            command.len() <= BINDING_TEXT_CAP && chord.len() <= BINDING_TEXT_CAP
        })
}

pub(super) fn decode(bytes: &[u8]) -> Result<Event, EventValidationError> {
    let event =
        serde_json::from_slice::<EventEnvelope>(bytes).map_err(|_| EventValidationError {
            kind: "event.invalid_json",
            message: "Event JSON could not be decoded".to_owned(),
        })?;

    if event.schema_version != SCHEMA_VERSION {
        return Err(EventValidationError {
            kind: "schema_version.mismatch",
            message: format!(
                "Event schema version {} does not match {}",
                event.schema_version, SCHEMA_VERSION
            ),
        });
    }

    validate_event(event)
}

pub(super) struct EventValidationError {
    pub(super) kind: &'static str,
    pub(super) message: String,
}

pub(super) fn validate_event(event: EventEnvelope) -> Result<Event, EventValidationError> {
    let EventEnvelope { kind, payload, .. } = event;
    let invalid_payload = |kind: &str| EventValidationError {
        kind: "event.invalid_payload",
        message: format!("Event payload for {kind} does not match schema version {SCHEMA_VERSION}"),
    };

    macro_rules! decode {
        ($payload:ty, $variant:ident) => {
            serde_json::from_value::<$payload>(payload)
                .map(Event::$variant)
                .map_err(|_| invalid_payload(&kind))
        };
    }

    match kind.as_str() {
        "terminal_attachment" => decode!(AttachmentPayload, Attachment),
        "terminal_attachment_ready" => decode!(AttachmentCompletionPayload, AttachmentReady),
        "terminal_attachment_action" => decode!(AttachmentActionPayload, AttachmentAction),
        "session_snapshot" => decode!(SessionSnapshotPayload, SessionSnapshot),
        "refresh_status" => Ok(Event::RefreshStatus),
        "click" => decode!(ClickPayload, Click),
        "focus_pane" => decode!(FocusPaneRequestPayload, FocusPane),
        "create_workspace" => decode!(CreateWorkspacePayload, CreateWorkspace),
        "create_tab" => decode!(CreateTabPayload, CreateTab),
        "focus_checkout" => decode!(FocusCheckoutPayload, FocusCheckout),
        "focus_tab" => decode!(FocusTabPayload, FocusTab),
        "rename_tab" => decode!(RenameTabPayload, RenameTab),
        "reorder_tab" => decode!(ReorderTabPayload, ReorderTab),
        "focus_device" => decode!(FocusDevicePayload, FocusDevice),
        "session_fold_toggle" => decode!(SessionFoldPayload, SessionFoldToggle),
        "inactive_checkouts_toggle" => {
            decode!(InactiveCheckoutsTogglePayload, InactiveCheckoutsToggle)
        }
        "inactive_projects_toggle" => {
            decode!(InactiveProjectsTogglePayload, InactiveProjectsToggle)
        }
        "project_checkouts_fold" => {
            decode!(ProjectCheckoutsFoldPayload, ProjectCheckoutsFold)
        }
        "checkout_agents_toggle" => {
            decode!(CheckoutAgentsTogglePayload, CheckoutAgentsToggle)
        }
        "set_primary_checkout" => decode!(SetPrimaryCheckoutPayload, SetPrimaryCheckout),
        "workspace_pin_set" => decode!(WorkspacePinSetPayload, WorkspacePinSet),
        "remove_workspace" => decode!(RemoveWorkspacePayload, RemoveWorkspace),
        "register_device" => decode!(RegisterDevicePayload, RegisterDevice),
        "device_host_consent" => decode!(DeviceHostConsentPayload, DeviceHostConsent),
        "device_host_retry" => decode!(DeviceHostRetryPayload, DeviceHostRetry),
        "remove_device" => decode!(RemoveDevicePayload, RemoveDevice),
        "test_device" => decode!(TestDevicePayload, TestDevice),
        "create_pane" => decode!(CreatePanePayload, CreatePane),
        "resize_pane" => decode!(ResizePanePayload, ResizePane),
        "toggle_zoom" => decode!(ToggleZoomPayload, ToggleZoom),
        "toggle_conversation" => decode!(PaneTargetPayload, ToggleConversation),
        "close_workspace" => decode!(ConfirmedWorkspacePayload, CloseWorkspace),
        "close_tab" => decode!(ConfirmedTabPayload, CloseTab),
        "close_pane" => decode!(ConfirmedPanePayload, ClosePane),
        "close_tree" => decode!(super::tree_close::CloseTreePayload, CloseTree),
        "check_close_status" => decode!(CheckCloseStatusPayload, CheckCloseStatus),
        "retry_agent_close" => decode!(CheckCloseStatusPayload, RetryAgentClose),
        "dismiss_agent_close" => decode!(CheckCloseStatusPayload, DismissAgentClose),
        "reopen_closed" => Ok(Event::ReopenClosed),
        "fork_pane" => decode!(ForkPanePayload, ForkPane),
        "agent_sleep_set" => decode!(AgentSleepSetPayload, AgentSleepSet),
        "agent_sleep" => decode!(PaneTargetPayload, AgentSleep),
        "archive_session" => decode!(
            super::session_archive::ArchiveSessionPayload,
            ArchiveSession
        ),
        "archive_resume" => decode!(super::session_archive::ArchiveTargetPayload, ArchiveResume),
        "archive_delete" => decode!(super::session_archive::ArchiveTargetPayload, ArchiveDelete),
        "archive_check" => decode!(super::session_archive::ArchiveTargetPayload, ArchiveCheck),
        "pane_input_submitted" => decode!(PaneTargetPayload, PaneInputSubmitted),
        "pane_input_sent" => decode!(PaneTargetPayload, PaneInputSent),
        "resolve_session" => decode!(PaneTargetPayload, ResolveSession),
        "revisit_star" => decode!(PaneTargetPayload, RevisitStar),
        "revisit_done" => decode!(PaneTargetPayload, RevisitDone),
        "deliver_letters_now" => decode!(PaneTargetPayload, DeliverLettersNow),
        "pane_visit" => decode!(PaneTargetPayload, PaneVisit),
        "agent_wake" => decode!(AgentWakePayload, AgentWake),
        "wake_sleeping_session" => decode!(SleepingSessionPayload, WakeSleepingSession),
        "check_sleeping_session" => decode!(SleepingSessionPayload, CheckSleepingSession),
        "pane_reopen" => decode!(PaneTargetPayload, PaneReopen),
        "agent_tree_toggle" => decode!(PaneTargetPayload, AgentTreeToggle),
        "session_tree_toggle" => decode!(PaneTargetPayload, SessionTreeToggle),
        "sidebar_lens_set" => decode!(super::detail::SidebarLensPayload, SidebarLensSet),
        "task_view_set" => decode!(super::detail::TaskViewPayload, TaskViewSet),
        "sidebar_project_set" => decode!(super::detail::SidebarProjectPayload, SidebarProjectSet),
        "practice_progress_set" => {
            decode!(super::detail::PracticeProgressPayload, PracticeProgressSet)
        }
        "practice_invite_close" => Ok(Event::PracticeInviteClose),
        "overview_graph_change" => decode!(
            super::overview_graph::OverviewGraphChangePayload,
            OverviewGraphChange
        ),
        "task_reveal" => decode!(super::detail::TaskRevealPayload, TaskReveal),
        "detail_open" => decode!(super::detail::DetailOpenPayload, DetailOpen),
        "detail_back" => Ok(Event::DetailBack),
        "detail_close" => Ok(Event::DetailClose),
        "detail_show" => decode!(super::detail::DetailShowPayload, DetailShow),
        "remote_control" => decode!(RemoteControlPayload, RemoteControl),
        "remote_file_list" => decode!(RemoteFileListPayload, RemoteFileList),
        "file_open" => decode!(FileOpenPayload, FileOpen),
        "reveal_path" => decode!(RevealPathPayload, RevealPath),
        "file_focus" => decode!(FileTabPayload, FileFocus),
        "file_keep_open" => decode!(FileTabPayload, FileKeepOpen),
        "file_close" => decode!(FileClosePayload, FileClose),
        "file_draft" => decode!(FileDraftPayload, FileDraft),
        "file_view" => decode!(FileViewPayload, FileView),
        "file_save" => decode!(FileSavePayload, FileSave),
        "file_conflict" => decode!(FileConflictPayload, FileConflict),
        "file_create" => decode!(ExplorerCreatePayload, FileCreate),
        "dir_create" => decode!(ExplorerCreatePayload, DirCreate),
        "path_rename" => decode!(PathRenamePayload, PathRename),
        "path_move" => decode!(PathMovePayload, PathMove),
        "path_copy" => decode!(PathCopyPayload, PathCopy),
        "path_import" => decode!(PathImportPayload, PathImport),
        "path_trash" => decode!(PathTrashPayload, PathTrash),
        "ui_state_update" => serde_json::from_value::<Box<UiStateUpdatePayload>>(payload)
            .map(Event::UiStateUpdate)
            .map_err(|_| invalid_payload(&kind)),
        // The focused checkout's sessions for the right panel; whatever the
        // payload says, nothing else is read.
        "sessions_refresh" => Ok(Event::SessionsRefresh),
        "sessions_set_filter" => decode!(SessionsFilterPayload, SessionsSetFilter),
        "links_open" => decode!(super::links::LinksOpenPayload, LinksOpen),
        "session_record_open" => decode!(super::links::RecordOpenPayload, SessionRecordOpen),
        "session_record_close" => Ok(Event::SessionRecordClose),
        "session_work_search" => {
            let search: super::links::RecordSearchPayload =
                serde_json::from_value(payload).map_err(|_| invalid_payload(&kind))?;
            if !search.bounded() {
                return Err(invalid_payload(&kind));
            }
            Ok(Event::SessionWorkSearch(search))
        }
        "session_missed_status" => decode!(super::links::MissedStatusPayload, SessionMissedStatus),
        "backlog_add" => decode!(super::backlog::BacklogAddPayload, BacklogAdd),
        "backlog_retry" => decode!(super::backlog::BacklogIntentPayload, BacklogRetry),
        "backlog_dismiss" => decode!(super::backlog::BacklogIntentPayload, BacklogDismiss),
        "backlog_undo" => decode!(super::backlog::BacklogItemPayload, BacklogUndo),
        "backlog_origin" => decode!(crate::links::ledger::BacklogOriginIdentity, BacklogOrigin),
        "backlog_origin_close" => decode!(
            crate::links::ledger::BacklogOriginIdentity,
            BacklogOriginClose
        ),

        "links_close" => Ok(Event::LinksClose),
        "archive_open" => decode!(ArchiveOpenPayload, ArchiveOpen),
        "memory_action" => decode!(MemoryActionPayload, MemoryAction),
        "memory_open" => decode!(MemoryOpenPayload, MemoryOpen),
        "suggestion_action" => decode!(
            super::suggestions::SuggestionActionPayload,
            SuggestionAction
        ),
        "suggestion_evidence" => decode!(
            super::suggestion_evidence::SuggestionEvidencePayload,
            SuggestionEvidence
        ),
        "suggestion_settings" => decode!(
            super::suggestions::SuggestionSettingsPayload,
            SuggestionSettings
        ),
        "hide_ai_switch" => decode!(super::suggestions::HideAiSwitchPayload, HideAiSwitch),
        "suggestions_intro_seen" => Ok(Event::SuggestionsIntroSeen),
        "hide_ai_wake" => decode!(super::suggestions::HideAiWakePayload, HideAiWake),
        "project_memory_navigate" => decode!(ProjectMemoryNavigatePayload, ProjectMemoryNavigate),
        "memory_expand" => decode!(MemoryExpandPayload, MemoryExpand),
        "memory_evictions_seen" => decode!(MemoryEvictionsSeenPayload, MemoryEvictionsSeen),
        "memory_search" => decode!(MemorySearchPayload, MemorySearch),
        "retry_connect" => decode!(RetryConnectPayload, RetryConnect),
        "clone_repository" => decode!(CloneRepositoryPayload, CloneRepository),
        "cancel_repository_clone" => decode!(CancelRepositoryClonePayload, CancelRepositoryClone),
        "kit_reinstall" => decode!(KitReinstallPayload, KitReinstall),
        "kit_agent_set" => decode!(KitAgentSetPayload, KitAgentSet),
        "codex_daemon_disable" => decode!(CodexDaemonDisablePayload, CodexDaemonDisable),
        "kit_check" => decode!(KitCheckPayload, KitCheck),
        "ssh_hosts_list" => decode!(SshHostsListPayload, SshHostsList),
        "herdr_profiles_list" => decode!(HerdrProfilesListPayload, HerdrProfilesList),
        "herdr_profile_connect" => decode!(HerdrProfileConnectPayload, HerdrProfileConnect),
        "ui_attached" => decode!(UiAttachedPayload, UiAttached),
        "ai_settings" => decode!(AiSettingsPayload, AiSettings),
        "terminal_resize" => decode!(TerminalResizePayload, TerminalResize),
        "terminal_scroll" => decode!(TerminalScrollPayload, TerminalScroll),
        "terminal_click" => decode!(TerminalClickPayload, TerminalClick),
        "pane_find" => decode!(PaneFindPayload, PaneFind),
        "pane_find_open" => decode!(PaneFindOpenPayload, PaneFindOpen),
        "pane_text_scale" => decode!(PaneTextScalePayload, PaneTextScale),
        "editor_text_scale" => decode!(EditorTextScalePayload, EditorTextScale),
        "product_analytics_set" => decode!(ProductAnalyticsSetPayload, ProductAnalyticsSet),
        "product_analytics_connected" => Ok(Event::ProductAnalyticsConnected),
        "theme_set" => decode!(ThemeSetPayload, ThemeSet),
        "agent_logo_mode_set" => decode!(AgentLogoModeSetPayload, AgentLogoModeSet),
        "pane_turn_row_mode_set" => decode!(PaneTurnRowModeSetPayload, PaneTurnRowModeSet),
        "color_theme_set" => decode!(ColorThemeSetPayload, ColorThemeSet),
        "usage_pin_set" => decode!(UsagePinSetPayload, UsagePinSet),
        "usage_hidden_set" => decode!(UsageHiddenSetPayload, UsageHiddenSet),
        "interface_language_set" => decode!(InterfaceLanguageSetPayload, InterfaceLanguageSet),
        "agent_onboarding_apply" => decode!(AgentOnboardingApplyPayload, AgentOnboardingApply),
        "first_run_advance" => decode!(FirstRunAdvancePayload, FirstRunAdvance),
        "first_run_back" => decode!(FirstRunBackPayload, FirstRunBack),
        "herdr_takeover" => decode!(HerdrTakeoverPayload, HerdrTakeover),
        "first_run_draft" => decode!(FirstRunDraftPayload, FirstRunDraft),
        "desktop_notifications_set" => {
            decode!(DesktopNotificationsSetPayload, DesktopNotificationsSet)
        }
        "desktop_notifications_done_set" => {
            decode!(
                DesktopNotificationsDoneSetPayload,
                DesktopNotificationsDoneSet
            )
        }
        "screen_intro_seen" => decode!(ScreenIntroSeenPayload, ScreenIntroSeen),
        "changes_select" => decode!(ChangesSelectPayload, ChangesSelect),
        "git_worktree_open" => decode!(GitWorktreeOpenPayload, GitWorktreeOpen),
        "git_worktree_set_base" => decode!(GitWorktreeSetBasePayload, GitWorktreeSetBase),
        "create_worktree" => decode!(CreateWorktreePayload, CreateWorktree),
        "issue_source_set" => decode!(IssueSourceSetPayload, IssueSourceSet),
        "project_settings_set" => decode!(super::project_settings::SetPayload, ProjectSettingsSet),
        "issue_settings_set" => decode!(IssueSettingsSetPayload, IssueSettingsSet),
        "conductor_settings_set" => decode!(ConductorSettingsSetPayload, ConductorSettingsSet),
        "mission_answer" => decode!(MissionAnswerPayload, MissionAnswer),
        "issue_detail_request" => decode!(IssueDetailRequestPayload, IssueDetailRequest),
        "github_search" => decode!(GithubSearchPayload, GithubSearch),
        "worktree_name_suggest" => decode!(WorktreeNameSuggestPayload, WorktreeNameSuggest),
        "issue_set_open" => decode!(IssueSetOpenPayload, IssueSetOpen),
        "local_issue_update" => decode!(LocalIssueUpdatePayload, LocalIssueUpdate),
        "pr_link_issue" => decode!(PrLinkIssuePayload, PrLinkIssue),
        "pr_feedback_read" => decode!(PrFeedbackReadPayload, PrFeedbackRead),
        "pr_delegate" => decode!(PrDelegatePayload, PrDelegate),
        "pr_open" => decode!(PrOpenPayload, PrOpen),
        "pr_card_read" => decode!(PrCardReadPayload, PrCardRead),
        "pr_merge" => decode!(PrMergePayload, PrMerge),
        "pr_update_branch" => decode!(PrUpdateBranchPayload, PrUpdateBranch),
        "set_checkout_issue" => decode!(SetCheckoutPurposePayload, SetCheckoutIssue),
        "set_checkout_purpose" => decode!(SetCheckoutPurposePayload, SetCheckoutPurpose),
        "migrate_main_branch" => decode!(MigrateMainBranchPayload, MigrateMainBranch),
        "task_operation_ack" => decode!(TaskOperationAckPayload, TaskOperationAck),
        "task_agent_retry" => decode!(TaskAgentRetryPayload, TaskAgentRetry),
        "remove_worktree" => decode!(RemoveWorktreePayload, RemoveWorktree),
        "github_request" => decode!(GithubRequestPayload, GithubRequest),
        "fetch_look" => Ok(Event::FetchLook),
        "overview_refresh" => decode!(OverviewRefreshPayload, OverviewRefresh),
        "overview_open_result" => decode!(PaneTargetPayload, OverviewOpenResult),
        "request_view" => decode!(RequestViewPayload, RequestView),
        "window_focus" => decode!(WindowFocusPayload, WindowFocus),
        "cleanup_review" => decode!(CleanupReviewPayload, CleanupReview),
        "cleanup_confirm" => decode!(CleanupConfirmPayload, CleanupConfirm),
        "cleanup_dismiss" => Ok(Event::CleanupDismiss),
        "overview_open_section" => decode!(OverviewOpenSectionPayload, OverviewOpenSection),
        "agent_start_in_checkout" => decode!(AgentStartInCheckoutPayload, AgentStartInCheckout),
        "card_refresh" => Ok(Event::CardRefresh),
        "disk_watch" => decode!(super::disk_sweep::DiskWatchPayload, DiskWatch),
        "reconnect_pane" => decode!(FocusPanePayload, ReconnectPane),
        "pet_set_visible" => decode!(PetVisibilityPayload, PetSetVisible),
        "pet_toggle_visible" => Ok(Event::PetToggleVisible),
        "pet_move" => decode!(PetMovePayload, PetMove),
        "pet_drag" => decode!(PetDragPayload, PetDrag),
        "pet_activity" => Ok(Event::PetActivity),
        "pet_shortcut_update" => decode!(PetShortcutPayload, PetShortcutUpdate),
        "workspace_view" => decode!(WorkspaceViewPayload, WorkspaceView),
        "agent_layout" => decode!(AgentLayoutPayload, AgentLayout),
        "pane_move" => decode!(super::pane_move::Request, PaneMove),
        "pane_move_check" => decode!(super::pane_move::Check, PaneMoveCheck),
        "pane_move_preview" => decode!(super::pane_move_preview::Request, PaneMovePreview),
        "pane_move_preview_cancel" => {
            decode!(super::pane_move_preview::Cancel, PaneMovePreviewCancel)
        }
        "agent_group" => decode!(AgentGroupPayload, AgentGroup),
        "view_layout" => decode!(ViewLayoutPayload, ViewLayout),
        "browser_open" => decode!(BrowserOpenPayload, BrowserOpen),
        "view_comment" => decode!(super::view_feedback::ViewCommentPayload, ViewComment),
        "view_feedback_send" => {
            decode!(
                super::view_feedback::ViewFeedbackSendPayload,
                ViewFeedbackSend
            )
        }
        "browser_state" => decode!(BrowserStatePayload, BrowserState),
        "external_open" => decode!(ExternalOpenPayload, ExternalOpen),
        "external_grants" => decode!(ExternalGrantsPayload, ExternalGrants),
        _ => Err(EventValidationError {
            kind: "event.unknown_kind",
            message: format!("Unknown event kind: {kind}"),
        }),
    }
}

impl Runtime {
    pub(super) fn apply(&mut self, event: Event) -> bool {
        // A creation that carries the shell's key request id must leave that
        // request with an answer: one the core refused before it reached
        // Herdr is recorded as discarded here, so keys typed for it are
        // dropped and the shell stops sending them (PRD instant-pane-topology
        // D-11).
        let creation_request = match &event {
            Event::CreateTab(payload) => payload.request_id.clone(),
            Event::CreatePane(payload) => payload.request_id.clone(),
            Event::AgentStartInCheckout(payload) => payload.request_id.clone(),
            Event::CreateWorktree(payload) => payload.request_id.clone(),
            Event::ForkPane(payload) => payload.request_id.clone(),
            _ => None,
        };
        let mut changed = self.apply_event(event);
        if let Some(request_id) = creation_request {
            changed |= self.settle_creation_request(&request_id);
        }
        changed
    }

    /// A sleeping project's fold is its own: opening it holds it open until
    /// an agent wakes, and the project's ordinary fold stays as the operator
    /// set it (PRD sidebar-titlebar-refresh D-19). `None` when the project is
    /// not asleep, so the caller folds it the ordinary way; otherwise whether
    /// the fold moved. `expanded` absent flips it.
    fn fold_asleep_project(&mut self, workspace_id: &str, expanded: Option<bool>) -> Option<bool> {
        let asleep = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .chain(
                self.snapshot
                    .status
                    .remote
                    .iter()
                    .filter_map(|remote| remote.session.as_ref())
                    .flat_map(|session| &session.workspaces),
            )
            .find(|workspace| workspace.id == workspace_id)
            .and_then(|workspace| workspace.agent_presence.asleep.clone())?;
        let expanded = expanded.unwrap_or(!asleep.expanded);
        if expanded == asleep.expanded {
            return Some(false);
        }
        let key = crate::project_context::asleep_fold_key(workspace_id);
        let keys = &mut self.snapshot.ui_state.session_open_folds;
        keys.retain(|open| open != &key);
        if expanded {
            keys.push(key);
            keys.sort();
        }
        self.refresh_inactive_groups();
        self.persist_ui_state();
        Some(true)
    }

    /// Opens or folds one agent's children in the sidebar tree, or in
    /// Sessions, whose folds are kept apart (PRD D-25).
    fn toggle_agent_tree(&mut self, pane_id: String, sessions: bool) -> bool {
        let exists = self
            .snapshot
            .navigator
            .agents
            .iter()
            .any(|agent| agent.pane_id == pane_id)
            || self
                .snapshot
                .status
                .remote
                .iter()
                .filter_map(|remote| remote.session.as_ref())
                .any(|session| session.agents.iter().any(|agent| agent.pane_id == pane_id));
        if !exists {
            self.set_error(
                "agent_tree.parent_unavailable",
                format!("Agent pane {} is no longer available", pane_id),
                true,
            );
            return true;
        }
        let expanded = if sessions {
            &mut self.snapshot.ui_state.sessions_expanded_agent_pane_ids
        } else {
            &mut self.snapshot.ui_state.expanded_agent_pane_ids
        };
        if expanded.contains(&pane_id) {
            expanded.retain(|pane| pane != &pane_id);
        } else {
            expanded.push(pane_id);
        }
        if !sessions {
            self.refresh_agent_lineage();
        }
        self.persist_ui_state();
        true
    }

    pub(super) fn apply_event(&mut self, event: Event) -> bool {
        match event {
            Event::WorkspaceView(payload) => self.apply_workspace_view(payload),
            Event::AgentLayout(payload) => self.apply_agent_layout(payload),
            Event::PaneMove(payload) => self.apply_pane_move(payload),
            Event::PaneMoveCheck(payload) => self.check_pane_move(payload),
            Event::PaneMovePreview(payload) => self.request_pane_move_preview(payload),
            Event::PaneMovePreviewCancel(payload) => self.cancel_pane_move_preview(payload),
            Event::AgentGroup(payload) => self.apply_agent_group(payload),
            Event::ViewLayout(payload) => self.apply_view_layout(payload),
            Event::BrowserOpen(payload) => self.open_browser(payload),
            Event::ViewComment(payload) => self.apply_view_comment(payload),
            Event::ViewFeedbackSend(payload) => self.apply_view_feedback_send(payload),
            Event::BrowserState(payload) => self.record_browser_state(payload),
            Event::ExternalOpen(payload) => self.open_external(payload),
            Event::ExternalGrants(payload) => self.keep_external_grants(payload),
            Event::SessionsRefresh => self.request_sessions_refresh(),
            Event::LinksOpen(payload) => self.open_links(payload),
            Event::SessionRecordOpen(payload) => self.open_session_record(payload),
            Event::SessionRecordClose => self.close_session_record(),
            Event::SessionWorkSearch(payload) => self.search_work_sessions(payload),
            Event::SessionMissedStatus(payload) => self.set_missed_status(payload),
            Event::BacklogAdd(payload) => self.add_backlog(payload),
            Event::BacklogRetry(payload) => self.retry_backlog(payload),
            Event::BacklogDismiss(payload) => self.dismiss_backlog(payload),
            Event::BacklogUndo(payload) => self.undo_backlog(payload),
            Event::BacklogOrigin(payload) => self.read_backlog_origin(payload),
            Event::BacklogOriginClose(payload) => self.close_backlog_origin(payload),
            Event::LinksClose => self.close_links(),
            Event::SessionsSetFilter(payload) => {
                self.set_sessions_filter(&payload.provider, payload.query)
            }
            Event::ArchiveOpen(payload) => {
                self.open_archive_detail(&payload.kind, &payload.id, payload.preview)
            }
            Event::MemoryAction(payload) => self.apply_memory_action(payload),
            Event::MemoryOpen(payload) => {
                let memory = self.open_project_memory(payload.workspace_id.clone());
                self.open_suggestions(payload.workspace_id) | memory
            }
            Event::SuggestionAction(payload) => self.apply_suggestion_action(payload),
            Event::SuggestionEvidence(payload) => self.open_suggestion_evidence(payload),
            Event::SuggestionSettings(payload) => self.apply_suggestion_settings(payload),
            Event::HideAiSwitch(payload) => self.apply_hide_ai_switch(payload),
            Event::SuggestionsIntroSeen => self.note_suggestions_intro_seen(),
            Event::HideAiWake(payload) => self.note_hide_ai_wake(payload),
            Event::ProjectMemoryNavigate(payload) => self.navigate_project_memory(payload),
            Event::MemoryExpand(payload) => {
                self.expand_memory_card(&payload.workspace_id, payload.number)
            }
            Event::MemoryEvictionsSeen(payload) => {
                self.memory_evictions_seen(&payload.workspace_id, payload.through_unix_ms)
            }
            Event::MemorySearch(payload) => {
                self.search_project_memory(&payload.workspace_id, payload.query)
            }
            Event::Attachment(payload) => self.begin_attachment(payload),
            Event::AttachmentReady(payload) => self.attachment_ready(payload),
            Event::AttachmentAction(payload) => self.attachment_action(payload),
            Event::SessionSnapshot(payload) => self.ingest_session(Ok(payload)),
            Event::AgentSleepSet(payload) => self.set_agent_sleep_after(payload),
            Event::AgentSleep(payload) => self.request_agent_sleep(&payload.pane_id),
            Event::ArchiveSession(payload) => self.request_archive(payload),
            Event::ArchiveResume(payload) => self.request_archive_resume(payload),
            Event::ArchiveDelete(payload) => self.request_archive_delete(payload),
            Event::ArchiveCheck(payload) => self.check_archive_status(payload),
            // The phone's reply reached the pane; nothing on screen moves.
            Event::PaneInputSubmitted(payload) => {
                self.note_delivery_reply(&payload.pane_id);
                self.record_operator_submit(&payload.pane_id);
                false
            }
            // A phone key or reply is about to be written to the pane: a draft
            // may grow there, which the doorbell must not type over.
            Event::ResolveSession(payload) => self.resolve_session(
                &payload.pane_id,
                crate::agent_state::sessions::ResolveSource::Operator,
            ),
            Event::RevisitStar(payload) => self.star_session(&payload.pane_id),
            Event::RevisitDone(payload) => self.finish_revisit(&payload.pane_id),
            Event::PaneInputSent(payload) => {
                self.note_delivery_key(&payload.pane_id, "phone");
                false
            }
            Event::DeliverLettersNow(payload) => self.deliver_letters_now(&payload.pane_id),
            Event::PaneVisit(payload) => self.record_pane_visit(payload.pane_id),
            Event::AgentWake(payload) => self.request_agent_wake(payload),
            Event::WakeSleepingSession(payload) => self.request_dormant_wake(&payload.sleep_id),
            Event::CheckSleepingSession(payload) => self.request_dormant_status(&payload.sleep_id),
            Event::PaneReopen(payload) => self.request_pane_reopen(&payload.pane_id),
            Event::RefreshStatus => self.request_status_refresh(),
            Event::PetSetVisible(payload) => self.set_pet_visible(payload.visible),
            Event::PetToggleVisible => {
                let visible = !self.snapshot.ui_state.pet_visible;
                self.set_pet_visible(visible)
            }
            Event::PetMove(payload) => {
                let origin = PetOriginSnapshot {
                    x: payload.x,
                    y: payload.y,
                };
                if self.snapshot.ui_state.pet_origin == Some(origin) {
                    return false;
                }
                self.snapshot.ui_state.pet_origin = Some(origin);
                self.persist_ui_state();
                self.refresh_pet();
                true
            }
            Event::PetDrag(payload) => {
                if self.pet_dragging == payload.dragging {
                    return false;
                }
                self.pet_dragging = payload.dragging;
                self.note_pet_activity();
                self.refresh_pet()
            }
            Event::PetActivity => {
                self.note_pet_activity();
                self.refresh_pet()
            }
            Event::PetShortcutUpdate(payload) => {
                let accelerator = payload
                    .accelerator
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty());
                let error = payload.error.filter(|value| !value.trim().is_empty());
                let unchanged = self.snapshot.ui_state.pet_shortcut == accelerator
                    && self.snapshot.pet.shortcut_error == error;
                if unchanged {
                    return false;
                }
                self.snapshot.ui_state.pet_shortcut = accelerator;
                self.snapshot.pet.shortcut_error = error;
                self.persist_ui_state();
                self.refresh_pet();
                true
            }
            Event::Click(payload) => {
                let _ = (payload.x, payload.y, payload.button, payload.click_count);
                self.snapshot.focused.surface = payload.surface;
                true
            }
            Event::FocusPane(payload) => {
                if payload.origin == PaneFocusOrigin::Operator {
                    self.acknowledge_operator_focus(payload.client_id.as_deref(), payload.sequence);
                }
                let pane_id = payload.pane_id.clone();
                self.focus_pane(payload.pane_id, payload.origin, payload.request_id);
                if payload.reveal_unseen && self.snapshot.status.last_error.is_none() {
                    self.reveal_unseen_view(&pane_id);
                }
                if payload.focus_device
                    && self.snapshot.status.last_error.is_none()
                    && !self.device_in_front(self.node.as_str())
                {
                    self.bring_device_forward(self.node.as_str().to_owned());
                }
                true
            }
            Event::ReconnectPane(payload) => {
                let pane_id = payload.pane_id;
                let pane_exists = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .flat_map(|checkout| checkout.tabs.iter())
                    .flat_map(|tab| tab.panes.iter())
                    .any(|pane| pane.id == pane_id);
                if !pane_exists {
                    self.set_error(
                        "pane.reconnect_missing",
                        format!("Pane {pane_id} no longer exists"),
                        false,
                    );
                    return true;
                }
                self.reconnect_terminal(&pane_id);
                true
            }
            Event::KitAgentSet(payload) => {
                self.request_kit_agent_set(&payload.device_id, &payload.agent, payload.enabled)
            }
            Event::CodexDaemonDisable(payload) => {
                self.request_codex_daemon_disable(&payload.device_id)
            }
            Event::KitReinstall(payload) => self.request_kit_reinstall(
                &payload.device_id,
                payload.components.as_deref(),
                payload.agents.as_deref(),
            ),
            Event::KitCheck(_) => self.request_kit_check(),
            Event::SshHostsList(_) => self.request_ssh_hosts(),
            Event::HerdrProfilesList(_) => self.request_herdr_profiles(),
            Event::HerdrProfileConnect(payload) => {
                self.connect_herdr_profile(&payload.profile_id, payload.host_consent)
            }
            // Nothing drawn changes; the coordinator reads it on its next wake.
            Event::UiAttached(payload) => {
                self.ui_attached = payload.attached;
                false
            }
            Event::AiSettings(payload) => self.apply_ai_settings(payload),
            Event::RetryConnect(payload) => self.retry_remote_device(&payload.target_id),
            Event::CloneRepository(payload) => {
                self.clone_repository(&payload.url, &payload.parent, &payload.name)
            }
            Event::CancelRepositoryClone(payload) => self.cancel_repository_clone(payload.id),
            Event::CreateWorkspace(payload) => {
                if let Some(device) = payload
                    .device_id
                    .clone()
                    .filter(|device| device != self.node.as_str())
                {
                    if payload.new_folder {
                        self.set_error(
                            "workspace.create_local_only",
                            "A new project can be created on the core's own machine only",
                            false,
                        );
                        return true;
                    }
                    return self.create_device_registration(
                        &device,
                        payload.path,
                        payload.label,
                        payload.confirmed_outside_home,
                    );
                }
                if let Some(context) = self.live.as_ref().cloned() {
                    // A folder whose removal is still closing panes keeps the
                    // registration id it is about to lose; adding it now would
                    // open a pane in that workspace and then watch the retire
                    // delete the registration under it, with nothing to say
                    // the add did not stick.
                    if let Some(workspace_id) = self.workspace_removal_in_flight_for(&payload.path)
                    {
                        self.set_error(
                            "workspace.remove_in_flight",
                            format!(
                                "{} is still being removed; wait for its panes to close, then add it again",
                                payload.path
                            ),
                            false,
                        );
                        self.push_diagnostic(
                            "workspace.create.refused_during_removal",
                            format!("Workspace {workspace_id} removal is closing panes"),
                        );
                        let node = self.node.as_str().to_owned();
                        self.settle_project_open(
                            &node,
                            &payload.path,
                            Err("workspace.remove_in_flight"),
                            super::project_open::Forward::No,
                        );
                        return true;
                    }
                    // A folder that is a project already is answered, not made
                    // again, once a removal closing its panes has had its say: the shell refuses a spelling it can match itself,
                    // but a typed `~/` path reaches the core as the absolute one
                    // hided's boundary resolved, which only the core can match.
                    if !payload.new_folder {
                        let wanted = payload.path.trim_end_matches('/');
                        let registered = self
                            .snapshot
                            .ui_state
                            .workspace_registrations
                            .iter()
                            .find(|row| {
                                row.device_id == self.node.as_str()
                                    && row.path.trim_end_matches('/') == wanted
                            });
                        if let Some(existing) = registered {
                            self.set_error(
                                "workspace.create_registered",
                                format!("{} is already a project", existing.label),
                                false,
                            );
                            let node = self.node.as_str().to_owned();
                            self.settle_project_open(
                                &node,
                                &payload.path,
                                Ok(()),
                                super::project_open::Forward::Checkout,
                            );
                            return true;
                        }
                    }
                    if !self
                        .workspace_creations_in_flight
                        .insert(payload.path.clone())
                    {
                        self.push_diagnostic(
                            "workspace.create.duplicate",
                            format!("Workspace creation is already running for {}", payload.path),
                        );
                        return false;
                    }
                    let path = payload.path.clone();
                    let result = live::spawn_workspace_creation(
                        context,
                        self.node.clone(),
                        payload.path,
                        payload.label,
                        payload.initialize_git,
                        payload.new_folder,
                        self.snapshot.ui_state.workspace_registrations.clone(),
                    );
                    if let Err(message) = result {
                        self.workspace_creations_in_flight.remove(&path);
                        self.set_error("workspace.create_worker_failed", message, true);
                        let node = self.node.as_str().to_owned();
                        self.settle_project_open(
                            &node,
                            &path,
                            Err("workspace.create_worker_failed"),
                            super::project_open::Forward::No,
                        );
                    } else {
                        self.push_diagnostic(
                            "workspace.create.requested",
                            format!("Creating workspace from {path}"),
                        );
                    }
                    return true;
                }
                self.set_error(
                    "workspace.control_unavailable",
                    "Workspace creation requires a live Herdr connection so its initial tab and pane can be created",
                    true,
                );
                let node = self.node.as_str().to_owned();
                self.settle_project_open(
                    &node,
                    &payload.path,
                    Err("workspace.control_unavailable"),
                    super::project_open::Forward::No,
                );
                true
            }
            Event::CreateTab(payload) => {
                let Some((workspace_id, checkout_id, cwd, host)) = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .find(|workspace| workspace.id == payload.workspace_id)
                    .and_then(|workspace| {
                        payload
                            .checkout_id
                            .as_deref()
                            .and_then(|checkout_id| {
                                workspace
                                    .checkouts
                                    .iter()
                                    .find(|checkout| checkout.id == checkout_id)
                            })
                            .or_else(|| workspace.checkouts.first())
                            .map(|checkout| {
                                (
                                    workspace.id.clone(),
                                    checkout.id.clone(),
                                    checkout.path.clone(),
                                    super::projects::tab_host(
                                        workspace,
                                        checkout,
                                        self.node.as_str(),
                                        &self.node,
                                    ),
                                )
                            })
                    })
                else {
                    let workspace_exists = self
                        .snapshot
                        .navigator
                        .workspaces
                        .iter()
                        .any(|workspace| workspace.id == payload.workspace_id);
                    if workspace_exists {
                        self.set_error("tab.no_checkout", "Workspace has no checkout", false);
                        return true;
                    }
                    self.set_error(
                        "tab.unknown_workspace",
                        format!("Workspace {} is not registered", payload.workspace_id),
                        false,
                    );
                    return true;
                };
                let key = (self.node.as_str().to_owned(), cwd.clone());
                let area_id = payload.area_id.or_else(|| {
                    self.agent_layout_of(&key)
                        .map(|layout| layout.tree.active_area.clone())
                });
                if area_id.as_ref().is_some_and(|id| {
                    self.agent_layout_of(&key)
                        .is_none_or(|layout| layout.tree.area(id).is_none())
                }) {
                    self.set_error(
                        "agent_layout.unknown_area",
                        "The Agent area is no longer available",
                        false,
                    );
                    return true;
                }
                if !self.admit_agent_tab(&cwd) {
                    return true;
                }
                let label = payload.label.trim();
                if label.is_empty() {
                    self.set_error("tab.invalid_label", "Tab label cannot be empty", false);
                    return true;
                }
                if self.live.is_none() {
                    self.set_error(
                        "tab.control_unavailable",
                        "Tab creation requires a live Herdr connection",
                        true,
                    );
                    return true;
                }
                let Some(admission_id) = self.reserve_agent_creation(&key) else {
                    return true;
                };
                self.snapshot.navigator.focused_workspace_id = Some(workspace_id.clone());
                self.snapshot.navigator.focused_checkout_id = Some(checkout_id);
                self.snapshot.navigator.root_path = Some(cwd.clone());
                self.sync_changes_root_path();
                self.yield_surface_to_terminal();
                self.persist_current_ui_state();
                self.begin_op_timing(
                    &super::op_timing::tab_create_op_id(admission_id),
                    "tab.create",
                    None,
                    &[],
                );
                if let Some(request_id) = payload.request_id.as_deref() {
                    self.input_requests.open(
                        request_id,
                        super::terminal_input::InputOrigin::TabCreate(admission_id),
                        std::time::Instant::now(),
                    );
                    self.sync_input_requests();
                }
                let admission_path = cwd.clone();
                let group_id = self
                    .workspace_views
                    .as_ref()
                    .and_then(|store| store.views.get(self.node.as_str(), &cwd))
                    .and_then(|view| view.agent_groups.selected.clone())
                    .map(String::into_boxed_str);
                let action = match host {
                    TabHost::Workspace(workspace_id) => RemoteControlAction::CreateTab {
                        workspace_id,
                        cwd,
                        label: label.to_owned(),
                        area_id,
                        group_id,
                        admission_id: Some(admission_id),
                    },
                    TabHost::Open(owner) => RemoteControlAction::OpenOwner {
                        owner,
                        cwd,
                        label: label.to_owned(),
                        area_id,
                        group_id,
                        admission_id: Some(admission_id),
                    },
                };
                self.push_diagnostic(
                    "tab.create.requested",
                    format!("Creating {}", action.kind()),
                );
                if let Err(message) = self.submit_local_control(action) {
                    self.finish_agent_admission(&admission_path, admission_id);
                    self.op_timings
                        .finish(&super::op_timing::tab_create_op_id(admission_id), "failed");
                    self.discard_input_request(
                        &super::terminal_input::InputOrigin::TabCreate(admission_id),
                        "not_sent",
                    );
                    self.set_error("tab.create_worker_failed", message, true);
                }
                true
            }
            Event::FocusCheckout(payload) => {
                if let Some(display_id) = payload.display_id.as_deref()
                    && let Err(error) = self.view_display_known(
                        &payload.workspace_id,
                        &payload.checkout_id,
                        display_id,
                    )
                {
                    self.set_error(error.kind(), error.message(), false);
                    return true;
                }
                if let Some(expanded) = payload.project_expanded
                    && self
                        .fold_asleep_project(&payload.workspace_id, Some(expanded))
                        .is_none()
                {
                    let ids = &mut self.snapshot.ui_state.collapsed_workspace_ids;
                    ids.retain(|id| id != &payload.workspace_id);
                    if !expanded {
                        ids.push(payload.workspace_id.clone());
                        ids.sort();
                    }
                    Self::apply_workspace_expansion(
                        &mut self.snapshot.navigator.workspaces,
                        &self.snapshot.ui_state.collapsed_workspace_ids,
                    );
                }
                if let Some(expanded) = payload.expanded {
                    let ids = &mut self.snapshot.ui_state.session_collapsed_checkout_ids;
                    ids.retain(|id| id != &payload.checkout_id);
                    if !expanded {
                        ids.push(payload.checkout_id.clone());
                        ids.sort();
                    }
                }
                let mut changed = self.focus_checkout(&payload.workspace_id, &payload.checkout_id);
                // The display follows whenever the checkout came forward: a
                // later failure inside the checkout focus (a pane projection
                // worker) must not leave the checkout moved without it.
                if let Some(display_id) = payload.display_id.as_deref()
                    && self.snapshot.navigator.focused_checkout_id.as_deref()
                        == Some(payload.checkout_id.as_str())
                {
                    changed |= self.focus_view_display_of(
                        &payload.workspace_id,
                        &payload.checkout_id,
                        display_id,
                    );
                }
                if payload.focus_device
                    && self.snapshot.status.last_error.is_none()
                    && !self.device_in_front(self.node.as_str())
                {
                    self.bring_device_forward(self.node.as_str().to_owned());
                }
                changed
            }
            Event::FocusTab(payload) => {
                if self.agent_tab_waiting(&payload.tab_id) {
                    self.set_error(
                        "agent_layout.display_limit",
                        "This Agent tab is waiting. Close a tab to make room.",
                        false,
                    );
                    return true;
                }
                let Some(workspace_snapshot) = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter_mut()
                    .find(|workspace| workspace.id == payload.workspace_id)
                else {
                    self.set_error(
                        "tab.unknown_workspace",
                        format!("Workspace {} is not registered", payload.workspace_id),
                        false,
                    );
                    return true;
                };
                let Some(checkout) = workspace_snapshot
                    .checkouts
                    .iter_mut()
                    .find(|checkout| checkout.id == payload.checkout_id)
                else {
                    self.set_error(
                        "tab.unknown_checkout",
                        format!("Checkout {} is not available", payload.checkout_id),
                        false,
                    );
                    return true;
                };
                let Some(index) = checkout
                    .tabs
                    .iter()
                    .position(|tab| tab.id.as_deref() == Some(payload.tab_id.as_str()))
                else {
                    self.set_error(
                        "tab.unknown",
                        format!("Tab {} is not available", payload.tab_id),
                        false,
                    );
                    return true;
                };
                let checkout_path = checkout.path.clone();
                // Hide owns the visible tab, so the active mark moves here,
                // on the frame the operator asked for it, and Herdr is told
                // afterwards. Focusing a tab still does not reorder the
                // strip: Herdr owns the order, and moving the tab here made
                // every switch look like a reorder until the next catalog
                // rebuild undid it.
                let already_visible = checkout.active_tab_id.as_deref() == Some(&payload.tab_id);
                checkout.active_tab_id = Some(payload.tab_id.clone());
                let first_pane_id = checkout.tabs[index]
                    .panes
                    .first()
                    .map(|pane| pane.id.clone());
                // Return to the pane the operator last had in that tab. Its
                // layout is already here, so the tab's own focused pane is
                // known without asking Herdr for it.
                let next_pane_id = self.tab_focus_pane_id(&payload.tab_id, first_pane_id);
                self.snapshot.navigator.focused_workspace_id = Some(payload.workspace_id);
                self.snapshot.navigator.focused_checkout_id = Some(payload.checkout_id.clone());
                self.snapshot.navigator.root_path = Some(checkout_path);
                self.sync_changes_root_path();
                self.refresh_inactive_groups();
                self.visible_tab_ids
                    .insert(payload.checkout_id.clone(), payload.tab_id.clone());
                // Nothing is cleared here. The tab being selected already has
                // its layout in the snapshot, so the canvas draws it on this
                // frame instead of showing an empty canvas until Herdr
                // answers.
                self.select_terminal_pane(next_pane_id.clone());
                self.sync_active_tab_projection();
                // The operator chose this tab, so its pane now holds Hide's
                // keyboard and the read record follows it. Leaving the record
                // on the pane of the tab just left kept marking that pane read
                // while nobody was looking at it.
                self.operator_focused_pane_id = next_pane_id;
                self.refresh_pane_read_state();
                self.yield_surface_to_terminal();
                self.persist_current_ui_state();
                if payload.focus_device && !self.device_in_front(self.node.as_str()) {
                    self.bring_device_forward(self.node.as_str().to_owned());
                }
                // A first visit attaches the tab's panes now. Waiting for the
                // next session update to do it left the canvas empty until
                // Herdr happened to emit something, up to the catalog window.
                if self.live.is_some() {
                    let pane_ids = self
                        .snapshot
                        .pane_layouts
                        .iter()
                        .find(|layout| layout.tab_id == payload.tab_id)
                        .map(|layout| {
                            layout
                                .pane_ids()
                                .into_iter()
                                .map(str::to_owned)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    for pane_id in pane_ids {
                        self.request_terminal_control(&pane_id);
                    }
                }
                // Rule 11: reaching for the tab already showing, with nothing
                // in flight, converges on the state it is already in and
                // sends Herdr no second notification.
                if already_visible
                    && self.view_focus_settled_on(ViewFocusSlot::Tab, &payload.tab_id)
                {
                    return true;
                }
                if self.live.is_none() {
                    self.set_error(
                        "tab.control_unavailable",
                        "Tab focus requires a live Herdr connection",
                        true,
                    );
                    return true;
                }
                // A tab focus still waiting on the lane is replaced there by
                // this one; one already sent is answered anyway, and its
                // answer is told from Herdr's own moves by the order the
                // requests left in (`tab_focus.rs`).
                if let Err(message) = self.submit_local_control(RemoteControlAction::FocusTab {
                    tab_id: payload.tab_id,
                }) {
                    self.set_error("tab.focus_worker_failed", message, true);
                }
                true
            }
            Event::RenameTab(payload) => self.rename_tab(payload),
            Event::ReorderTab(payload) => self.reorder_tab(payload),
            Event::FocusDevice(payload) => {
                self.bring_device_forward(payload.device_id);
                true
            }
            Event::ProjectCheckoutsFold(payload) => {
                if let Some(changed) =
                    self.fold_asleep_project(&payload.workspace_id, payload.expanded)
                {
                    return changed;
                }
                let ids = &mut self.snapshot.ui_state.collapsed_workspace_ids;
                let was_expanded = !ids.contains(&payload.workspace_id);
                let expanded = payload.expanded.unwrap_or(!was_expanded);
                if expanded == was_expanded {
                    return false;
                }
                ids.retain(|id| id != &payload.workspace_id);
                if !expanded {
                    ids.push(payload.workspace_id);
                    ids.sort();
                }
                Self::apply_workspace_expansion(
                    &mut self.snapshot.navigator.workspaces,
                    &self.snapshot.ui_state.collapsed_workspace_ids,
                );
                self.persist_ui_state();
                true
            }
            Event::CheckoutAgentsToggle(payload) => {
                let ids = &mut self.snapshot.ui_state.session_collapsed_checkout_ids;
                if ids.contains(&payload.checkout_id) {
                    ids.retain(|id| id != &payload.checkout_id);
                } else {
                    ids.push(payload.checkout_id);
                    ids.sort();
                }
                self.persist_ui_state();
                true
            }
            Event::SessionFoldToggle(payload) => {
                if !self.session_fold_keys().contains(&payload.key) {
                    self.set_error(
                        "session_fold.unavailable",
                        "This project or device is no longer available".to_owned(),
                        false,
                    );
                    return true;
                }
                let keys = &mut self.snapshot.ui_state.session_open_folds;
                if keys.contains(&payload.key) {
                    keys.retain(|key| key != &payload.key);
                } else {
                    keys.push(payload.key);
                    keys.sort();
                }
                self.refresh_inactive_groups();
                self.persist_ui_state();
                true
            }
            Event::InactiveCheckoutsToggle(payload) => {
                // The fold is this machine's projects' and those of a node
                // that dials the core (D-29), keyed by the project's folder.
                let nodes = self
                    .node_sessions()
                    .flat_map(|(_, session)| &session.workspaces);
                let folds: Vec<bool> = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .chain(nodes)
                    .filter(|workspace| workspace.path == payload.project_path)
                    .map(|workspace| !workspace.inactive_checkouts.checkout_ids.is_empty())
                    .collect();
                if folds.is_empty() {
                    self.set_error(
                        "inactive_checkouts.unknown_project",
                        format!("Project {} is not available", payload.project_path),
                        false,
                    );
                    return true;
                }
                if !folds.contains(&true) {
                    self.set_error(
                        "inactive_checkouts.unavailable",
                        format!("Project {} has no inactive checkouts", payload.project_path),
                        false,
                    );
                    return true;
                }
                let expanded = &mut self
                    .snapshot
                    .ui_state
                    .expanded_inactive_checkout_project_paths;
                if expanded.contains(&payload.project_path) {
                    expanded.retain(|path| path != &payload.project_path);
                } else {
                    expanded.push(payload.project_path);
                    expanded.sort();
                }
                self.refresh_inactive_groups();
                self.persist_ui_state();
                true
            }
            Event::InactiveProjectsToggle(payload) => {
                let available = self
                    .snapshot
                    .navigator
                    .inactive_projects
                    .iter()
                    .any(|group| {
                        group.device_id == payload.device_id && !group.project_ids.is_empty()
                    });
                if !available {
                    self.set_error(
                        "inactive_projects.unavailable",
                        format!("Device {} has no inactive projects", payload.device_id),
                        false,
                    );
                    return true;
                }
                let expanded = &mut self.snapshot.ui_state.expanded_inactive_project_device_ids;
                if expanded.contains(&payload.device_id) {
                    expanded.retain(|device_id| device_id != &payload.device_id);
                } else {
                    expanded.push(payload.device_id);
                    expanded.sort();
                }
                self.refresh_inactive_groups();
                self.persist_ui_state();
                true
            }
            Event::WorkspacePinSet(payload) => self.set_workspace_pinned(payload),
            Event::SetPrimaryCheckout(payload) => self.set_primary_checkout(payload),
            Event::RemoveWorkspace(payload) => self.remove_workspace(payload),
            Event::DeviceHostConsent(payload) => {
                self.set_host_consent(payload.device_id.trim(), payload.allow)
            }
            Event::DeviceHostRetry(payload) => {
                let device_id = payload.device_id.trim().to_owned();
                if self
                    .link_origin(&device_id)
                    .is_some_and(|origin| !origin.core_redials())
                {
                    self.set_error(
                        "device.host.inbound",
                        "This machine connects to the core itself; it reconnects when it can reach the core",
                        false,
                    );
                } else if self.device_registration_exists(&device_id) {
                    self.close_device_host(&device_id, "retry requested");
                    self.retry_device_host_now(&device_id, super::hosts::HostAttempt::Operator);
                } else {
                    self.set_error(
                        "device.host.unknown_device",
                        format!("Device {device_id} is not registered"),
                        false,
                    );
                }
                true
            }
            Event::RegisterDevice(payload) => {
                if let Err(refusal) = self.register_device(payload) {
                    self.set_error(refusal.code, refusal.message, false);
                }
                true
            }
            Event::RemoveDevice(payload) => {
                if let Err(refusal) = self.remove_device(&payload.device_id) {
                    self.set_error(refusal.code, refusal.message, false);
                }
                true
            }
            Event::TestDevice(payload) => {
                let known = self
                    .snapshot
                    .navigator
                    .devices
                    .iter()
                    .any(|device| device.id == payload.device_id && device.kind == "remote");
                if !known {
                    self.set_error(
                        "device.unknown",
                        format!(
                            "Device {} is not a registered SSH device",
                            payload.device_id
                        ),
                        false,
                    );
                    return true;
                }
                self.start_device_test(&payload.device_id)
            }
            Event::CreatePane(payload) => {
                // A split names no pane; it acts on the current terminal pane.
                // With another device selected that pane is not the one the
                // operator is looking at, so the split is refused rather than
                // landing on this machine (PRD S5 B19). A shell that routes
                // remote splits sends `remote_control` instead.
                if let Some(device_id) = self
                    .snapshot
                    .navigator
                    .focused_device_id
                    .as_deref()
                    .filter(|device_id| *device_id != self.node.as_str())
                {
                    self.set_error(
                        "pane.device_mismatch",
                        format!(
                            "Device {device_id} is selected; nothing was split on this machine"
                        ),
                        false,
                    );
                    return true;
                }
                if payload.command.is_some() {
                    self.set_error(
                        "pane.command_unsupported",
                        "Pane split starts the configured shell; a command cannot be supplied",
                        false,
                    );
                    return true;
                }
                let Some(pane_id) = self.snapshot.terminal.pane_id.clone() else {
                    self.set_error(
                        "pane.no_current_pane",
                        "Select a terminal pane before splitting",
                        false,
                    );
                    return true;
                };
                let _ = payload.tab_id;
                if self.live.is_none() {
                    self.set_error(
                        "pane.control_unavailable",
                        "Pane split requires a live Herdr connection",
                        true,
                    );
                    return true;
                }
                let direction = payload.direction;
                let action = PaneControlAction::Split {
                    pane_id: pane_id.clone(),
                    direction,
                    cwd: Some(payload.cwd),
                };
                self.push_diagnostic(
                    format!("pane.split.{}.requested", direction.as_str()),
                    format!("Splitting pane {pane_id} {}", direction.as_str()),
                );
                self.begin_pane_operation(action, payload.request_id.as_deref());
                true
            }
            Event::ResizePane(payload) => {
                if !payload.amount.is_finite() || !(0.001..=0.5).contains(&payload.amount) {
                    self.set_error(
                        "pane.resize_invalid_amount",
                        "Pane resize amount must be between 0.001 and 0.5",
                        false,
                    );
                    return true;
                }
                if self.live.is_none() {
                    self.set_error(
                        "pane.control_unavailable",
                        "Pane resize requires a live Herdr connection",
                        true,
                    );
                    return true;
                }
                let pane_id = payload.pane_id;
                let direction = payload.direction;
                self.push_diagnostic(
                    "pane.resize.requested",
                    format!("Resizing pane {pane_id} {}", direction.as_str()),
                );
                self.begin_pane_operation(
                    PaneControlAction::Resize {
                        pane_id,
                        direction,
                        amount: payload.amount,
                    },
                    None,
                );
                true
            }
            Event::ToggleZoom(payload) => {
                if self.live.is_none() {
                    self.set_error(
                        "pane.control_unavailable",
                        "Pane zoom requires a live Herdr connection",
                        true,
                    );
                    return true;
                }
                let pane_id = payload.pane_id;
                if self.pane_alone_unzoomed(&pane_id) {
                    self.push_diagnostic(
                        "pane.zoom.single_pane",
                        format!("Pane {pane_id} is its tab's only pane; zoom was not sent"),
                    );
                    return true;
                }
                self.push_diagnostic(
                    "pane.zoom.requested",
                    format!("Toggling zoom for pane {pane_id}"),
                );
                self.begin_pane_operation(PaneControlAction::ToggleZoom { pane_id }, None);
                true
            }
            Event::ToggleConversation(payload) => {
                let is_agent = self.snapshot.navigator.agents.iter().any(|agent| {
                    agent.pane_id == payload.pane_id && conversation_agent_kind(&agent.agent_kind)
                });
                if !is_agent {
                    self.set_error(
                        "conversation.unsupported_pane",
                        format!(
                            "Pane {} does not expose an agent conversation",
                            payload.pane_id
                        ),
                        false,
                    );
                    return true;
                }
                let conversation = &mut self.snapshot.ui_state.conversation_pane_ids;
                if !conversation.remove(&payload.pane_id) {
                    conversation.insert(payload.pane_id.clone());
                }
                true
            }
            Event::CloseWorkspace(payload) => {
                let _ = (payload.workspace_id, payload.confirmed);
                false
            }
            Event::CloseTab(payload) => self.close_local_tab(payload.tab_id, payload.confirmed),
            Event::ClosePane(payload) => {
                self.close_local_pane(payload.pane_id, payload.confirmed, true)
            }
            Event::CloseTree(payload) => self.start_tree_close(payload),
            Event::CheckCloseStatus(payload) => self.check_close_status(&payload.key),
            Event::RetryAgentClose(payload) => self.retry_agent_close(&payload.key),
            Event::DismissAgentClose(payload) => self.dismiss_agent_close(&payload.key),
            Event::ReopenClosed => self.reopen_closed(),
            Event::ForkPane(payload) => {
                let pane_id = payload.pane_id;
                if self.pane_move_holds_pane(&pane_id) {
                    self.set_error(
                        "pane.move_pending",
                        "Check the pending pane move before forking this pane",
                        false,
                    );
                    return true;
                }
                let Some(agent) = self
                    .snapshot
                    .navigator
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == pane_id)
                else {
                    self.set_error(
                        "pane.fork_no_agent",
                        format!("Pane {pane_id} is not running an agent that can be forked"),
                        false,
                    );
                    return true;
                };
                let (Some(agent_kind), Some(session_id)) = (
                    ForkableAgent::parse(&agent.agent_kind),
                    agent.session_id.clone(),
                ) else {
                    self.set_error(
                        "pane.fork_unsupported_agent",
                        format!(
                            "Pane {pane_id} runs {} with no forkable session id",
                            agent.agent_kind
                        ),
                        false,
                    );
                    return true;
                };
                // A fork blocks until the agent has started, which takes long
                // enough for a second click to land. Refusing the second one by
                // name is what keeps one activation from becoming two sessions,
                // whichever placement either asked for.
                if self.forks_in_flight.contains_key(&pane_id) {
                    self.set_error(
                        "pane.fork_already_running",
                        format!("Pane {pane_id} is already being forked"),
                        false,
                    );
                    return true;
                }
                // A choice the agent does not take is refused before any pane
                // is made for the fork.
                let launch = self.launch_choice_of(agent);
                if let Err(detail) =
                    super::agent_choice::launch_arguments(agent_kind.kind(), &launch)
                {
                    self.set_error(
                        "pane.fork_invalid_launch_choice",
                        format!(
                            "Pane {pane_id}'s agent was started with a model or effort it does not take: {detail}"
                        ),
                        false,
                    );
                    return true;
                }
                self.fork_sequence += 1;
                let name = fork_name(
                    &pane_id,
                    &format!("{}-{}", self.fork_sequence, unix_milliseconds()),
                );
                // A fork's tab is named as a new tab is, so the strip calls it
                // by its agent rather than by the Herdr name it starts under.
                let (cwd, tab_label) = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .flat_map(|workspace| workspace.checkouts.iter())
                    .find_map(|checkout| {
                        let pane = checkout
                            .tabs
                            .iter()
                            .flat_map(|tab| tab.panes.iter())
                            .find(|pane| pane.id == pane_id)?;
                        Some((pane.cwd.clone(), checkout.next_tab_label.clone()))
                    })
                    .unzip();
                self.forks_in_flight.insert(
                    pane_id.clone(),
                    super::agents::PendingFork {
                        press: self.fork_sequence,
                        generation: self.live_generation,
                        // A view the operator chooses after this press is
                        // where they are now, and the fork does not take it.
                        issued_serial: self.control_lane.issued_serial(),
                        origin: payload.origin,
                        tab_id: None,
                        pane_id: None,
                        tab_label: tab_label.clone(),
                        focused: None,
                        kind: agent.agent_kind.clone(),
                        name: name.clone(),
                        launch: launch.clone(),
                    },
                );
                let Some(context) = self.live.as_ref().cloned() else {
                    self.forks_in_flight.remove(&pane_id);
                    self.set_error(
                        "pane.control_unavailable",
                        "Forking a pane requires a live Herdr connection",
                        true,
                    );
                    return true;
                };
                let request = ForkRequest {
                    connection_generation: self.live_generation,
                    parent_pane_id: pane_id.clone(),
                    agent: agent_kind,
                    session_id,
                    source_reference: agent
                        .row_facts
                        .as_ref()
                        .and_then(|facts| facts.native_reference.clone()),
                    cwd,
                    name,
                    tab_label,
                    placement: payload.placement,
                    codex_daemon: self.codex_daemon_for_pane(&pane_id),
                    launch,
                };
                // Keys typed between the press and the agent's start wait for
                // the fork's agent rather than reaching the shell that starts
                // it (B4); `ingest_fork_result` resolves or discards them.
                let request_id = payload
                    .request_id
                    .unwrap_or_else(|| format!("fork-{}", self.fork_sequence));
                if !self.input_requests.open(
                    &request_id,
                    super::terminal_input::InputOrigin::Fork(pane_id.clone()),
                    std::time::Instant::now(),
                ) {
                    self.forks_in_flight.remove(&pane_id);
                    self.set_error(
                        "pane.fork_input_unavailable",
                        "The fork could not reserve its input holder. Try again after other panes finish starting.",
                        false,
                    );
                    return true;
                }
                self.sync_input_requests();
                self.push_diagnostic("pane.fork.requested", format!("Forking pane {pane_id}"));
                if let Err(message) = live::spawn_agent_fork(context, request) {
                    // A worker that never started is a fork that failed, and
                    // the operator is owed the same notice and the same
                    // stderr diagnostic either way.
                    self.ingest_fork_result(&pane_id, Err(message), 0);
                }
                true
            }
            Event::RemoteControl(payload) => self.request_remote_control(payload),
            Event::RemoteFileList(payload) => self.request_remote_file_list(payload),
            Event::FileOpen(payload) if payload.view.is_some() && self.separate_view_areas() => {
                let view = payload.view.as_ref().expect("matched above");
                self.expect_view_opener(
                    &(view.device_id.clone(), view.path.clone()),
                    &payload.path,
                    crate::view_layout::DisplayKind::File,
                    payload.opener_pane.as_deref(),
                );
                self.open_file_in_front_view(
                    view,
                    &payload.workspace_id,
                    &payload.checkout_id,
                    &payload.path,
                    payload.beside,
                );
                self.persist_current_ui_state();
                true
            }
            Event::FileOpen(payload) => {
                let context_exists = self
                    .catalog_checkout(&payload.workspace_id, &payload.checkout_id)
                    .is_some();
                let context_is_focused = self.front_checkout()
                    == Some((payload.workspace_id.as_str(), payload.checkout_id.as_str()));
                if !context_exists || !context_is_focused {
                    self.set_error(
                        "file.invalid_context",
                        "A file tab requires the selected workspace and checkout",
                        false,
                    );
                    return true;
                }
                if self.separate_view_areas()
                    && let Some(key) =
                        self.workspace_key(&payload.workspace_id, &payload.checkout_id)
                {
                    self.expect_view_opener(
                        &key,
                        &payload.path,
                        crate::view_layout::DisplayKind::File,
                        payload.opener_pane.as_deref(),
                    );
                    self.open_file_in_view(
                        &key,
                        &payload.workspace_id,
                        &payload.checkout_id,
                        &payload.path,
                        payload.preview,
                        payload.beside,
                    );
                } else {
                    self.open_file_tab(
                        &payload.workspace_id,
                        &payload.checkout_id,
                        &payload.path,
                        payload.preview,
                    );
                }
                self.persist_current_ui_state();
                true
            }
            Event::FileKeepOpen(payload) => self.promote_editor_tab(&payload.tab_id),
            Event::RevealPath(payload) => self.reveal_path(payload),
            Event::FileFocus(payload) => {
                let Some(tab) = self
                    .snapshot
                    .editor
                    .tabs
                    .iter()
                    .find(|tab| tab.id == payload.tab_id)
                    .cloned()
                else {
                    self.set_error(
                        "file.focus_failed",
                        format!("File tab {} is not open", payload.tab_id),
                        false,
                    );
                    return true;
                };
                let Some((device_id, checkout)) = self
                    .catalog_checkout(&tab.workspace_id, &tab.checkout_id)
                    .map(|(workspace, checkout)| (workspace.device_id.clone(), checkout.clone()))
                else {
                    self.set_error("editor.invalid_context", "The editor tab's project or checkout is no longer available. Selection was kept.", false);
                    return true;
                };
                // Validate the document/diff before moving any selection. One
                // event restores the surface and its context, including MRU
                // visits across checkouts; no intermediate terminal frame.
                if let Err(message) = self.activate_editor_tab(&payload.tab_id) {
                    self.set_error("editor.focus_failed", message, false);
                    return true;
                }
                // A device's focus is its Herdr session's, which the core
                // follows; showing one of its files moves nothing here.
                if device_id != self.node.as_str() {
                    return true;
                }
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
                true
            }
            Event::FileClose(payload) => {
                if let Some(pending_save) = payload.pending_save {
                    self.start_file_save_then_close(payload.tab_id, pending_save, None)
                } else {
                    self.close_file_tab_now(&payload.tab_id)
                }
            }
            Event::FileView(payload) => {
                let Some(tab) = self.snapshot.editor.tabs.iter_mut().find(|tab| {
                    tab.id == payload.tab_id
                        && matches!(tab.kind, EditorTabKind::File | EditorTabKind::Diff)
                }) else {
                    self.set_error(
                        "file.view_rejected",
                        "The file or diff tab is no longer open",
                        false,
                    );
                    return true;
                };
                if tab.markdown_live == payload.markdown_live && tab.wrap == payload.wrap {
                    return false;
                }
                tab.markdown_live = payload.markdown_live;
                tab.wrap = payload.wrap;
                true
            }
            Event::FileDraft(payload) => {
                let tab_id = match payload.tab_id {
                    Some(tab_id) if self.file_document_open(&tab_id) => tab_id,
                    Some(tab_id) => {
                        self.set_error(
                            "file.draft_rejected",
                            format!("File tab {tab_id} is not open"),
                            false,
                        );
                        return true;
                    }
                    None => match self.snapshot.editor.active_tab_id.clone() {
                        Some(tab_id) => tab_id,
                        None => {
                            self.set_error("file.draft_rejected", "No file tab is active", false);
                            return true;
                        }
                    },
                };
                let Some(document) = self.editor_documents.get_mut(&tab_id).map(Edited::edit)
                else {
                    self.set_error(
                        "file.draft_rejected",
                        "The active file tab has no document state",
                        false,
                    );
                    return true;
                };
                let was_dirty = document.dirty;
                match files::update_draft(document, payload.contents_utf8) {
                    Ok(()) => {
                        let first_edit = !was_dirty && document.dirty;
                        self.sync_file_tab_dirty(&tab_id);
                        self.sync_active_editor_document();
                        // The first edit keeps a preview tab (B5); an echo of
                        // the same contents is not an edit, and a document
                        // that is dirty already was kept by its first edit.
                        if first_edit {
                            self.promote_editor_tab(&tab_id);
                        }
                    }
                    Err(message) => self.set_error("file.draft_rejected", message, false),
                }
                true
            }
            Event::FileSave(payload) => self.request_file_save(payload, CloseAfter::Nothing),
            Event::FileConflict(payload) => {
                let tab_id = match payload.tab_id {
                    Some(tab_id) if self.file_document_open(&tab_id) => tab_id,
                    Some(tab_id) => {
                        self.set_error(
                            "file.conflict_without_tab",
                            format!("File tab {tab_id} is not open"),
                            false,
                        );
                        return true;
                    }
                    None => match self.snapshot.editor.active_tab_id.clone() {
                        Some(tab_id) => tab_id,
                        None => {
                            self.set_error(
                                "file.conflict_without_tab",
                                "No file tab is active",
                                false,
                            );
                            return true;
                        }
                    },
                };
                match payload.action.as_str() {
                    "reload" => self.reload_document(&tab_id),
                    "keep_editing" => self.keep_editing_document(&tab_id),
                    _ => {
                        self.set_error(
                            "file.invalid_conflict_action",
                            "Conflict action must be reload or keep_editing",
                            false,
                        );
                        true
                    }
                }
            }
            Event::FileCreate(payload) => {
                // The created file opens into a display, so a Workspace at its
                // display cap refuses the create before anything is made.
                let created = format!("{}/{}", payload.parent.trim_end_matches('/'), payload.name);
                if self.separate_view_areas()
                    && let Some(key) = self.front_workspace_key()
                    && !self.admit_view_open(
                        &key,
                        &created,
                        crate::view_layout::DisplayKind::File,
                        None,
                        false,
                        false,
                    )
                {
                    return true;
                }
                self.request_explorer_change(ExplorerRequest {
                    root: payload.root,
                    device: payload.device_id,
                    change: ExplorerChange::Create {
                        kind: files::ExplorerOperationKind::FileCreate,
                        parent: payload.parent,
                        name: payload.name,
                    },
                })
            }
            Event::DirCreate(payload) => self.request_explorer_change(ExplorerRequest {
                root: payload.root,
                device: payload.device_id,
                change: ExplorerChange::Create {
                    kind: files::ExplorerOperationKind::DirCreate,
                    parent: payload.parent,
                    name: payload.name,
                },
            }),
            Event::PathRename(payload) => self.request_explorer_change(ExplorerRequest {
                root: payload.root,
                device: payload.device_id,
                change: ExplorerChange::Rename {
                    path: payload.path,
                    name: payload.name,
                },
            }),
            Event::PathMove(payload) => self.request_explorer_change(ExplorerRequest {
                root: payload.root,
                device: payload.device_id,
                change: ExplorerChange::Move {
                    path: payload.path,
                    destination: payload.destination,
                },
            }),
            Event::PathCopy(payload) => self.request_explorer_change(ExplorerRequest {
                root: payload.root,
                device: payload.device_id,
                change: ExplorerChange::Copy {
                    path: payload.path,
                    destination: payload.destination,
                },
            }),
            Event::PathImport(payload) => self.request_explorer_change(ExplorerRequest {
                root: payload.root,
                device: payload.device_id,
                change: ExplorerChange::Import {
                    destination: payload.destination,
                    items: payload
                        .items
                        .into_iter()
                        .map(|item| files::ImportItem {
                            path: item.path,
                            directory: item.directory,
                            staged: item.staged,
                            data: String::new(),
                        })
                        .collect(),
                },
            }),
            Event::PathTrash(payload) => self.request_explorer_change(ExplorerRequest {
                root: payload.root,
                device: payload.device_id,
                change: ExplorerChange::Trash {
                    path: payload.path,
                    select_after: payload.select_after,
                    inode: payload.inode,
                },
            }),
            Event::TerminalClick(payload) => {
                // D8 explicitly chooses Herdr's detected agent as the policy
                // boundary until its frame protocol carries mouse mode.
                // Do not infer it from titles, output, or a session id.
                let agent = self
                    .snapshot
                    .navigator
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == payload.pane_id);
                let report = agent
                    .and_then(|agent| hide_agent_adapter::adapter(&agent.agent_kind))
                    .is_some_and(|row| row.terminal_click);
                crate::diagnostic!(serde_json::json!({
                    "component": "terminal", "kind": "terminal.click_routed",
                    "pane_id": payload.pane_id,
                    "basis": if agent.is_some() { "herdr.agent.list" } else { "not_detected" },
                    "agent_kind": agent.map(|agent| agent.agent_kind.as_str()),
                    "route": if report { "sgr_mouse" } else { "local_selection" },
                    "column": payload.column, "row": payload.row,
                }));
                if report {
                    // SGR uses one-based cells, uppercase M for press and
                    // lowercase m for release. No newline or Enter is sent.
                    let column = u32::from(payload.column) + 1;
                    let row = u32::from(payload.row) + 1;
                    // The shell sends crossterm's bitset, the one Herdr's
                    // terminal.scroll takes. SGR has bits for shift, alt and
                    // control only, so command (bit 8) is dropped here on
                    // purpose: a TUI has no way to receive it.
                    let flags = ((payload.modifiers & 1) * 4)
                        | ((payload.modifiers & 2) * 8)
                        | ((payload.modifiers & 4) * 2);
                    let bytes =
                        format!("\x1b[<{flags};{column};{row}M\x1b[<{flags};{column};{row}m");
                    self.write_terminal_control(&payload.pane_id, bytes.as_bytes());
                    return self.snapshot.status.last_error.is_some();
                }
                false
            }
            Event::TerminalScroll(payload) => {
                if !matches!(payload.direction.as_str(), "up" | "down") || payload.lines == 0 {
                    self.set_error(
                        "terminal.invalid_scroll",
                        "Scroll needs a direction and positive line count",
                        false,
                    );
                    return true;
                }
                // Herdr owns the pane's history, so the wheel is a request it
                // answers with a fresh frame rather than a local buffer move.
                let lines =
                    i32::from(payload.lines) * if payload.direction == "up" { 1 } else { -1 };
                self.scroll_pane(
                    &payload.pane_id,
                    terminal::ScrollRequest {
                        lines,
                        column: payload.column,
                        row: payload.row,
                        modifiers: payload.modifiers,
                    },
                )
            }
            Event::TerminalResize(payload) => {
                if payload.rows == 0 || payload.cols == 0 {
                    self.set_error(
                        "terminal.invalid_resize",
                        "Terminal dimensions must be positive",
                        false,
                    );
                    return true;
                }
                let size = (payload.rows, payload.cols);
                let previous = self.terminal_sizes.insert(payload.pane_id.clone(), size);
                // A view reporting the size the pane is already running at is
                // the common case right after an attach. Sending it on would
                // make Herdr answer with a second full frame for a size that
                // never changed.
                // A pane's first size is what the next launch attaches with,
                // so it is written now. Later sizes ride the next UI-state
                // save rather than putting a file write in the middle of a
                // window drag.
                if previous.is_none() {
                    self.persist_ui_state();
                }
                // The grid follows the drawn rectangle the moment the view
                // reports it, whether or not Herdr has confirmed the layout
                // behind it (PRD grid-ahead-of-herdr D-03): the PTY size is
                // the controller's, so a size sent on a prediction is right
                // whenever the prediction is, and a layout Herdr draws
                // differently makes the view report once more.
                // The node asks Herdr for a new view's whole screen itself
                // when its grid did not change, so an unchanged size goes
                // nowhere.
                if previous != Some(size) {
                    self.send_terminal_size(&payload.pane_id);
                }
                false
            }
            Event::PaneFind(payload) => {
                if payload.term.is_empty() {
                    if self.snapshot.find == PaneFindSnapshot::default() {
                        return false;
                    }
                    self.snapshot.find = PaneFindSnapshot::default();
                    return true;
                }
                // A device's pane is searched through that device's Herdr; one
                // that cannot be reached says why in the bar.
                let route = match self.pane_api_route(&payload.pane_id) {
                    Ok(route) => route,
                    Err(reason) => {
                        self.snapshot.find = PaneFindSnapshot {
                            pane_id: Some(payload.pane_id.clone()),
                            term: payload.term.clone(),
                            unavailable_reason: Some(reason),
                            ..PaneFindSnapshot::default()
                        };
                        return true;
                    }
                };
                // A step continues from the match the operator is on, which is
                // only the stored one when it belongs to this pane and term.
                let current_index = if self.snapshot.find.pane_id.as_deref()
                    == Some(payload.pane_id.as_str())
                    && self.snapshot.find.term == payload.term
                {
                    self.snapshot.find.index
                } else {
                    0
                };
                let request = live::PaneFindRequest {
                    term: payload.term.clone(),
                    options: crate::find::PaneFindOptions {
                        case_sensitive: payload.case_sensitive,
                        whole_word: payload.whole_word,
                        regex: payload.regex,
                    },
                    step: payload.step,
                    current_index,
                };
                if let Err(message) = live::spawn_pane_find(route, request) {
                    self.set_error("pane.find_worker_failed", message, true);
                }
                false
            }
            Event::PaneFindOpen(payload) => {
                self.open_pane_find(payload.pane_id, payload.request_id)
            }
            Event::PaneTextScale(payload) => {
                let current = self
                    .snapshot
                    .ui_state
                    .pane_text_scales
                    .get(&payload.pane_id)
                    .copied()
                    .unwrap_or(DEFAULT_PANE_TEXT_SCALE);
                let Some(next) = self.stepped_text_scale(current, &payload.direction) else {
                    return true;
                };
                // A pane at the default is absent rather than stored at 1.0,
                // so resetting every pane leaves an empty map rather than a
                // row per pane the user ever touched.
                let changed = if next == DEFAULT_PANE_TEXT_SCALE {
                    self.snapshot
                        .ui_state
                        .pane_text_scales
                        .remove(&payload.pane_id)
                        .is_some()
                } else {
                    self.snapshot
                        .ui_state
                        .pane_text_scales
                        .insert(payload.pane_id.clone(), next)
                        != Some(next)
                };
                if changed {
                    self.persist_ui_state();
                }
                changed
            }
            Event::FetchLook => {
                // Changes nothing on screen; the fetch pump reads the result.
                self.look_for_fetch(std::time::Instant::now());
                false
            }
            Event::GithubRequest(payload) => {
                if payload.look && !payload.refresh {
                    // A look changes nothing the operator sees, and a look at
                    // a project that is not the one in front, or not read,
                    // is not an error.
                    if let Some(project) = self.local_git_project_path(&payload.workspace_id)
                        && self
                            .focused_local_checkout()
                            .is_some_and(|(workspace, _)| workspace.path == project)
                        && self.read_when_looked_at(&project, std::time::Instant::now())
                            == super::projects::LookRead::Asked
                    {
                        crate::diagnostic!(serde_json::json!({
                            "component": "github",
                            "kind": "read.looked_at",
                            "project": project,
                        }));
                    }
                    return false;
                }
                let project = self.local_git_project_path(&payload.workspace_id);
                if project.is_none()
                    && let Some((Some(node), workspace)) = self.issue_project(&payload.workspace_id)
                    && workspace.is_git
                {
                    // A node's repositories are all read without being
                    // asked, up to the limit; a refresh reads one again.
                    if payload.refresh {
                        self.reread_node_github(&node, &workspace.path);
                    }
                    return payload.refresh;
                }
                let Some(project) = project else {
                    self.set_error(
                        "github.invalid_project",
                        "GitHub lookup requires a registered local Git project".to_owned(),
                        false,
                    );
                    return true;
                };
                // Every local Git project is read without being asked, so a
                // plain request only matters past the limit, where it puts the
                // project among the first read; a refresh reads it again.
                let named = self.github_wanted.insert(project.clone());
                if payload.refresh {
                    self.refresh_pull_requests(&project);
                }
                let changed = (named || payload.refresh) && self.apply_pull_requests();
                changed || payload.refresh
            }
            Event::OverviewOpenResult(payload) => self.open_result(&payload.pane_id),
            Event::RequestView(payload) => {
                self.observe_request_view(payload.observing, std::time::Instant::now());
                false
            }
            Event::WindowFocus(payload) => self.set_window_focused(payload.focused),
            Event::OverviewRefresh(payload) => {
                let project = self
                    .snapshot
                    .navigator
                    .workspaces
                    .iter()
                    .find(|workspace| {
                        workspace.id == payload.workspace_id
                            && workspace.is_git
                            && workspace.remote_target_id.is_none()
                    })
                    .map(|workspace| workspace.path.clone());
                let Some(project) = project else {
                    // A node's Git project is read again on the node and by
                    // its repository's name here, as this machine's is (D-29).
                    if let Some((Some(node), workspace)) = self.issue_project(&payload.workspace_id)
                        && workspace.is_git
                    {
                        self.reread_node_github(&node, &workspace.path);
                        self.request_device_worktrees(&node, true);
                        return true;
                    }
                    crate::diagnostic!(serde_json::json!({
                        "component": "overview", "kind": "refresh.invalid_project",
                        "workspace_id": payload.workspace_id
                    }));
                    return false;
                };
                self.github_wanted.insert(project.clone());
                self.refresh_pull_requests(&project);
                self.refresh_project_worktrees(&project);
                self.apply_pull_requests();
                true
            }
            Event::CleanupReview(payload) => self.review_cleanup(&payload.workspace_id),
            Event::CleanupConfirm(payload) => self.confirm_cleanup(payload),
            Event::CleanupDismiss => {
                // A worker still running owns the snapshot; it cannot be
                // dismissed until the worker has ended.
                let Some(cleanup) = self.cleanup.as_ref() else {
                    return false;
                };
                if self.cleanup_worker.is_some() {
                    return false;
                }
                let node = self.cleanup_node(&cleanup.workspace_id);
                self.cleanup = None;
                self.publish_cleanup(node.as_deref());
                true
            }
            Event::OverviewOpenSection(payload) => self.overview_open_section(payload),
            Event::AgentStartInCheckout(payload) => self.agent_start_in_checkout(payload),
            Event::CardRefresh => {
                // The card's one refresh button re-reads both the remote
                // answer and the local counts, because the operator pressing
                // it means "this is out of date", not "gh is out of date".
                self.reread_focused_project();
                true
            }
            Event::DiskWatch(payload) => self.watch_disk(payload),
            Event::GitWorktreeOpen(payload) => self.open_git_worktree(payload.checkout_path),
            Event::GitWorktreeSetBase(payload) => {
                self.set_git_worktree_base(payload.repository_root, payload.branch)
            }
            Event::CreateWorktree(payload) => self.create_project_worktree(payload),
            Event::SetCheckoutIssue(payload) => self.set_checkout_issue(payload),
            Event::IssueSourceSet(payload) => self.set_issue_source(payload),
            Event::ProjectSettingsSet(payload) => self.set_project_settings(payload),
            Event::IssueSettingsSet(payload) => self.set_issue_settings(payload),
            Event::ConductorSettingsSet(payload) => {
                let mut settings = self.snapshot.ui_state.conductor_settings.clone();
                if let Some(concurrency) = payload.concurrency {
                    settings.concurrency = concurrency;
                }
                if let Some(scope) = payload.default_scope {
                    settings.default_scope = scope;
                }
                if let Some(candidates) = payload.candidates {
                    settings.candidates = candidates;
                }
                if let Some(strength) = payload.default_strength {
                    settings.default_strength = strength;
                }
                if let Some(other_vendor) = payload.other_vendor {
                    settings.other_vendor = other_vendor;
                }
                self.set_conductor_settings(settings)
            }
            Event::MissionAnswer(payload) => self.mission_answer_event(
                &payload.mission,
                payload.key,
                payload.option,
                payload.text,
                payload.resend,
            ),
            Event::IssueDetailRequest(payload) => self.request_issue_detail(payload),
            Event::GithubSearch(payload) => self.request_github_search(payload),
            Event::WorktreeNameSuggest(payload) => self.suggest_worktree_name(payload),
            Event::IssueSetOpen(payload) => self.set_issue_open(payload),
            Event::LocalIssueUpdate(payload) => self.update_local_issue(payload),
            Event::PrLinkIssue(payload) => self.link_pr_issue(payload),
            Event::PrFeedbackRead(payload) => self.read_pr_feedback(payload),
            Event::PrDelegate(payload) => self.delegate_pr(payload),
            Event::PrOpen(payload) => self.open_pull_request_card(payload),
            Event::PrCardRead(payload) if payload.look_again => self.look_again_pr_card(payload),
            Event::PrCardRead(payload) => self.read_pr_card(payload),
            Event::PrMerge(payload) => self.merge_pull_request(payload),
            Event::PrUpdateBranch(payload) => self.update_pull_request_branch(payload),
            Event::SetCheckoutPurpose(payload) => self.set_checkout_purpose(payload),
            Event::MigrateMainBranch(payload) => self.migrate_main_branch(payload),
            Event::TaskOperationAck(payload) => self.acknowledge_task_operation(payload.id),
            Event::TaskAgentRetry(payload) => self.retry_task_agent(payload.id),
            Event::RemoveWorktree(payload) => self.remove_git_worktree(payload),
            Event::EditorTextScale(payload) => {
                let current = self.snapshot.ui_state.editor_text_scale;
                let Some(next) = self.stepped_text_scale(current, &payload.direction) else {
                    return true;
                };
                if next == current {
                    return false;
                }
                self.snapshot.ui_state.editor_text_scale = next;
                self.persist_ui_state();
                true
            }
            Event::AgentOnboardingApply(payload) => {
                self.apply_agent_onboarding(payload.agents, payload.finish)
            }
            Event::FirstRunAdvance(payload) => {
                self.advance_first_run(payload.from, payload.notifications)
            }
            Event::FirstRunBack(payload) => self.back_first_run(payload.from),
            Event::HerdrTakeover(payload) => self.herdr_takeover_action(payload.action),
            Event::FirstRunDraft(payload) => self.update_first_run_draft(payload),
            Event::DesktopNotificationsSet(payload) => self
                .set_desktop_notification_preferences(Some(payload.enabled), payload.done_enabled),
            Event::DesktopNotificationsDoneSet(payload) => {
                self.set_desktop_notification_preferences(None, Some(payload.enabled))
            }
            Event::ScreenIntroSeen(payload) => self.mark_screen_intro_seen(payload.screen),
            Event::UsagePinSet(payload) => {
                match self.snapshot.ui_state.usage.pin(
                    crate::usage::prefs::Pin {
                        agent: payload.agent,
                        window: payload.window,
                    },
                    payload.enabled,
                    &self.snapshot.usage,
                ) {
                    Ok(false) => false,
                    Ok(true) => {
                        self.persist_ui_state();
                        true
                    }
                    Err(kind) => {
                        self.set_error(kind, "Usage preferences were not saved", false);
                        true
                    }
                }
            }
            Event::UsageHiddenSet(payload) => {
                match self
                    .snapshot
                    .ui_state
                    .usage
                    .hide(payload.agent, payload.hidden)
                {
                    Ok(false) => false,
                    Ok(true) => {
                        self.persist_ui_state();
                        true
                    }
                    Err(kind) => {
                        self.set_error(kind, "Usage preferences were not saved", false);
                        true
                    }
                }
            }
            Event::InterfaceLanguageSet(payload) => {
                if !payload.language.is_null()
                    && serde_json::from_value::<crate::model::InterfaceLanguage>(
                        payload.language.clone(),
                    )
                    .is_err()
                {
                    self.set_error(
                        "interface_language.invalid",
                        "The interface language was not saved",
                        false,
                    );
                    return true;
                }
                let language = (!payload.language.is_null()).then_some(payload.language);
                if self.snapshot.ui_state.interface_language == language {
                    return false;
                }
                self.snapshot.ui_state.interface_language = language;
                self.persist_ui_state();
                true
            }
            Event::ProductAnalyticsConnected => {
                if let Some(sink) = &self.product_analytics {
                    sink.capture(crate::product_analytics::Event::Connected);
                }
                false
            }
            Event::ProductAnalyticsSet(payload) => {
                if self.snapshot.ui_state.product_analytics_enabled == payload.enabled {
                    return false;
                }
                if let Some(sink) = &self.product_analytics {
                    sink.set_enabled(payload.enabled);
                }
                self.snapshot.ui_state.product_analytics_enabled = payload.enabled;
                self.persist_ui_state();
                true
            }
            Event::ThemeSet(payload) => {
                if self.snapshot.ui_state.theme == payload.theme {
                    return false;
                }
                self.snapshot.ui_state.theme = payload.theme;
                self.persist_ui_state();
                true
            }
            Event::AgentLogoModeSet(payload) => {
                if self.snapshot.ui_state.agent_logo_mode == payload.mode {
                    return false;
                }
                self.snapshot.ui_state.agent_logo_mode = payload.mode;
                self.persist_ui_state();
                true
            }
            Event::PaneTurnRowModeSet(payload) => {
                if self.snapshot.ui_state.pane_turn_row_mode == payload.mode {
                    return false;
                }
                self.snapshot.ui_state.pane_turn_row_mode = payload.mode;
                self.persist_ui_state();
                true
            }
            Event::ColorThemeSet(payload) => {
                if self.snapshot.ui_state.color_theme == payload.color_theme {
                    return false;
                }
                self.snapshot.ui_state.color_theme = payload.color_theme;
                self.persist_ui_state();
                true
            }
            Event::ChangesSelect(payload) => {
                let Some(path) = payload.path else {
                    if self.snapshot.changes.selected_path.is_none() {
                        return false;
                    }
                    let changes = self.snapshot.changes.edit();
                    changes.selected_path = None;
                    changes.diff = None;
                    return true;
                };
                if self.snapshot.changes.root_path.as_deref()
                    != self.snapshot.navigator.changes_root_path.as_deref()
                    || self.snapshot.navigator.changes_root_path.is_none()
                {
                    self.set_error(
                        "diff.invalid_context",
                        "History no longer belongs to the selected checkout",
                        false,
                    );
                    return true;
                }
                let entries = if payload.committed {
                    &self.snapshot.changes.committed
                } else {
                    &self.snapshot.changes.entries
                };
                if !entries.iter().any(|entry| entry.path == path) {
                    self.set_error(
                        "diff.selection_unavailable",
                        format!("{path} is no longer in the selected Changes group"),
                        false,
                    );
                    return true;
                }
                // The checkout in front on the selected device, whose History
                // this is, owns the diff tab.
                let Some((workspace_id, checkout_id)) = self.front_checkout_owned() else {
                    self.set_error(
                        "diff.invalid_context",
                        "A diff tab requires the selected workspace and checkout",
                        false,
                    );
                    return true;
                };
                if self.separate_view_areas() {
                    let changed = self.show_diff_in_view(
                        &workspace_id,
                        &checkout_id,
                        &path,
                        payload.committed,
                        payload.preview,
                        payload.beside,
                    );
                    if changed {
                        self.persist_current_ui_state();
                    }
                    return changed;
                }
                let tab_id =
                    Self::diff_tab_id(&workspace_id, &checkout_id, &path, payload.committed);
                if self.snapshot.editor.active_tab_id.as_deref() == Some(tab_id.as_str()) {
                    return if payload.preview {
                        false
                    } else {
                        self.promote_editor_tab(&tab_id)
                    };
                }
                self.show_diff_tab(
                    &workspace_id,
                    &checkout_id,
                    &path,
                    payload.committed,
                    payload.preview,
                );
                self.persist_current_ui_state();
                true
            }
            Event::AgentTreeToggle(payload) => self.toggle_agent_tree(payload.pane_id, false),
            Event::SessionTreeToggle(payload) => self.toggle_agent_tree(payload.pane_id, true),
            Event::SidebarLensSet(payload) => self.set_sidebar_lens(payload),
            Event::TaskViewSet(payload) => self.set_task_view(payload),
            Event::SidebarProjectSet(payload) => self.set_sidebar_project(payload),
            Event::PracticeProgressSet(payload) => self.set_practice_progress(payload),
            Event::PracticeInviteClose => self.close_practice_invite(),
            Event::OverviewGraphChange(payload) => self.change_overview_graph(payload),
            Event::TaskReveal(payload) => self.reveal_task(payload),
            Event::DetailOpen(payload) => self.open_detail(payload.item),
            Event::DetailBack => self.detail_back(),
            Event::DetailClose => self.close_detail(),
            Event::DetailShow(payload) => self.show_detail(payload.shown),
            Event::UiStateUpdate(payload) => {
                let payload = *payload;
                if let Some(visible) = payload.usage_window_visible {
                    self.usage_window_visible = visible;
                }
                if let Some(open) = payload.usage_popover_open {
                    if open && !self.usage_popover_open {
                        self.usage_popover_open_generation =
                            self.usage_popover_open_generation.saturating_add(1);
                    }
                    self.usage_popover_open = open;
                }
                if payload.usage_window_visible.is_some() || payload.usage_popover_open.is_some() {
                    self.feed_usage_ledger();
                }
                // Pet placement, visibility, and shortcut belong to the pet
                // events; a navigator or keyboard save must not erase them.
                let current = self.snapshot.ui_state.clone();
                let previous_ui_state = current.clone();
                let git_was_visible = current.right_panel_visible
                    && matches!(current.right_panel_section, RightPanelSection::Overview);
                let sessions_were_visible = current.right_panel_visible
                    && matches!(current.right_panel_section, RightPanelSection::Sessions);
                self.snapshot.ui_state = UiStateSnapshot {
                    left_sidebar_visible: payload
                        .left_sidebar_visible
                        .unwrap_or(current.left_sidebar_visible),
                    device_rail_visible: payload
                        .device_rail_visible
                        .unwrap_or(current.device_rail_visible),
                    right_panel_visible: payload
                        .right_panel_visible
                        .unwrap_or(current.right_panel_visible),
                    // An unrecognised section name keeps the current one
                    // rather than silently resetting the panel to Explorer.
                    right_panel_section: payload
                        .right_panel_section
                        .as_deref()
                        .and_then(RightPanelSection::parse)
                        .unwrap_or(current.right_panel_section),
                    resolved_sessions: current.resolved_sessions,
                    revisit_sessions: current.revisit_sessions,
                    session_resolution_inputs: current.session_resolution_inputs,
                    session_collapsed_checkout_ids: current.session_collapsed_checkout_ids,
                    session_open_folds: current.session_open_folds,
                    // Recorded by the core when a Rename lands.
                    operator_tab_names: current.operator_tab_names,
                    expanded_paths: payload.expanded_paths,
                    device_expanded_paths: payload
                        .device_expanded_paths
                        .unwrap_or(current.device_expanded_paths),
                    collapsed_workspace_ids: current.collapsed_workspace_ids,
                    collapsed_checkout_ids: payload
                        .collapsed_checkout_ids
                        .unwrap_or(current.collapsed_checkout_ids),
                    expanded_checkout_ids: current.expanded_checkout_ids,
                    expanded_inactive_checkout_project_paths: current
                        .expanded_inactive_checkout_project_paths,
                    expanded_inactive_project_device_ids: current
                        .expanded_inactive_project_device_ids,
                    project_base_branches: current.project_base_branches,
                    project_settings: current.project_settings,
                    project_settings_writes: current.project_settings_writes,
                    expanded_agent_pane_ids: current.expanded_agent_pane_ids,
                    sessions_expanded_agent_pane_ids: current.sessions_expanded_agent_pane_ids,
                    // Each has its own event (`sidebar_lens_set`,
                    // `task_view_set`, `sidebar_project_set`,
                    // `practice_progress_set`, `practice_invite_close`, `detail_*`,
                    // `overview_graph_change`).
                    sidebar_lens: current.sidebar_lens,
                    task_view: current.task_view,
                    sidebar_project: current.sidebar_project,
                    practice: current.practice,
                    practice_invite_closed: current.practice_invite_closed,
                    detail: current.detail,
                    overview_graph: current.overview_graph,
                    selected_path: payload.selected_path,
                    // The keyboard's pane, checkout and device are the
                    // core's: it moves them on the event that asks for the
                    // move and tells Herdr. A whole-state save carries the
                    // shell's last copy of them, which a newer choice can
                    // already have overtaken, so it moves none of them.
                    selected_pane_id: current.selected_pane_id,
                    focus_from_herdr: current.focus_from_herdr,
                    shortcut_bindings: if bindings_fit(&payload.shortcut_bindings) {
                        payload.shortcut_bindings
                    } else {
                        self.set_error(
                            "ui_state.shortcuts_invalid",
                            "Shortcuts were not saved: too many or too long",
                            false,
                        );
                        current.shortcut_bindings.clone()
                    },
                    shortcut_bindings_imported: current.shortcut_bindings_imported,
                    browser_shortcut_bindings: match payload.browser_shortcut_bindings {
                        Some(bindings) if bindings_fit(&bindings) => bindings,
                        Some(_) => {
                            self.set_error(
                                "ui_state.browser_shortcuts_invalid",
                                "Browser shortcuts were not saved: too many or too long",
                                false,
                            );
                            current.browser_shortcut_bindings.clone()
                        }
                        None => current.browser_shortcut_bindings.clone(),
                    },
                    pet_visible: self.snapshot.ui_state.pet_visible,
                    pet_origin: self.snapshot.ui_state.pet_origin,
                    pet_shortcut: self.snapshot.ui_state.pet_shortcut.clone(),
                    focused_device_id: current.focused_device_id,
                    focused_checkout_id: current.focused_checkout_id,
                    workspace_registrations: payload
                        .workspace_registrations
                        .unwrap_or(current.workspace_registrations),
                    device_registrations: payload
                        .device_registrations
                        .unwrap_or(current.device_registrations),
                    // The name a move kept for this machine is the core's own, never a shell's.
                    machine_label: current.machine_label,
                    accent_hex: payload.accent_hex.unwrap_or(current.accent_hex),
                    // `theme_set` owns the theme; a shared UI-state save carries it through.
                    theme: current.theme,
                    // `agent_logo_mode_set` owns the logo mode the same way.
                    agent_logo_mode: current.agent_logo_mode,
                    pane_turn_row_mode: current.pane_turn_row_mode,
                    // `color_theme_set` owns the color theme the same way.
                    color_theme: current.color_theme,
                    interface_language: current.interface_language,
                    usage: current.usage,
                    // `agent_onboarding_*` events own the first-run choice.
                    agent_onboarding: current.agent_onboarding,
                    agent_onboarding_agents: current.agent_onboarding_agents,
                    // The wizard's own events own its progress, the
                    // notification switch and the introductions seen.
                    first_run: current.first_run,
                    first_run_choices: current.first_run_choices,
                    desktop_notifications: current.desktop_notifications,
                    product_analytics_enabled: current.product_analytics_enabled,
                    desktop_notifications_done: current.desktop_notifications_done,
                    screen_intros_seen: current.screen_intros_seen,
                    // A width outside the drag's range keeps the last one; the
                    // shell never sends it, so there is nothing to show.
                    sidebar_width: match payload.sidebar_width {
                        Some(width) if crate::model::sidebar_width_fits(width) => width,
                        Some(width) => {
                            self.push_diagnostic(
                                "ui_state.sidebar_width_out_of_range",
                                format!(
                                    "Sidebar width {width} is outside {}..={}; kept {}",
                                    crate::model::SIDEBAR_WIDTH_MIN,
                                    crate::model::SIDEBAR_WIDTH_MAX,
                                    current.sidebar_width
                                ),
                            );
                            current.sidebar_width
                        }
                        None => current.sidebar_width,
                    },
                    font_size: payload.font_size.unwrap_or(current.font_size),
                    // A size outside the slider's range keeps the last one,
                    // as the sidebar width does; the slider never sends it.
                    terminal_font_size: match payload.terminal_font_size {
                        Some(size) if crate::model::terminal_font_size_fits(size) => size,
                        Some(size) => {
                            self.push_diagnostic(
                                "ui_state.terminal_font_size_out_of_range",
                                format!(
                                    "Terminal font size {size} is outside {}..={}; kept {}",
                                    crate::model::TERMINAL_FONT_SIZE_MIN,
                                    crate::model::TERMINAL_FONT_SIZE_MAX,
                                    current.terminal_font_size
                                ),
                            );
                            current.terminal_font_size
                        }
                        None => current.terminal_font_size,
                    },
                    // The zoom chords own this map; a navigator or keyboard
                    // save must not erase it, for the same reason the pet
                    // fields above are carried through.
                    pane_text_scales: current.pane_text_scales,
                    editor_text_scale: current.editor_text_scale,
                    conversation_pane_ids: current.conversation_pane_ids,
                    pane_read_records: current.pane_read_records,
                    pane_completions: current.pane_completions,
                    pane_launches: current.pane_launches,
                    request_verbs: current.request_verbs,
                    // `agent_sleep_set` owns the setting and the core owns
                    // the sleep records; a shared save carries both through.
                    agent_sleep_after_hours: current.agent_sleep_after_hours,
                    // Settings › Issues owns these through their own events.
                    project_issue_sources: current.project_issue_sources,
                    issue_settings: current.issue_settings,
                    // Settings › Conductor owns it through its own event.
                    conductor_settings: current.conductor_settings,
                    // Every agent start owns this; a shared save carries it.
                    agent_start: current.agent_start,
                    agent_sleep: current.agent_sleep,
                    session_archive: current.session_archive,
                    // The core records it on the front change; a shared save
                    // carries it through, or any save would empty the list.
                    recent_checkouts: current.recent_checkouts,
                    // The shell's visits own this through `pane_visit`; a
                    // shared save carries it through for the same reason.
                    recent_pane_ids: current.recent_pane_ids,
                };
                // Visibility and popover activity wake the provider reader,
                // but they are not durable preferences. The shell sends the
                // current durable values with the shared UI-state event; if
                // those values did not change, do not rewrite state.json or
                // run unrelated catalog reconciliation.
                if self.snapshot.ui_state == previous_ui_state {
                    return true;
                }
                self.refresh_inactive_groups();
                Self::apply_workspace_expansion(
                    &mut self.snapshot.navigator.workspaces,
                    &self.snapshot.ui_state.collapsed_workspace_ids,
                );
                self.refresh_agent_lineage();
                let git_is_visible = self.snapshot.ui_state.right_panel_visible
                    && matches!(
                        self.snapshot.ui_state.right_panel_section,
                        RightPanelSection::Overview
                    );
                if git_is_visible && !git_was_visible {
                    self.reread_focused_project();
                }
                if git_is_visible != git_was_visible {
                    self.refresh_card();
                }
                let sessions_are_visible = self.snapshot.ui_state.right_panel_visible
                    && matches!(
                        self.snapshot.ui_state.right_panel_section,
                        RightPanelSection::Sessions
                    );
                if sessions_are_visible && !sessions_were_visible {
                    self.request_sessions_refresh();
                }
                // Session sync owns session-derived temporary workspaces.
                // UI-state persistence must not rebuild from an empty session
                // and erase the catalog that the user is currently viewing.
                match self.write_ui_state() {
                    Ok(()) => true,
                    Err(message) => {
                        self.set_error("ui_state.save_failed", message, true);
                        true
                    }
                }
            }
        }
    }
}

impl Event {
    /// Terminal input and output, which arrive per keystroke and per chunk
    /// and never move a Workspace's areas.
    pub(super) fn is_terminal_io(&self) -> bool {
        matches!(
            self,
            Event::TerminalResize(_)
                | Event::TerminalScroll(_)
                | Event::TerminalClick(_)
                | Event::Attachment(_)
                | Event::AttachmentReady(_)
                | Event::AttachmentAction(_)
        )
    }
}

impl Runtime {
    /// Sends the size the pane's view last reported to its node, which
    /// resizes its session (an observer attaches again once the size
    /// settles) and records `terminal.resize_settled`. An operation whose
    /// record waits for this pane's first frame after its resize starts
    /// watching now (PRD grid-ahead-of-herdr D-06).
    pub(super) fn send_terminal_size(&mut self, pane_id: &str) {
        let Some(&(rows, cols)) = self.terminal_sizes.get(pane_id) else {
            return;
        };
        self.terminals
            .control(hide_node_link::terminal::TerminalControl::Resize {
                pane: pane_id.to_owned(),
                size: hide_node_link::terminal::GridSize { rows, cols },
            });
        if self.op_timings.resize_sent(pane_id) {
            self.terminals
                .control(hide_node_link::terminal::TerminalControl::WatchFrame {
                    pane: pane_id.to_owned(),
                });
        }
    }
}
