//! The terminal path between a screen, the core and the node that runs a
//! pane (PRD core-host-node-terminal).
//!
//! A node owns its panes' terminal attaches and their bytes: keys from the
//! screen reach the node's writer and a frame from Herdr reaches the screen
//! without passing the core. The core decides which panes attach and keeps
//! the facts it projects; it says so with [`TerminalControl`] and hears what
//! happened as [`TerminalReport`]. Neither carries a key or a frame's bytes,
//! except the few writes the core itself makes (a click's mouse report, a
//! pasted attachment's paths).
//!
//! Pane ids here are the node's own: a device's node knows its panes by its
//! Herdr's ids, and the screen side scopes them to the device.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

/// Panes one node keeps attached at once (D-18). A pane wanted past it is
/// refused as unavailable and tried again when it is next shown.
pub const MAX_ATTACHED_PANES: usize = 64;
/// Key bytes one pane may have on their way to a device before that pane's
/// flow is ended (D-18).
pub const MAX_UNSENT_KEY_BYTES: usize = 256 * 1024;
/// Frame watches ([`TerminalControl::WatchFrame`]) a node keeps: the core
/// waits on at most this many operations at once, so past it the oldest
/// watch belongs to an operation the core has already closed.
pub const MAX_FRAME_WATCHES: usize = 64;
/// The longest pane or creation id a screen may name to a node. A device
/// pane's id, `remote:<device>:pane:<Herdr's id>`, is the longest, with a
/// device id of at most 256 bytes; the node's pane caps count entries, so
/// this bounds their bytes too.
pub const MAX_PANE_ID_BYTES: usize = 512;
/// Output bytes one pane may have on their way to one reader (a screen, or
/// the link up from a device) before they are dropped and the pane is drawn
/// again from a full frame once the reader has caught up (D-16).
pub const MAX_UNSENT_OUTPUT_BYTES: usize = 1024 * 1024;
/// The longest terminal line a device link carries either way. A full frame
/// of a dense, coloured screen on a large display stays well under it once
/// base64 has grown it by a third; a longer line ends the link rather than
/// this process's memory, since a device's node is another machine's
/// program.
pub const MAX_TERMINAL_LINE_BYTES: usize = 8 * 1024 * 1024;
/// Recent output kept for a screen that attaches again: the first of these
/// two bounds to be reached (D-16).
pub const RETAINED_CHUNKS: usize = 512;
pub const RETAINED_BYTES: usize = 2 * 1024 * 1024;
/// Keys held for a pane or a creation that cannot take them yet, per holder
/// (PRD instant-pane-topology D-11).
pub const INPUT_HOLD_LIMIT_BYTES: usize = 64 * 1024;
/// The oldest held key a pane is given, in milliseconds.
pub const HELD_INPUT_MAX_AGE_MS: u64 = 3_000;
/// Creation requests whose keys are held at once.
pub const INPUT_REQUEST_LIMIT: usize = 8;
/// Keys typed while a paste's files are prepared, held behind the paste.
pub const MAX_ATTACHMENT_INPUT_BYTES: usize = 64 * 1024;

