//! What a phone looks at (PRD mobile-companion-ui D-06 to D-10, D-15, D-16):
//! the Views the agent of an open detail opened that are still open, and the
//! content of one of them.
//!
//! The phone never names a path. It names a View by its id, as the list
//! frame gave it, and this file finds the View among the Views the core says
//! that agent has open now (`Core::agent_looks`); a View that is not there,
//! another agent's View and a request that carries any field beyond the id
//! are refused and recorded as `scope.refused`. The bytes come through the
//! readers the desktop's `file_bytes` uses, over this Mac's boundary or a
//! device's helper (`server::send_file_bytes`,
//! `server::stream_device_file_bytes`), so a path is confined to a checkout
//! the same way for both. A diff is a list of changed file names and line
//! counts, never a line of code; a page's screen is another node's work and
//! shows its address here.
//!
//! Content leaves as JSON text, so bytes travel as base64 chunks small
//! enough for one relay message (`CHUNK`), at most `WINDOW` of them ahead of
//! what the phone confirmed (`look_ack`): the relay closes a peer whose
//! queue fills, so a transfer is paced by the phone and never by a timer.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::Message;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use herdr_core::{ChangedFileStatus, DisplayKind, LookChange, LookView, LookViews};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use super::Mobile;
use super::projection::AgentKey;
use crate::server;

/// The most a text document sends: the first 1 MiB, then "the rest is on the
/// Mac" (D-10).
pub const TEXT_CAP: u64 = 1024 * 1024;
/// The largest image a phone is sent (D-10).
pub const IMAGE_CAP: u64 = 10 * 1024 * 1024;
/// Raw bytes in one chunk: 96 KiB is 128 KiB of base64, half of the relay's
/// 256 KiB message.
const CHUNK: usize = 96 * 1024;
/// Chunks sent ahead of the phone's last `look_ack`.
const WINDOW: u64 = 4;
/// Changed files a list names; the totals still count every file.
const CHANGE_FILES: usize = 200;
/// Characters of a name, an address or a folder one frame carries, so the
/// list of 64 Views stays under one relay message.
const NAME_CHARS: usize = 200;
const URL_CHARS: usize = 500;

/// How a phone treats a View, from what the list already knows of it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shown {
    /// A diff: the list of changed files.
    Changes,
    /// A page the desktop host draws.
    Page,
    /// A document drawn from its text.
    Document,
    Image,
    /// Named, never read: a PDF, a video, a file outside every checkout.
    NameOnly,
}

impl Shown {
    pub fn of(view: &LookView) -> Self {
        match view.kind {
            DisplayKind::Diff => Self::Changes,
            DisplayKind::Browser => Self::Page,
            DisplayKind::External => Self::NameOnly,
            // The table of outputs decides: an image is drawn, a PDF, a video
            // and a board are named (reading a movie to find that out would
            // cost the day's relay budget), anything else is read as text.
            DisplayKind::File => match view.output {
                Some("image") => Self::Image,
                Some("pdf" | "video" | "board") => Self::NameOnly,
                _ => Self::Document,
            },
        }
    }

    /// The word the list frame carries.
    fn word(self) -> &'static str {
        match self {
            Self::Changes => "changes",
            Self::Page => "page",
            Self::Document => "doc",
            Self::Image => "image",
            Self::NameOnly => "other",
        }
    }
}

fn cut(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

/// The list frame for the agent `key`: one row per View, with no path.
pub fn list_frame(key: &AgentKey, looks: &LookViews) -> Value {
    let views: Vec<Value> = looks
        .views
        .iter()
        .map(|view| {
            let mut row = json!({
                "id": view.display_id,
                "kind": Shown::of(view).word(),
                "name": cut(&view.label, NAME_CHARS),
                "unseen": view.unseen,
                "at": view.at_unix_ms,
            });
            if let Some(url) = &view.url {
                row["url"] = json!(cut(url, URL_CHARS));
            }
            row
        })
        .collect();
    json!({
        "type": "looks", "device_id": key.device_id, "pane_id": key.pane_id,
        "views": views, "omitted": looks.omitted,
    })
}

/// What a `look_open` may carry, and nothing else.
const OPEN_FIELDS: [&str; 5] = ["type", "request_id", "device_id", "pane_id", "view_id"];

/// A request for one View's content, checked for shape.
#[derive(Debug, Eq, PartialEq)]
pub struct Request<'a> {
    pub request_id: &'a str,
    pub view_id: &'a str,
}