/// What the core decides about a node's pane terminal.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum TerminalControl {
    /// Attach the pane, in control when Herdr allows it. A pane with no
    /// size waits for [`TerminalControl::Resize`]. `manual` is an operator's
    /// Reconnect: it starts again from a first attempt.
    Attach {
        pane: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<GridSize>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        manual: bool,
    },
    /// The pane's tab left the attach window: end its session and keep only
    /// that it was released, with the core's words for why.
    Release { pane: String, message: String },
    /// Herdr no longer has the pane: forget everything about it.
    Forget { pane: String },
    /// Hide asked Herdr to close the pane (`closing`), or that close ended.
    /// A session that ends while its pane closes is not a failure, and keys
    /// for it are refused.
    Closing { pane: String, closing: bool },
    /// The PTY size the core settled for the pane: the grid its view fits
    /// in the rectangle drawn for it, confirmed by Herdr or not, since the
    /// controller alone sizes the PTY (PRD grid-ahead-of-herdr D-02).
    Resize { pane: String, size: GridSize },
    /// The panes of the tabs on screen. A failed attach is retried on its
    /// own only while its pane is shown.
    Shown { panes: Vec<String> },
    /// The pane the core's keyboard is in, or none of this node's: a screen
    /// key into any other pane moves the keyboard there, however the
    /// keyboard left it (B1).
    Focus {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pane: Option<String>,
    },
    /// A wheel for a controlled pane.
    Scroll {
        pane: String,
        lines: i32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        column: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        row: Option<u16>,
        #[serde(default)]
        modifiers: u8,
    },
    /// Bytes the core writes itself (a click's mouse report).
    Write { pane: String, data: String },
    /// The agent in the pane sleeps: keys for it are dropped until it wakes.
    Asleep { pane: String, asleep: bool },
    /// A creation the screen sends keys against before Herdr names its pane.
    RequestOpen { request: String, fork: bool },
    /// The pane exists but its creation is still starting. All clients' keys
    /// for it join this request's queue on the pane's node.
    RequestHold { request: String, pane: String },
    /// Herdr's answer for the creation named its pane.
    RequestResolve { request: String, pane: String },
    /// The creation made no pane the keys may follow.
    RequestDiscard { request: String, reason: String },
    /// A paste is being prepared for the pane: keys for it wait behind it.
    AttachmentHold { pane: String, intent: String },
    /// The paste is being cancelled: keys for the pane are refused until it
    /// is released.
    AttachmentRefuse { intent: String },
    /// The paste's text is ready: write it, then the keys held behind it,
    /// to the session of `generation`.
    AttachmentDeliver {
        intent: String,
        generation: u64,
        paste: String,
    },
    /// The paste ended without being written: drop the keys held behind it.
    AttachmentRelease { intent: String },
    /// Report when the pane's next frame reaches the screen. A node keeps
    /// at most [`MAX_FRAME_WATCHES`] of them, apart from its panes.
    WatchFrame { pane: String },
    /// Whether the core has a screen of its own that draws this node's
    /// panes, for a node that draws them on its own screens (PRD
    /// core-host-node-remote-core D-06): while it has none, the node's pane
    /// output stays on the node, and when one arrives every pane it holds
    /// is sent again whole.
    Mirror { on: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct GridSize {
    pub rows: u16,
    pub cols: u16,
}

/// The attach state of one pane, as the screen shows it. Field values are
/// the ones `TerminalPaneSnapshot` has always carried.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct PaneTerminalState {
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub generation: u64,
    pub attempt: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_category: Option<String>,
    pub retry_decision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_attempt_at_unix_ms: Option<u64>,
}

/// What a node tells the core about its panes' terminals.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "report", rename_all = "snake_case")]
pub enum TerminalReport {
    /// The pane's attach state changed.
    State {
        pane: String,
        #[serde(flatten)]
        state: PaneTerminalState,
    },
    /// The first frame of a session reached the screen.
    FirstFrame { pane: String, generation: u64 },
    /// The frame a [`TerminalControl::WatchFrame`] waited for.
    FrameShown { pane: String, at_unix_ms: u64 },
    /// The operator typed into the pane. `at_unix_ms` is when the last key
    /// this report covers was typed; `submitted` is whether an Enter was
    /// among them; `focus` is a key from the screen, which moves the
    /// keyboard to the pane.
    Input {
        pane: String,
        at_unix_ms: u64,
        submitted: bool,
        focus: bool,
    },
    /// Something the screen shows went wrong (`status.last_error`).
    Error {
        pane: String,
        kind: String,
        message: String,
    },
    /// A note for the core's diagnostic list.
    Note {
        pane: String,
        kind: String,
        message: String,
    },
    /// Keys held for a creation were discarded at the node (its cap).
    RequestDiscarded { request: String, reason: String },
    /// A key could not join the paste's held input.
    AttachmentInput {
        intent: String,
        outcome: AttachmentInputOutcome,
        bytes: usize,
    },
    /// The paste was written (`written`), or the session could not take it.
    AttachmentDelivered { intent: String, written: bool },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentInputOutcome {
    /// The held input reached its cap.
    Limit,
    /// The paste is being cancelled.
    Cancelling,
}

/// Where a screen's key goes: a pane, or a creation whose pane Herdr has not
/// named yet.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyTarget {
    Pane(String),
    Request(String),
}

/// What travels down a device link for its terminals, one JSON line
/// [`TerminalLine`] that takes no call slot.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalDown {
    Control {
        control: TerminalControl,
    },
    Key {
        target: KeyTarget,
        data: String,
        typed_at_unix_ms: u64,
    },
    View {
        pane: String,
        size: GridSize,
        new_view: bool,
    },
    Redraw {
        pane: String,
    },
}

/// What travels up a device link from its terminals, one JSON line
/// [`TerminalLine`] beside the link's answers and events.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalUp {
    Output(TerminalOutput),
    /// The pane is gone: a node that drew it from a core's relay forgets
    /// what it kept of it (PRD core-host-node-remote-core D-20).
    Forget {
        pane: String,
    },
    Report {
        report: TerminalReport,
    },
    /// A diagnostic record the device's terminal service wrote; the core's
    /// side files it with the device it came from, since a device's own log
    /// reaches no one.
    Diagnostic {
        record: serde_json::Value,
    },
}

/// Bytes as every terminal line and control carries them: base64.
pub fn encode_base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The bytes a terminal line or control carries as base64.
pub fn decode_base64(value: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|error| format!("base64 payload could not be decoded: {error}"))
}