/// Why a request was refused before the core was asked.
#[derive(Debug, Eq, PartialEq)]
pub enum Malformed {
    /// It named something other than a View id: a path, a root, anything the
    /// phone has no business naming (D-16).
    Beyond(String),
    Invalid,
}

/// Reads `look_open` and `look_seen`: a request id (for the first), the
/// agent, and the View's id.
pub fn request(message: &Value) -> Result<Request<'_>, Malformed> {
    let Some(fields) = message.as_object() else {
        return Err(Malformed::Invalid);
    };
    let allowed = |name: &str| OPEN_FIELDS.contains(&name);
    if let Some(name) = fields.keys().find(|name| !allowed(name)) {
        return Err(Malformed::Beyond(name.chars().take(32).collect()));
    }
    let view_id = message
        .get("view_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= 128)
        .ok_or(Malformed::Invalid)?;
    let request_id = message
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 64
                && id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        })
        .unwrap_or("");
    Ok(Request {
        request_id,
        view_id,
    })
}

/// A View the phone may ask about, found among the Views the core lists for
/// the agent now. `None` is a View that is closed, never was, or is another
/// agent's.
pub fn find(looks: &LookViews, view_id: &str) -> Option<LookView> {
    looks
        .views
        .iter()
        .find(|view| view.display_id == view_id)
        .cloned()
}

/// A transfer in progress: the phone's confirmations reach it through
/// `acked`, and it ends with the detail or with the next request.
pub struct Transfer {
    request_id: String,
    acked: watch::Sender<u64>,
    task: tokio::task::JoinHandle<()>,
}

impl Transfer {
    /// The phone has received chunks up to `seq`.
    pub fn ack(&self, request_id: &str, seq: u64) {
        if self.request_id == request_id {
            self.acked.send_if_modified(|acked| {
                let next = (*acked).max(seq.saturating_add(1));
                let changed = next != *acked;
                *acked = next;
                changed
            });
        }
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn result(request_id: &str, view_id: &str, state: &str) -> Value {
    json!({"type": "look_result", "request_id": request_id, "view_id": view_id, "state": state})
}

/// Starts sending `view`'s content. The answer arrives as frames: a
/// `look_result` for everything that is not bytes, else a `look_head`, its
/// `look_chunk`s and a `look_end`.
pub fn open(
    mobile: &Arc<Mobile>,
    request_id: &str,
    view: LookView,
    frames: mpsc::Sender<Value>,
) -> Transfer {
    let (acked, window) = watch::channel(0_u64);
    let mobile = Arc::clone(mobile);
    let id = request_id.to_owned();
    let task = tokio::spawn(async move {
        send(&mobile, &id, &view, &frames, window).await;
    });
    Transfer {
        request_id: request_id.to_owned(),
        acked,
        task,
    }
}

async fn send(
    mobile: &Arc<Mobile>,
    request_id: &str,
    view: &LookView,
    frames: &mpsc::Sender<Value>,
    window: watch::Receiver<u64>,
) {
    let shown = Shown::of(view);
    let answer = match shown {
        Shown::Changes => changes(mobile, request_id, view).await,
        Shown::Page => {
            // The screen of a page is the next node's; the address is all
            // the phone has now (B21).
            let mut frame = result(request_id, &view.display_id, "page");
            frame["url"] = json!(cut(view.url.as_deref().unwrap_or(""), URL_CHARS));
            frame
        }
        Shown::NameOnly => result(request_id, &view.display_id, "other"),
        Shown::Document | Shown::Image => {
            let cap = if shown == Shown::Image {
                IMAGE_CAP
            } else {
                TEXT_CAP
            };
            match read(mobile, request_id, view, cap, shown == Shown::Image).await {
                Ok(read) => return chunks(request_id, view, shown, read, frames, window).await,
                Err(state) => result(request_id, &view.display_id, state),
            }
        }
    };
    let _ = frames.send(answer).await;
}

/// The change list of the View's checkout: names, statuses and line counts.
async fn changes(mobile: &Arc<Mobile>, request_id: &str, view: &LookView) -> Value {
    match changed_files(mobile, request_id, view).await {
        Ok(frame) => frame,
        Err(state) => result(request_id, &view.display_id, state),
    }
}

async fn changed_files(
    mobile: &Arc<Mobile>,
    request_id: &str,
    view: &LookView,
) -> Result<Value, &'static str> {
    let core = Arc::clone(mobile.core());
    let (device, checkout) = (view.device_id.clone(), view.checkout_path.clone());
    let committed = view.committed.unwrap_or(false);
    // The core answers which root and host to read on its owner thread; the
    // Git read runs here, on the blocking pool.
    let read = tokio::task::spawn_blocking(move || {
        core.prepare_checkout_changes(&device, &checkout, committed)
            .and_then(herdr_core::PreparedChanges::read)
    })
    .await;
    let files = match read {
        Ok(Ok(files)) => files,
        Ok(Err(reason)) => {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "look.changes_unavailable",
                "view_id": view.display_id, "reason": reason,
            }));
            return Err(if reason.starts_with("link.") {
                "device_unreachable"
            } else {
                "unavailable"
            });
        }
        Err(error) => {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "look.changes_failed",
                "view_id": view.display_id, "message": error.to_string(),
            }));
            return Err("unavailable");
        }
    };
    let new_files = count_new_files(mobile, request_id, view, &files).await;
    Ok(changes_frame(
        request_id,
        &view.display_id,
        &files,
        &new_files,
    ))
}