/// One terminal line on a device link, either way: `{"terminal": …}`.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TerminalLine<T> {
    pub terminal: T,
}

/// The prefix every terminal line starts with, so a link's reader can tell
/// one from an answer without parsing it.
pub const TERMINAL_LINE_PREFIX: &[u8] = b"{\"terminal\":";

/// One pane's output as it leaves a node.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct TerminalOutput {
    pub pane: String,
    /// Base64 of the bytes the screen writes.
    pub data: String,
    /// The bytes draw the whole screen (they start with a reset).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub full: bool,
}

/// A node's terminals, as a screen and the core reach them. Every method
/// returns at once: work that waits runs on the node's own threads.
pub trait TerminalNode: Send + Sync {
    fn control(&self, control: TerminalControl);

    /// A key from the screen, typed at `typed_at_unix_ms`. A key the node
    /// cannot take is reported, never written elsewhere.
    fn key(&self, target: KeyTarget, bytes: Vec<u8>, typed_at_unix_ms: u64);

    /// The grid the screen draws the pane at; `new_view` is a view that has
    /// nothing drawn yet and needs a full frame.
    fn view(&self, pane: &str, size: GridSize, new_view: bool);

    /// A reader lost the pane's output: draw it again from a full frame.
    fn redraw(&self, pane: &str);
}

/// Where a node's reports go: the core, for this machine's node, or the
/// device link, for a device's. A node reports under its own lock, so a
/// sink must take the report at once and never call the node back.
pub trait ReportSink: Send + Sync {
    fn report(&self, report: TerminalReport);
}

/// Every node's terminals behind one [`TerminalNode`], as the core and the
/// screen reach them: this machine's panes by their own ids, a device's by
/// the ids the core scopes to it ([`device_pane_id`]).
pub trait TerminalRoutes: TerminalNode {
    /// The device's node took its link: its panes' terminals go there.
    fn install_device(&self, device: &str, node: Arc<dyn TerminalNode>);
    /// The device's link ended or the device was removed: its panes have no
    /// terminal until a link is installed again.
    fn remove_device(&self, device: &str);
    /// The device's link is up but carries no terminals, for `reason` (its
    /// node speaks an older protocol, or its terminal service did not
    /// start): its panes say so until a link with terminals is installed.
    fn terminals_unstarted(&self, device: &str, reason: &str);
}

/// The id the core gives pane `pane` of `device`.
pub fn device_pane_id(device: &str, pane: &str) -> String {
    format!("{}{pane}", device_pane_prefix(device))
}

/// What every pane id of `device` starts with.
pub fn device_pane_prefix(device: &str) -> String {
    format!("remote:{device}:pane:")
}

/// The device and its own id for a pane id the core scoped to a device
/// ([`device_pane_id`]). The split is at the first `:pane:`, which is the
/// separator because no device id holds `:` ([`device_id_is_unambiguous`]).
pub fn split_device_pane_id(pane: &str) -> Option<(&str, &str)> {
    pane.strip_prefix("remote:")?.split_once(":pane:")
}

/// Whether `device` can name a device. An id holding `:` could hold `:pane:`
/// or end in `:pane`, and then `remote:a:pane:pane:w1` or
/// `remote:a:pane:b:pane:w1` would read as a pane of `a`, so one device
/// could speak for another's pane; the core registers and connects no such
/// device. Without `:` the first `:pane:` always follows the device id.
pub fn device_id_is_unambiguous(device: &str) -> bool {
    !device.contains(':')
}