/// Untracked files a list reads to count their lines: each is one read through
/// the desktop's file readers, so the rest are named `새 파일` without a count.
const NEW_FILES_COUNTED: usize = 20;

/// What reading an untracked file told: git counts none of them, so a new file
/// is counted the way git counts a changed one, by the lines of its text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NewFile {
    Lines(u32),
    Binary,
}

/// The lines of `bytes`, as `git diff --numstat` counts a new file's: every
/// line end, and a last line that has none.
fn count_lines(bytes: &[u8]) -> u32 {
    let ends = bytes.iter().filter(|byte| **byte == b'\n').count();
    let unterminated = bytes.last().is_some_and(|last| *last != b'\n');
    u32::try_from(ends + usize::from(unterminated)).unwrap_or(u32::MAX)
}

/// Reads the first untracked files of the list (by their index) and says how
/// many lines each has, or that it is not text. A file that cannot be read, or
/// is longer than the text cap, stays uncounted: that is not a binary.
async fn count_new_files(
    mobile: &Arc<Mobile>,
    request_id: &str,
    view: &LookView,
    files: &[LookChange],
) -> HashMap<usize, NewFile> {
    let mut counted = HashMap::new();
    let wanted = files
        .iter()
        .enumerate()
        .take(CHANGE_FILES)
        .filter(|(_, file)| file.status == ChangedFileStatus::Untracked)
        .take(NEW_FILES_COUNTED);
    for (index, file) in wanted {
        let Ok(relative) = hide_platform::path::RelPath::parse(&file.path) else {
            continue;
        };
        let mut new_file = view.clone();
        new_file.path = hide_platform::path::wire_join(&view.checkout_path, &relative);
        let Ok(read) = read(mobile, request_id, &new_file, TEXT_CAP, false).await else {
            continue;
        };
        if read.total > read.bytes.len() as u64 {
            continue;
        }
        counted.insert(
            index,
            if is_text(&read.bytes) {
                NewFile::Lines(count_lines(&read.bytes))
            } else {
                NewFile::Binary
            },
        );
    }
    counted
}

/// The `look_changes` frame: every file's status and counts, never a line of
/// code. A file is `binary` when git could not count it (its counts are
/// absent and it is not new) or when reading it showed it is not text.
fn changes_frame(
    request_id: &str,
    view_id: &str,
    files: &[LookChange],
    new_files: &HashMap<usize, NewFile>,
) -> Value {
    let counts = |index: usize, file: &LookChange| match new_files.get(&index) {
        Some(NewFile::Lines(lines)) => (Some(*lines), Some(0), false),
        Some(NewFile::Binary) => (None, None, true),
        None => (
            file.added,
            file.removed,
            file.added.is_none()
                && file.removed.is_none()
                && file.status != ChangedFileStatus::Untracked,
        ),
    };
    let (mut added, mut removed) = (0_u64, 0_u64);
    let mut rows = Vec::new();
    for (index, file) in files.iter().enumerate() {
        let (file_added, file_removed, binary) = counts(index, file);
        added += u64::from(file_added.unwrap_or(0));
        removed += u64::from(file_removed.unwrap_or(0));
        if index < CHANGE_FILES {
            rows.push(json!({
                "path": cut(&file.path, URL_CHARS),
                "status": file.status,
                "added": file_added,
                "removed": file_removed,
                "binary": binary,
            }));
        }
    }
    json!({
        "type": "look_changes", "request_id": request_id, "view_id": view_id,
        "files": rows, "count": files.len(), "added": added, "removed": removed,
    })
}

/// What the file readers answered: the bytes up to the cap, the file's size,
/// and whether the cap cut it.
struct Read {
    bytes: Vec<u8>,
    total: u64,
}

/// Reads the View's file up to `cap` bytes through the desktop's readers.
/// The error is the `look_result` state to answer.
async fn read(
    mobile: &Arc<Mobile>,
    request_id: &str,
    view: &LookView,
    cap: u64,
    refuse_past_cap: bool,
) -> Result<Read, &'static str> {
    if view.device_id != mobile.node().as_str() {
        // A device that is not linked has nothing to read, which the phone
        // words as that and not as a read that failed (B26).
        let (core, device) = (Arc::clone(mobile.core()), view.device_id.clone());
        let linked = tokio::task::spawn_blocking(move || core.node_link(&device).map(|_| ())).await;
        if let Some(state) = link_state(linked.unwrap_or(Ok(())), &view.display_id) {
            return Err(state);
        }
    }
    let event = json!({
        "schema_version": 2, "kind": "file_bytes",
        "payload": {
            "request_id": request_id, "device_id": view.device_id,
            "root": view.checkout_path, "path": view.path,
            "offset": 0, "length": cap,
        },
    });
    let (sink, mut frames) = mpsc::channel::<Message>(2);
    let (boundary, roots) = (mobile.boundary(), mobile.roots());
    let reading = if view.device_id == mobile.node().as_str() {
        let mut sink = sink;
        tokio::spawn(async move {
            let _ = server::send_file_bytes(&mut sink, &boundary, &roots, &event).await;
        })
    } else {
        let (core, device) = (Arc::clone(mobile.core()), view.device_id.clone());
        tokio::spawn(async move {
            let _ =
                server::stream_device_file_bytes(sink, core, boundary, roots, device, event).await;
        })
    };
    let mut bytes = Vec::new();
    let mut total = 0_u64;
    let outcome = loop {
        let Some(message) = frames.recv().await else {
            // The reader ended without saying it was done: a lost read.
            break Err("failed");
        };
        match message {
            Message::Binary(frame) => {
                let Some((header, data)) = server::split_bytes_frame(&frame) else {
                    break Err("failed");
                };
                total = header.get("total").and_then(Value::as_u64).unwrap_or(0);
                if refuse_past_cap && total > cap {
                    // An image past its cap is named, not sent (B18).
                    break Err("too_large");
                }
                bytes.extend_from_slice(data);
                if header.get("eof").and_then(Value::as_bool).unwrap_or(false) {
                    break Ok(());
                }
            }
            Message::Text(text) => {
                let reason = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|frame| {
                        frame
                            .pointer("/payload/reason")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .unwrap_or_default();
                herdr_core::diagnostic!(json!({
                    "component": "mobile_phone", "kind": "look.read_refused",
                    "view_id": view.display_id, "reason": reason,
                }));
                break Err(match reason.as_str() {
                    "too_large" => "too_large",
                    _ => "failed",
                });
            }
            _ => break Err("failed"),
        }
    };
    reading.abort();
    outcome?;
    Ok(Read { bytes, total })
}