impl TerminalReport {
    /// The pane the report is about, for the reports that name one.
    pub fn pane_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::State { pane, .. }
            | Self::FirstFrame { pane, .. }
            | Self::FrameShown { pane, .. }
            | Self::Input { pane, .. }
            | Self::Error { pane, .. }
            | Self::Note { pane, .. } => Some(pane),
            Self::RequestDiscarded { .. }
            | Self::AttachmentInput { .. }
            | Self::AttachmentDelivered { .. } => None,
        }
    }

    /// Whether the two reports are about the same subject: a pane's state,
    /// first frame, shown frame or keys, one kind of a pane's error or note,
    /// a creation's discard, or a paste's refused input or result. Of two
    /// waiting about one subject, the later says what its reader still
    /// needs ([`TerminalReport::superseded_by`]); a queue past its cap keeps
    /// one, so it loses no state and grows only by new subjects.
    pub fn same_subject(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::State { pane: a, .. }, Self::State { pane: b, .. })
            | (Self::FirstFrame { pane: a, .. }, Self::FirstFrame { pane: b, .. })
            | (Self::FrameShown { pane: a, .. }, Self::FrameShown { pane: b, .. })
            | (Self::Input { pane: a, .. }, Self::Input { pane: b, .. })
            | (
                Self::RequestDiscarded { request: a, .. },
                Self::RequestDiscarded { request: b, .. },
            )
            | (Self::AttachmentInput { intent: a, .. }, Self::AttachmentInput { intent: b, .. })
            | (
                Self::AttachmentDelivered { intent: a, .. },
                Self::AttachmentDelivered { intent: b, .. },
            ) => a == b,
            (
                Self::Error {
                    pane: a, kind: k, ..
                },
                Self::Error {
                    pane: b, kind: l, ..
                },
            )
            | (
                Self::Note {
                    pane: a, kind: k, ..
                },
                Self::Note {
                    pane: b, kind: l, ..
                },
            ) => a == b && k == l,
            _ => false,
        }
    }

    /// `later`, about this report's subject, in its place, and whether it
    /// waits where this one did rather than where `later` arrived. Keys fold
    /// in what the earlier ones said (a submit, a focus move) and keep the
    /// earlier place unless the later key moves the keyboard, so the
    /// keyboard still ends in the pane typed into last; a refused paste's
    /// bytes add up; every other report's latest word is the state, applied
    /// after what arrived before it.
    pub fn superseded_by(self, later: Self) -> (Self, bool) {
        match (self, later) {
            (
                Self::Input {
                    submitted: submitted_before,
                    focus: focus_before,
                    ..
                },
                Self::Input {
                    pane,
                    at_unix_ms,
                    submitted,
                    focus,
                },
            ) => (
                Self::Input {
                    pane,
                    at_unix_ms,
                    submitted: submitted || submitted_before,
                    focus: focus || focus_before,
                },
                !focus,
            ),
            (
                Self::AttachmentInput {
                    bytes: bytes_before,
                    ..
                },
                Self::AttachmentInput {
                    intent,
                    outcome,
                    bytes,
                },
            ) => (
                Self::AttachmentInput {
                    intent,
                    outcome,
                    bytes: bytes.saturating_add(bytes_before),
                },
                false,
            ),
            (_, later) => (later, false),
        }
    }
}

impl TerminalControl {
    /// The pane the control is about, for the controls that name one.
    pub fn pane_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::Attach { pane, .. }
            | Self::Release { pane, .. }
            | Self::Forget { pane }
            | Self::Closing { pane, .. }
            | Self::Resize { pane, .. }
            | Self::Scroll { pane, .. }
            | Self::Write { pane, .. }
            | Self::Asleep { pane, .. }
            | Self::AttachmentHold { pane, .. }
            | Self::WatchFrame { pane } => Some(pane),
            Self::Shown { .. }
            | Self::Focus { .. }
            | Self::RequestOpen { .. }
            | Self::RequestHold { .. }
            | Self::RequestResolve { .. }
            | Self::RequestDiscard { .. }
            | Self::AttachmentRefuse { .. }
            | Self::AttachmentDeliver { .. }
            | Self::AttachmentRelease { .. }
            | Self::Mirror { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every id a device may hold splits back into itself and its pane,
    /// whatever the pane is named, and the ids that could not are refused.
    #[test]
    fn a_scoped_pane_id_splits_back_into_its_device_and_pane() {
        for device in ["mini", "a", "studio-2", "x_pane"] {
            assert!(device_id_is_unambiguous(device), "{device}");
            for pane in ["w1:p1", "pane:w1", "b:pane:w1"] {
                assert_eq!(
                    split_device_pane_id(&device_pane_id(device, pane)),
                    Some((device, pane))
                );
            }
        }
        for device in ["a:pane", "a:pane:b", "a:", ":x", "a:b"] {
            assert!(!device_id_is_unambiguous(device), "{device}");
        }
    }

    #[test]
    fn a_control_line_names_its_operation_and_omits_absent_fields() {
        let line = serde_json::to_value(TerminalDown::Control {
            control: TerminalControl::Attach {
                pane: "w1:p1".into(),
                size: None,
                manual: false,
            },
        })
        .unwrap();
        assert_eq!(
            line,
            serde_json::json!({"kind": "control", "control": {"op": "attach", "pane": "w1:p1"}})
        );
    }

    #[test]
    fn a_state_report_flattens_the_pane_state() {
        let report = TerminalReport::State {
            pane: "w1:p1".into(),
            state: PaneTerminalState {
                state: "controlling".into(),
                mode: Some("control".into()),
                generation: 3,
                attempt: 1,
                message: None,
                exit_category: None,
                retry_decision: "none".into(),
                last_attempt_at_unix_ms: None,
            },
        };
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["report"], "state");
        assert_eq!(value["state"], "controlling");
        assert_eq!(
            serde_json::from_value::<TerminalReport>(value).unwrap(),
            report
        );
    }
}