/// The `look_result` state for a device whose link could not be had: it is
/// `device_unreachable`, and why goes to the log (B26). A link that is up says
/// nothing.
fn link_state(link: Result<(), String>, view_id: &str) -> Option<&'static str> {
    let reason = link.err()?;
    herdr_core::diagnostic!(json!({
        "component": "mobile_phone", "kind": "look.device_unreachable",
        "view_id": view_id, "reason": reason,
    }));
    Some("device_unreachable")
}

/// Whether the bytes of a text document are text: UTF-8, allowing the cut at
/// the end that the cap makes.
fn is_text(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(error) => error.error_len().is_none() && error.valid_up_to() + 4 > bytes.len(),
    }
}

/// Sends the head, the chunks and the end of one read, paced by the phone.
async fn chunks(
    request_id: &str,
    view: &LookView,
    shown: Shown,
    read: Read,
    frames: &mpsc::Sender<Value>,
    mut window: watch::Receiver<u64>,
) {
    let id = &view.display_id;
    if shown == Shown::Document && !is_text(&read.bytes) {
        let _ = frames.send(result(request_id, id, "other")).await;
        return;
    }
    let mode = match shown {
        Shown::Image => "image",
        // The desktop's own rule for what a Markdown file is, so the two draw alike.
        _ if hide_host::document::language_for(std::path::Path::new(&view.path)).as_deref()
            == Some("markdown") =>
        {
            "markdown"
        }
        _ => "text",
    };
    let size = read.bytes.len();
    let head = json!({
        "type": "look_head", "request_id": request_id, "view_id": id, "mode": mode,
        "size": size, "total": read.total.max(size as u64),
        "truncated": read.total > size as u64,
        "chunks": size.div_ceil(CHUNK),
    });
    if frames.send(head).await.is_err() {
        return;
    }
    for (seq, bytes) in read.bytes.chunks(CHUNK).enumerate() {
        let seq = seq as u64;
        // The phone has to have confirmed all but the last `WINDOW` chunks
        // before the next leaves.
        if window
            .wait_for(|acked| seq < *acked + WINDOW)
            .await
            .is_err()
        {
            return;
        }
        let frame = json!({
            "type": "look_chunk", "request_id": request_id, "seq": seq,
            "data": STANDARD.encode(bytes),
        });
        if frames.send(frame).await.is_err() {
            return;
        }
    }
    let _ = frames
        .send(json!({"type": "look_end", "request_id": request_id, "view_id": id}))
        .await;
}

/// The event the Desk sends when its lane shows an output: the View is seen.
/// It is the phone's too (D-15).
pub fn seen_event(view: &LookView) -> Vec<u8> {
    json!({
        "schema_version": 2, "kind": "desk_output_seen",
        "payload": {
            "device_id": view.device_id, "checkout": view.checkout_path, "id": view.display_id,
        },
    })
    .to_string()
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(label: &str, kind: DisplayKind) -> LookView {
        LookView {
            device_id: "test-node".into(),
            checkout_path: "/repo".into(),
            display_id: "d-1".into(),
            kind,
            label: label.into(),
            path: format!("/repo/{label}"),
            url: None,
            committed: None,
            output: None,
            unseen: true,
            at_unix_ms: 1,
        }
    }

    #[test]
    fn a_view_is_shown_by_its_kind_and_the_output_table() {
        let shown = |label: &str, kind, output| {
            let mut view = view(label, kind);
            view.output = output;
            Shown::of(&view)
        };
        assert_eq!(
            shown("a.png", DisplayKind::File, Some("image")),
            Shown::Image
        );
        assert_eq!(
            shown("notes.md", DisplayKind::File, Some("document")),
            Shown::Document
        );
        assert_eq!(shown("main.rs", DisplayKind::File, None), Shown::Document);
        assert_eq!(
            shown("deck.pdf", DisplayKind::File, Some("pdf")),
            Shown::NameOnly
        );
        assert_eq!(
            shown("clip.mov", DisplayKind::File, Some("video")),
            Shown::NameOnly
        );
        assert_eq!(shown("main.rs", DisplayKind::Diff, None), Shown::Changes);
        assert_eq!(shown("localhost", DisplayKind::Browser, None), Shown::Page);
        assert_eq!(
            shown("report.pdf", DisplayKind::External, None),
            Shown::NameOnly
        );
        assert_eq!(shown(".env", DisplayKind::File, None), Shown::Document);
    }

    #[test]
    fn the_list_names_a_view_and_never_its_path() {
        let key = AgentKey {
            device_id: "test-node".into(),
            pane_id: "w1:p1".into(),
        };
        let shown = view("notes.md", DisplayKind::File);
        let frame = list_frame(
            &key,
            &LookViews {
                views: vec![shown],
                omitted: 2,
            },
        );
        let sent = frame.to_string();
        assert!(!sent.contains("/repo"), "{sent}");
        assert_eq!(frame["views"][0]["id"], "d-1");
        assert_eq!(frame["views"][0]["kind"], "doc");
        assert_eq!(frame["omitted"], 2);
    }

    #[test]
    fn sixty_four_long_views_still_fit_one_relay_message() {
        let key = AgentKey {
            device_id: "d".repeat(64),
            pane_id: "p".repeat(64),
        };
        let mut long = view(&"n".repeat(4000), DisplayKind::Browser);
        long.url = Some("u".repeat(4000));
        let frame = list_frame(
            &key,
            &LookViews {
                views: vec![long; 64],
                omitted: 0,
            },
        );
        assert!(
            frame.to_string().len() < 200 * 1024,
            "{}",
            frame.to_string().len()
        );
    }

    #[test]
    fn a_view_is_found_only_among_the_agents_open_ones() {
        let looks = LookViews {
            views: vec![view("a.md", DisplayKind::File)],
            omitted: 0,
        };
        assert!(find(&looks, "d-1").is_some());
        assert!(find(&looks, "d-2").is_none());
        assert!(find(&LookViews::default(), "d-1").is_none());
    }

    #[test]
    fn the_phone_marks_a_view_seen_with_the_event_the_desk_sends() {
        let sent: Value =
            serde_json::from_slice(&seen_event(&view("a.md", DisplayKind::File))).unwrap();
        assert_eq!(sent["kind"], "desk_output_seen");
        assert_eq!(sent["payload"]["id"], "d-1");
        assert_eq!(sent["payload"]["checkout"], "/repo");
        assert_eq!(sent["payload"]["device_id"], "test-node");
    }

    async fn sent_next(frames: &mut mpsc::Receiver<Value>) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(5), frames.recv())
            .await
            .expect("a frame within five seconds")
            .expect("the transfer is still sending")
    }

    #[tokio::test]
    async fn chunks_go_out_a_window_ahead_of_what_the_phone_confirmed() {
        let looked = view("notes.md", DisplayKind::File);
        let bytes = vec![b'a'; CHUNK * 6 + 10];
        let total = bytes.len() as u64 + 5;
        let (frames, mut sent) = mpsc::channel(64);
        let (acked, window) = watch::channel(0_u64);
        let task = tokio::spawn(async move {
            let read = Read { bytes, total };
            chunks("r-1", &looked, Shown::Document, read, &frames, window).await;
        });
        let head = sent_next(&mut sent).await;
        assert_eq!(head["type"], "look_head");
        assert_eq!(head["mode"], "markdown");
        assert_eq!(head["chunks"], 7);
        assert_eq!(head["truncated"], true);
        for seq in 0..WINDOW {
            let chunk = sent_next(&mut sent).await;
            assert_eq!(
                (chunk["type"].as_str(), chunk["seq"].as_u64()),
                (Some("look_chunk"), Some(seq))
            );
        }
        // The fifth waits for the phone: a timer never releases it.
        let waiting =
            tokio::time::timeout(std::time::Duration::from_millis(100), sent.recv()).await;
        assert!(waiting.is_err(), "a chunk left beyond the window");
        // Confirming the first chunk (`seq` 0) lets one more go.
        acked.send(1).unwrap();
        assert_eq!(sent_next(&mut sent).await["seq"], WINDOW);
        acked.send(7).unwrap();
        assert_eq!(sent_next(&mut sent).await["seq"], WINDOW + 1);
        assert_eq!(sent_next(&mut sent).await["seq"], WINDOW + 2);
        let end = sent_next(&mut sent).await;
        assert_eq!(
            (end["type"].as_str(), end["view_id"].as_str()),
            (Some("look_end"), Some("d-1"))
        );
        task.await.unwrap();
    }

    #[tokio::test]
    async fn a_document_is_drawn_as_markdown_by_the_desktops_own_rule() {
        for (name, mode) in [
            ("a.md", "markdown"),
            ("a.markdown", "markdown"),
            ("a.mdown", "markdown"),
            ("a.mdx", "text"),
            ("a.txt", "text"),
        ] {
            let looked = view(name, DisplayKind::File);
            let (frames, mut sent) = mpsc::channel(8);
            let (_acked, window) = watch::channel(0_u64);
            let read = Read {
                bytes: b"# title".to_vec(),
                total: 7,
            };
            chunks("r-3", &looked, Shown::Document, read, &frames, window).await;
            assert_eq!(sent_next(&mut sent).await["mode"], mode, "{name}");
        }
    }

    fn change(path: &str, status: ChangedFileStatus, counts: Option<(u32, u32)>) -> LookChange {
        LookChange {
            path: path.to_owned(),
            status,
            added: counts.map(|counts| counts.0),
            removed: counts.map(|counts| counts.1),
        }
    }

    /// B14: a new text file is counted by its lines as git counts a changed
    /// one; "binary" is only what git could not count.
    #[test]
    fn lines_are_counted_as_git_counts_a_new_file() {
        assert_eq!(count_lines(b""), 0);
        assert_eq!(count_lines(b"one\n"), 1);
        assert_eq!(count_lines(b"one\ntwo"), 2);
        assert_eq!(count_lines(b"one\ntwo\n"), 2);
        assert_eq!(count_lines(b"\n\n"), 2);
    }

    #[test]
    fn a_new_text_file_is_counted_and_only_what_git_cannot_count_is_binary() {
        use ChangedFileStatus::{Modified, Untracked};
        let files = [
            change("src/old.rs", Modified, Some((6, 2))),
            change("src/fresh.ts", Untracked, None),
            change("assets/logo.png", Modified, None),
            change("fresh.bin", Untracked, None),
            change("data/huge.csv", Untracked, None),
        ];
        let new_files = HashMap::from([(1, NewFile::Lines(2)), (3, NewFile::Binary)]);
        let frame = changes_frame("r-1", "d-1", &files, &new_files);
        let rows: Vec<(&str, Value, Value, bool)> = frame["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["path"].as_str().unwrap(),
                    row["added"].clone(),
                    row["removed"].clone(),
                    row["binary"].as_bool().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            rows,
            [
                ("src/old.rs", json!(6), json!(2), false),
                ("src/fresh.ts", json!(2), json!(0), false),
                ("assets/logo.png", Value::Null, Value::Null, true),
                ("fresh.bin", Value::Null, Value::Null, true),
                // Not read (past the twenty, or too long): new, neither counted nor binary.
                ("data/huge.csv", Value::Null, Value::Null, false),
            ]
        );
        assert_eq!(
            (frame["added"].as_u64(), frame["removed"].as_u64()),
            (Some(8), Some(2))
        );
        assert_eq!(frame["count"], 5);
    }

    /// B26: a device whose link could not be had is unreachable, not a read
    /// that failed; a linked one says nothing.
    #[test]
    fn a_device_without_a_link_is_unreachable_and_a_linked_one_is_not() {
        assert_eq!(link_state(Ok(()), "d-1"), None);
        assert_eq!(
            link_state(Err("The device helper is not ready".to_owned()), "d-1"),
            Some("device_unreachable")
        );
    }

    #[tokio::test]
    async fn a_document_that_is_not_text_is_named_and_never_sent() {
        let looked = view("notes.txt", DisplayKind::File);
        let (frames, mut sent) = mpsc::channel(8);
        let (_acked, window) = watch::channel(0_u64);
        let read = Read {
            bytes: vec![0x50, 0x4b, 0, 3, 4],
            total: 5,
        };
        chunks("r-2", &looked, Shown::Document, read, &frames, window).await;
        let answer = sent_next(&mut sent).await;
        assert_eq!(
            (answer["type"].as_str(), answer["state"].as_str()),
            (Some("look_result"), Some("other"))
        );
        assert!(sent.try_recv().is_err(), "nothing follows the answer");
    }
}
