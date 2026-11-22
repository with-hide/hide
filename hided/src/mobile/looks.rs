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
//! counts, never a line of code; a page's screen is the picture the desktop
//! app shows of it now (`page_shot.rs`), sent as an image beside its address,
//! the machine that took it and when; an HTML file of a checkout is sent as
//! one document with the files it names folded in (`html_look.rs`), which the
//! phone draws itself.
//!
//! Content leaves as JSON text, so bytes travel as base64 chunks of one relay
//! frame each (`CHUNK`). The relay drops a phone whose eight-frame queue
//! fills, so the connection keeps at most `LOOK_FRAMES` of these frames that
//! the phone has not confirmed (`look_ack`), whichever transfer sent them
//! (`Pace`): a transfer is paced by the phone and never by a timer.

use std::sync::Arc;

use axum::extract::ws::Message;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hide_core::agent_state::phone::FRAME_LIMIT;
use hide_core::{ChangedFileStatus, DisplayKind, LookChange, LookView, LookViews};
use hide_relay_protocol::{frame::MAX_QUEUED_FRAMES, noise::CHUNK_DATA};
use serde_json::{Value, json};
use tokio::sync::{mpsc, watch};

use super::Mobile;
use super::html_look::{self, Place};
use super::page_shot;
use super::projection::AgentKey;
use crate::server;

/// The most a text document sends: the first 1 MiB, then "the rest is on the
/// Mac" (D-10).
pub const TEXT_CAP: u64 = 1024 * 1024;
/// The largest image a phone is sent (D-10).
pub const IMAGE_CAP: u64 = 10 * 1024 * 1024;
/// A `look_chunk` frame's text beside its base64: its type, a request id of
/// at most 64 characters (`request`), a sequence number and the JSON around
/// them.
const CHUNK_FRAME_EXTRA: usize = 160;
/// Raw bytes in one chunk: the most whose frame is one relay frame, in whole
/// three-byte groups, so the phone joins the chunks' base64 as one text.
const CHUNK: usize = (CHUNK_DATA - CHUNK_FRAME_EXTRA) / 4 * 3;
/// The relay frames of View content a connection may have out that the phone
/// has not confirmed: the socket's eight less the relay frames of the largest
/// frame the phone is sent beside them (`FRAME_LIMIT`, five), so a list or a
/// conversation page of any size still fits beside a transfer.
/// That holds while the phone opens one View at a time. A View opened before
/// the last one's chunk 0 was confirmed adds that transfer's head and chunk 0
/// (`Ledger::may_send`): one such reopen leaves three frames, about 196 KiB,
/// for the rest, and only a phone that opens three Views in a row without
/// receiving anything can fill the queue (`docs/ARCHITECTURE.md`).
const LOOK_FRAMES: u64 = (MAX_QUEUED_FRAMES - FRAME_LIMIT.div_ceil(CHUNK_DATA)) as u64;
const _: () = assert!(CHUNK.is_multiple_of(3));
const _: () = assert!(CHUNK.div_ceil(3) * 4 + CHUNK_FRAME_EXTRA <= CHUNK_DATA);
// A head and a chunk 0 must fit, or a View could never start.
const _: () = assert!(LOOK_FRAMES >= 2);
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
    /// An HTML file of a checkout, which the phone draws itself from the
    /// page and the files beside it (`html_look`), however it was opened.
    Html,
    /// A document drawn from its text.
    Document,
    Image,
    /// Named, never read: a PDF, a video, a file outside every checkout.
    NameOnly,
}

impl Shown {
    pub fn of(view: &LookView) -> Self {
        if html_path(view).is_some() {
            return Self::Html;
        }
        match view.kind {
            DisplayKind::Diff => Self::Changes,
            DisplayKind::Browser => Self::Page,
            // A card has no opener, so no agent's list holds one.
            DisplayKind::External | DisplayKind::Card => Self::NameOnly,
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
            Self::Html => "html",
            Self::Document => "doc",
            Self::Image => "image",
            Self::NameOnly => "other",
        }
    }
}

/// The wire path of the HTML file a View shows, when it shows one inside its
/// checkout: a file View of an HTML file, or a page View whose address is
/// such a file. The desktop's rule for which file is HTML decides.
fn html_path(view: &LookView) -> Option<String> {
    let path = match view.kind {
        DisplayKind::File => view.path.clone(),
        DisplayKind::Browser => crate::file_url::file_path(view.url.as_deref()?)?.0,
        DisplayKind::Diff | DisplayKind::External | DisplayKind::Card => return None,
    };
    let name = std::path::Path::new(hide_platform::path::wire_name(&path));
    let html = hide_host::document::language_for(name).as_deref() == Some("html");
    (html && Place::of(&view.checkout_path, &path).is_some()).then_some(path)
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
            let shown = Shown::of(view);
            let mut row = json!({
                "id": view.display_id,
                "kind": shown.word(),
                "name": cut(&view.label, NAME_CHARS),
                "unseen": view.unseen,
                "at": view.at_unix_ms,
            });
            // A local page's address is its path, which a phone never gets.
            if let Some(url) = view.url.as_ref().filter(|_| shown != Shown::Html) {
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

/// The View frames one phone connection has sent and not yet known to have
/// left the relay's queue. One per connection, for its whole life, shared by
/// every transfer it runs: a View the phone gave up, closed or replaced
/// leaves its frames in the relay's queue ahead of the next one's, and the
/// phone drops them without a confirmation.
pub struct Pace {
    ledger: watch::Sender<Ledger>,
    /// Held while a frame is numbered and handed to the socket, so frames
    /// reach the socket in the order they are numbered.
    sending: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Ledger {
    /// View frames numbered on this connection.
    sent: u64,
    /// Every frame numbered below this has left the relay's queue.
    drained: u64,
    /// The one transfer that may send; an earlier one stops at its next frame.
    current: Option<Current>,
}

struct Current {
    request_id: String,
    /// The number of its chunk 0, once sent.
    first_chunk: u64,
    chunks_sent: u64,
    /// Whether the phone confirmed any of its chunks.
    confirmed: bool,
}

/// Which frame of a transfer is to be sent.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Part {
    Head,
    Chunk,
    End,
}

impl Ledger {
    /// Whether `request_id` may send its next frame now; `None` once a later
    /// transfer took its place.
    fn may_send(&self, request_id: &str) -> Option<bool> {
        let current = self
            .current
            .as_ref()
            .filter(|current| current.request_id == request_id)?;
        // Only a confirmed chunk of this transfer proves that what earlier
        // ones left in the queue is gone (the relay keeps a socket's frames in
        // order), so its head and chunk 0, or its end when it has none, go
        // whatever is still out: holding them back could wait forever.
        let opening = !current.confirmed && current.chunks_sent == 0;
        Some(opening || self.sent - self.drained < LOOK_FRAMES)
    }

    /// Numbers the next frame of `request_id`; false once it is not current.
    fn record(&mut self, request_id: &str, part: Part) -> bool {
        let number = self.sent;
        let Some(current) = self
            .current
            .as_mut()
            .filter(|current| current.request_id == request_id)
        else {
            return false;
        };
        if part == Part::Chunk {
            if current.chunks_sent == 0 {
                current.first_chunk = number;
            }
            current.chunks_sent += 1;
        }
        self.sent += 1;
        true
    }
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            ledger: watch::channel(Ledger::default()).0,
            sending: tokio::sync::Mutex::new(()),
        }
    }
}

impl Pace {
    /// `request_id` becomes the one transfer that sends; the one before stops.
    fn begin(&self, request_id: &str) {
        self.ledger.send_modify(|ledger| {
            ledger.current = Some(Current {
                request_id: request_id.to_owned(),
                first_chunk: 0,
                chunks_sent: 0,
                confirmed: false,
            });
        });
    }

    /// The phone received chunk `seq` of `request_id`, and with it every frame
    /// sent before it. A confirmation of another request, or of a chunk not
    /// sent, proves nothing and changes nothing.
    pub fn ack(&self, request_id: &str, seq: u64) {
        self.ledger.send_if_modified(|ledger| {
            let Some(current) = ledger
                .current
                .as_mut()
                .filter(|current| current.request_id == request_id && seq < current.chunks_sent)
            else {
                return false;
            };
            current.confirmed = true;
            let drained = current.first_chunk + seq + 1;
            let changed = drained > ledger.drained;
            ledger.drained = ledger.drained.max(drained);
            changed
        });
    }

    /// Sends one frame of `request_id` once the ledger has room for it; false
    /// when a later transfer took its place or the socket is gone.
    async fn send(
        &self,
        request_id: &str,
        part: Part,
        frame: Value,
        frames: &mpsc::Sender<Value>,
    ) -> bool {
        let mut ledger = self.ledger.subscribe();
        let room = ledger
            .wait_for(|ledger| ledger.may_send(request_id) != Some(false))
            .await
            .map(|ledger| ledger.may_send(request_id).is_some());
        if !matches!(room, Ok(true)) {
            return false;
        }
        let _sending = self.sending.lock().await;
        let mut current = false;
        self.ledger
            .send_modify(|ledger| current = ledger.record(request_id, part));
        current && frames.send(frame).await.is_ok()
    }
}

/// A transfer in progress; it ends with the detail or with the next request.
pub struct Transfer {
    task: tokio::task::JoinHandle<()>,
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
    pace: &Arc<Pace>,
    request_id: &str,
    view: LookView,
    frames: mpsc::Sender<Value>,
) -> Transfer {
    pace.begin(request_id);
    let (mobile, pace) = (Arc::clone(mobile), Arc::clone(pace));
    let id = request_id.to_owned();
    let task = tokio::spawn(async move {
        send(&mobile, &id, &view, &frames, &pace).await;
    });
    Transfer { task }
}

async fn send(
    mobile: &Arc<Mobile>,
    request_id: &str,
    view: &LookView,
    frames: &mpsc::Sender<Value>,
    pace: &Pace,
) {
    let shown = Shown::of(view);
    let answer = match shown {
        Shown::Changes => changes(mobile, request_id, view).await,
        Shown::Page => match page_shot::take(mobile, view, IMAGE_CAP).await {
            Ok(shot) => {
                let read = Read {
                    total: shot.jpeg.len() as u64,
                    bytes: shot.jpeg,
                    page: Some(PageFacts {
                        url: cut(view.url.as_deref().unwrap_or(""), URL_CHARS),
                        taken_unix_ms: shot.taken_unix_ms,
                        device_id: shot.device_id,
                    }),
                };
                return chunks(request_id, view, shown, read, frames, pace).await;
            }
            // The address is all the phone has when the picture cannot be
            // taken (B21); why is in the diagnostic log.
            Err(_) => {
                let mut frame = result(request_id, &view.display_id, "page");
                frame["url"] = json!(cut(view.url.as_deref().unwrap_or(""), URL_CHARS));
                frame
            }
        },
        Shown::NameOnly => result(request_id, &view.display_id, "other"),
        Shown::Html => match html_page(mobile, request_id, view).await {
            Ok(read) => return chunks(request_id, view, shown, read, frames, pace).await,
            Err(state) => result(request_id, &view.display_id, state),
        },
        Shown::Document | Shown::Image => {
            let cap = if shown == Shown::Image {
                IMAGE_CAP
            } else {
                TEXT_CAP
            };
            match read(
                mobile,
                request_id,
                view,
                &view.path,
                cap,
                shown == Shown::Image,
            )
            .await
            {
                Ok(read) => return chunks(request_id, view, shown, read, frames, pace).await,
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
            .and_then(hide_core::PreparedChanges::read)
    })
    .await;
    let files = match read {
        Ok(Ok(files)) => files,
        Ok(Err(reason)) => {
            hide_core::diagnostic!(json!({
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
            hide_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "look.changes_failed",
                "view_id": view.display_id, "message": error.to_string(),
            }));
            return Err("unavailable");
        }
    };
    Ok(changes_frame(request_id, &view.display_id, &files))
}

/// The `look_changes` frame: every file's status and counts, never a line of
/// code. The desktop's list shows no number for a file Git does not track yet,
/// and neither does this one; a file is `binary` only when Git tracks it and
/// could not count it.
fn changes_frame(request_id: &str, view_id: &str, files: &[LookChange]) -> Value {
    let (mut added, mut removed) = (0_u64, 0_u64);
    let mut rows = Vec::new();
    for (index, file) in files.iter().enumerate() {
        added += u64::from(file.added.unwrap_or(0));
        removed += u64::from(file.removed.unwrap_or(0));
        if index < CHANGE_FILES {
            let binary = file.added.is_none()
                && file.removed.is_none()
                && file.status != ChangedFileStatus::Untracked;
            rows.push(json!({
                "path": cut(&file.path, URL_CHARS),
                "status": file.status,
                "added": file.added,
                "removed": file.removed,
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
    /// What a page's picture carries beside its bytes; absent for a file.
    page: Option<PageFacts>,
}

/// A page's picture is the page as of a moment, on one machine's desktop.
struct PageFacts {
    url: String,
    taken_unix_ms: u64,
    device_id: String,
}

/// Reads `path` of the View's checkout up to `cap` bytes through the
/// desktop's readers. The error is the `look_result` state to answer.
async fn read(
    mobile: &Arc<Mobile>,
    request_id: &str,
    view: &LookView,
    path: &str,
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
            "root": view.checkout_path, "path": path,
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
                let reason = refusal_reason(&text);
                hide_core::diagnostic!(json!({
                    "component": "mobile_phone", "kind": "look.read_refused",
                    "view_id": view.display_id, "reason": reason,
                }));
                break Err(refusal_state(&reason));
            }
            _ => break Err("failed"),
        }
    };
    reading.abort();
    outcome?;
    Ok(Read {
        bytes,
        total,
        page: None,
    })
}

/// An HTML file with the files of its checkout that it names folded in
/// (`html_look`): the page is read whole up to the image cap, then each named
/// file, then what its stylesheets name, while they fit under that cap and
/// `MAX_FILES`. A file that is missing, outside the checkout or past either
/// limit stays as its address and draws as missing; how many is logged.
async fn html_page(
    mobile: &Arc<Mobile>,
    request_id: &str,
    view: &LookView,
) -> Result<Read, &'static str> {
    let path = html_path(view).ok_or("failed")?;
    let place = Place::of(&view.checkout_path, &path).ok_or("failed")?;
    let page = read(mobile, request_id, view, &path, IMAGE_CAP, true).await?;
    let text = String::from_utf8(page.bytes).map_err(|_| "other")?;
    let page_dir = place.page.parent().unwrap_or_default();
    let mut budget = IMAGE_CAP.saturating_sub(text.len() as u64);
    let mut read_files: Vec<(hide_platform::path::RelPath, Vec<u8>)> = Vec::new();
    let mut left = 0_usize;
    let mut level = html_look::wanted(&text, &page_dir, false);
    for depth in 1..=html_look::MAX_DEPTH {
        let mut next = Vec::new();
        for file in level {
            if read_files.iter().any(|(read, _)| *read == file) {
                continue;
            }
            // Base64 is four bytes for every three.
            let room = budget / 4 * 3;
            if read_files.len() >= html_look::MAX_FILES || room == 0 {
                left += 1;
                continue;
            }
            let Ok(bytes) = read(mobile, request_id, view, &place.wire(&file), room, true)
                .await
                .map(|read| read.bytes)
            else {
                left += 1;
                continue;
            };
            budget = budget.saturating_sub((bytes.len() as u64).div_ceil(3) * 4);
            if html_look::is_stylesheet(&file)
                && depth < html_look::MAX_DEPTH
                && let Ok(css) = std::str::from_utf8(&bytes)
            {
                let dir = file.parent().unwrap_or_default();
                next.extend(html_look::wanted(css, &dir, true));
            }
            read_files.push((file, bytes));
        }
        level = next;
    }
    if left > 0 {
        hide_core::diagnostic!(json!({
            "component": "mobile_phone", "kind": "look.html_files_left",
            "view_id": view.display_id, "count": left,
        }));
    }
    let folded = html_look::data_addresses(&read_files);
    let document = html_look::fold(&text, &page_dir, false, &folded);
    // A file named many times is folded in at each place it is named.
    if document.len() as u64 > IMAGE_CAP {
        return Err("too_large");
    }
    Ok(Read {
        total: document.len() as u64,
        bytes: document.into_bytes(),
        page: None,
    })
}

/// The `look_result` state for a device whose link could not be had: it is
/// `device_unreachable`, and why goes to the log (B26). A link that is up says
/// nothing.
fn link_state(link: Result<(), String>, view_id: &str) -> Option<&'static str> {
    let reason = link.err()?;
    hide_core::diagnostic!(json!({
        "component": "mobile_phone", "kind": "look.device_unreachable",
        "view_id": view_id, "reason": reason,
    }));
    Some("device_unreachable")
}

/// Why the readers' text frame (`path_refused`, `file_bytes_error`) refused
/// a read.
fn refusal_reason(text: &str) -> String {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|frame| {
            frame
                .pointer("/payload/reason")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default()
}

/// The `look_result` state for why the readers refused a file: one that is
/// gone since the View opened is `not_found`, one outside every checkout this
/// Mac reads is `outside_roots`, and the phone shows either as its reason with
/// nothing to retry; any other refusal is a read that failed.
fn refusal_state(reason: &str) -> &'static str {
    match reason {
        "too_large" => "too_large",
        "not_found" => "not_found",
        "outside_checkout" | "outside_home" => "outside_roots",
        _ => "failed",
    }
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
    pace: &Pace,
) {
    let id = &view.display_id;
    if shown == Shown::Document && !is_text(&read.bytes) {
        let _ = frames.send(result(request_id, id, "other")).await;
        return;
    }
    let mode = match shown {
        Shown::Image => "image",
        Shown::Page => "page",
        Shown::Html => "html",
        // The desktop's own rule for what a Markdown file is, so the two draw alike.
        _ if hide_host::document::language_for(std::path::Path::new(&view.path)).as_deref()
            == Some("markdown") =>
        {
            "markdown"
        }
        _ => "text",
    };
    let size = read.bytes.len();
    let mut head = json!({
        "type": "look_head", "request_id": request_id, "view_id": id, "mode": mode,
        "size": size, "total": read.total.max(size as u64),
        "truncated": read.total > size as u64,
        "chunks": size.div_ceil(CHUNK),
    });
    if let Some(page) = &read.page {
        head["url"] = json!(page.url);
        head["taken_at"] = json!(page.taken_unix_ms);
        head["taken_by"] = json!(page.device_id);
    }
    if !pace.send(request_id, Part::Head, head, frames).await {
        return;
    }
    for (seq, bytes) in read.bytes.chunks(CHUNK).enumerate() {
        let frame = json!({
            "type": "look_chunk", "request_id": request_id, "seq": seq,
            "data": STANDARD.encode(bytes),
        });
        if !pace.send(request_id, Part::Chunk, frame, frames).await {
            return;
        }
    }
    let end = json!({"type": "look_end", "request_id": request_id, "view_id": id});
    pace.send(request_id, Part::End, end, frames).await;
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
        // An HTML file of the checkout is drawn by the phone, opened as a
        // file or as a page; a web address and a file elsewhere are not.
        assert_eq!(
            shown("mock.html", DisplayKind::File, Some("document")),
            Shown::Html
        );
        assert_eq!(shown("mock.HTM", DisplayKind::File, None), Shown::Html);
        let page = |url: &str| {
            let mut view = view("", DisplayKind::Browser);
            view.path = String::new();
            view.url = Some(url.into());
            Shown::of(&view)
        };
        assert_eq!(page("file:///repo/design/mock.html"), Shown::Html);
        assert_eq!(page("file:///repo/design/my%20mock.html#top"), Shown::Html);
        assert_eq!(page("file:///repo/design/logo.svg"), Shown::Page);
        assert_eq!(page("file:///elsewhere/mock.html"), Shown::Page);
        assert_eq!(page("https://example.com/mock.html"), Shown::Page);
    }

    #[test]
    fn a_local_page_is_listed_without_its_address() {
        let key = AgentKey {
            device_id: "test-node".into(),
            pane_id: "w1:p1".into(),
        };
        let mut local = view("mock.html", DisplayKind::Browser);
        local.url = Some("file:///repo/mock.html".into());
        let mut web = view("site", DisplayKind::Browser);
        web.url = Some("https://example.com/".into());
        let frame = list_frame(
            &key,
            &LookViews {
                views: vec![local, web],
                omitted: 0,
            },
        );
        assert_eq!(frame["views"][0]["kind"], "html");
        assert!(frame["views"][0].get("url").is_none());
        assert_eq!(frame["views"][1]["kind"], "page");
        assert_eq!(frame["views"][1]["url"], "https://example.com/");
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

    /// A ledger with `request_id` as its transfer, as `open` leaves it.
    fn paced(request_id: &str) -> Arc<Pace> {
        let pace = Arc::new(Pace::default());
        pace.begin(request_id);
        pace
    }

    /// Whether a frame is waiting: with time paused, the wait ends as soon as
    /// every task is idle, so this says the transfer stopped, not that it was
    /// slow.
    async fn nothing_waiting(frames: &mut mpsc::Receiver<Value>) -> bool {
        tokio::time::timeout(std::time::Duration::from_secs(60), frames.recv())
            .await
            .is_err()
    }

    /// Spawns the transfer of `bytes` under `request_id` on `pace`.
    fn transfer(
        pace: &Arc<Pace>,
        request_id: &str,
        bytes: Vec<u8>,
        frames: &mpsc::Sender<Value>,
    ) -> tokio::task::JoinHandle<()> {
        let (pace, frames, request_id) = (Arc::clone(pace), frames.clone(), request_id.to_owned());
        tokio::spawn(async move {
            let read = Read {
                total: bytes.len() as u64,
                bytes,
                page: None,
            };
            let looked = view("photo.png", DisplayKind::File);
            chunks(&request_id, &looked, Shown::Image, read, &frames, &pace).await;
        })
    }

    #[tokio::test(start_paused = true)]
    async fn chunks_go_out_only_as_far_as_the_phone_confirmed() {
        let looked = view("notes.md", DisplayKind::File);
        let bytes = vec![b'a'; CHUNK * 6 + 10];
        let total = bytes.len() as u64 + 5;
        let (frames, mut sent) = mpsc::channel(64);
        let pace = paced("r-1");
        let paced_here = Arc::clone(&pace);
        let task = tokio::spawn(async move {
            let read = Read {
                bytes,
                total,
                page: None,
            };
            chunks("r-1", &looked, Shown::Document, read, &frames, &paced_here).await;
        });
        let head = sent_next(&mut sent).await;
        assert_eq!(head["type"], "look_head");
        assert_eq!(head["mode"], "markdown");
        assert_eq!(head["chunks"], 7);
        assert_eq!(head["truncated"], true);
        // The head and the chunks after it fill the ledger.
        for seq in 0..LOOK_FRAMES - 1 {
            let chunk = sent_next(&mut sent).await;
            assert_eq!(
                (chunk["type"].as_str(), chunk["seq"].as_u64()),
                (Some("look_chunk"), Some(seq))
            );
        }
        // The next waits for the phone: a timer never releases it, nor a
        // confirmation of a chunk that was not sent or of another request.
        assert!(
            nothing_waiting(&mut sent).await,
            "a chunk left past the ledger"
        );
        pace.ack("r-1", 6);
        pace.ack("r-0", 0);
        assert!(
            nothing_waiting(&mut sent).await,
            "an ack of nothing sent made room"
        );
        // Chunk 0 confirmed: it and the head have left, so two more go.
        pace.ack("r-1", 0);
        assert_eq!(sent_next(&mut sent).await["seq"], LOOK_FRAMES - 1);
        assert_eq!(sent_next(&mut sent).await["seq"], LOOK_FRAMES);
        assert!(nothing_waiting(&mut sent).await);
        for seq in LOOK_FRAMES..=6 {
            pace.ack("r-1", seq);
        }
        let mut rest = Vec::new();
        while let Ok(frame) =
            tokio::time::timeout(std::time::Duration::from_secs(1), sent.recv()).await
        {
            let frame = frame.expect("the transfer is still sending");
            if let Some(seq) = frame["seq"].as_u64() {
                pace.ack("r-1", seq);
            }
            rest.push(frame);
            if rest.last().is_some_and(|frame| frame["type"] == "look_end") {
                break;
            }
        }
        let seqs: Vec<u64> = rest
            .iter()
            .filter_map(|frame| frame["seq"].as_u64())
            .collect();
        assert_eq!(seqs, (LOOK_FRAMES + 1..7).collect::<Vec<_>>());
        let end = rest.last().unwrap();
        assert_eq!(
            (end["type"].as_str(), end["view_id"].as_str()),
            (Some("look_end"), Some("d-1"))
        );
        task.await.unwrap();
    }

    /// The relay frames a phone that reads nothing is left with after it
    /// opened `opens` Views, each replacing the last (`closed_between`: the
    /// detail closed in between, which ends the transfer before the next
    /// begins), counted as the relay counts them by sealing each frame in a
    /// real session.
    async fn queued_after(opens: usize, closed_between: bool) -> usize {
        use hide_relay_protocol::noise::{NoiseKeypair, Session};
        let key = NoiseKeypair::generate().unwrap();
        let mut phone = Session::initiator(&key.public_key(), 7).unwrap();
        let mut device = Session::responder(&key, 7).unwrap();
        device
            .read_handshake(&phone.write_handshake().unwrap())
            .unwrap();
        phone
            .read_handshake(&device.write_handshake().unwrap())
            .unwrap();

        // One ledger for the connection's life, as `phone.rs` keeps it.
        let pace = Arc::new(Pace::default());
        let (frames, mut sent) = mpsc::channel(64);
        let mut queued = 0;
        let mut running: Option<tokio::task::JoinHandle<()>> = None;
        for open in 0..opens {
            // The longest request id a phone may send, and the largest image.
            let request_id = format!("{open:0>64}");
            if closed_between && let Some(task) = running.take() {
                task.abort();
            }
            pace.begin(&request_id);
            if let Some(task) = running.take() {
                task.abort();
            }
            running = Some(transfer(
                &pace,
                &request_id,
                vec![0x89; IMAGE_CAP as usize],
                &frames,
            ));
            while let Ok(Some(frame)) =
                tokio::time::timeout(std::time::Duration::from_secs(60), sent.recv()).await
            {
                let relay_frames = device.send_message(frame.to_string().as_bytes()).unwrap();
                if frame["type"] == "look_chunk" {
                    assert_eq!(relay_frames.len(), 1, "a chunk is one relay frame");
                }
                queued += relay_frames.len();
            }
            assert!(
                !running.as_ref().unwrap().is_finished(),
                "the transfer waits for the phone"
            );
        }
        running.unwrap().abort();
        queued
    }

    /// A phone that reads nothing confirms nothing: every View frame sent sits
    /// in the relay's queue for it, and the relay drops a phone whose queue
    /// fills. One View leaves room for the largest frame the phone is sent
    /// beside it; each View opened before anything arrived adds only its head
    /// and chunk 0, closed in between or not.
    #[tokio::test(start_paused = true)]
    async fn a_phone_that_reads_nothing_never_fills_its_relay_queue() {
        let largest_beside = FRAME_LIMIT.div_ceil(CHUNK_DATA);
        let one = queued_after(1, false).await;
        assert_eq!(one as u64, LOOK_FRAMES);
        assert!(
            one + largest_beside <= MAX_QUEUED_FRAMES,
            "{one} + {largest_beside}"
        );
        for opens in [2, 3] {
            for closed_between in [false, true] {
                let queued = queued_after(opens, closed_between).await as u64;
                assert!(
                    queued <= LOOK_FRAMES + 2 * (opens as u64 - 1),
                    "{queued} frames after {opens} opens (closed between: {closed_between})"
                );
            }
        }
        assert!(queued_after(2, false).await < MAX_QUEUED_FRAMES);
    }

    /// The ledger never waits on frames no confirmation will clear: a View
    /// with no chunk (its head and end) and the end of a finished View are
    /// never confirmed, and the View opened after them still runs to its end.
    #[tokio::test(start_paused = true)]
    async fn a_view_after_unconfirmed_frames_still_finishes() {
        let pace = Arc::new(Pace::default());
        let (frames, mut sent) = mpsc::channel(64);
        // A finished View whose chunk was confirmed, its end not.
        pace.begin("r-1");
        let first = transfer(&pace, "r-1", vec![1; 10], &frames);
        assert_eq!(sent_next(&mut sent).await["type"], "look_head");
        assert_eq!(sent_next(&mut sent).await["seq"], 0);
        pace.ack("r-1", 0);
        assert_eq!(sent_next(&mut sent).await["type"], "look_end");
        first.await.unwrap();
        // An empty View: a head and an end, nothing to confirm.
        pace.begin("r-2");
        let empty = transfer(&pace, "r-2", Vec::new(), &frames);
        assert_eq!(sent_next(&mut sent).await["type"], "look_head");
        assert_eq!(sent_next(&mut sent).await["type"], "look_end");
        empty.await.unwrap();
        // The next View goes to its end, the phone confirming each chunk.
        pace.begin("r-3");
        let last = transfer(&pace, "r-3", vec![1; CHUNK * 5], &frames);
        let mut chunks = 0;
        loop {
            let frame = sent_next(&mut sent).await;
            if let Some(seq) = frame["seq"].as_u64() {
                pace.ack("r-3", seq);
                chunks += 1;
            }
            if frame["type"] == "look_end" {
                break;
            }
        }
        assert_eq!(chunks, 5);
        last.await.unwrap();
    }

    /// A file deleted since its View opened, and one outside every checkout,
    /// are answered with their reason, which the phone shows with nothing to
    /// retry; the readers' answer is what `read` turns into the state.
    #[tokio::test]
    async fn a_file_gone_or_outside_every_checkout_is_answered_with_its_reason() {
        let home = tempfile::tempdir().unwrap();
        let checkout = home.path().join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        std::fs::write(checkout.join("notes.md"), "# notes").unwrap();
        std::fs::write(home.path().join("secret.md"), "no").unwrap();
        let boundary = crate::boundary::Boundary::new(home.path()).unwrap();
        boundary.set_roots(vec![crate::boundary::Root {
            workspace_id: "w1".to_owned(),
            checkout_id: "c1".to_owned(),
            path: checkout.clone(),
        }]);
        let wire = |path: std::path::PathBuf| hide_platform::path::to_wire_lossy(&path);
        for (path, state) in [
            (wire(checkout.join("gone.md")), "not_found"),
            (wire(home.path().join("secret.md")), "outside_roots"),
        ] {
            let event = json!({"payload": {"request_id": "r-1", "path": path, "offset": 0, "length": TEXT_CAP}});
            let (mut sink, mut answers) = mpsc::channel::<Message>(2);
            server::send_opened_file_bytes(&mut sink, &event, boundary.open_file(&path))
                .await
                .unwrap();
            let Some(Message::Text(text)) = answers.recv().await else {
                panic!("{path}: no refusal");
            };
            assert_eq!(
                refusal_state(&refusal_reason(&text)),
                state,
                "{path}: {text}"
            );
        }
    }

    /// A device's helper judges its own paths: its refusals reach the phone
    /// as the same reasons this Mac's boundary gives, and anything else it
    /// answers is a read that failed.
    #[test]
    fn a_devices_refusal_reads_as_this_macs() {
        use hide_core::node_access::LinkError;
        use hide_node_link::{ErrorCode, HostError};
        let state = |error: LinkError| {
            refusal_state(
                server::device_refusal(&error).map_or("read_failed", |refusal| refusal.code()),
            )
        };
        let refused = |code| LinkError::Refused(HostError::new(code, "refused"));
        assert_eq!(state(refused(ErrorCode::NotFound)), "not_found");
        assert_eq!(state(refused(ErrorCode::OutsideRoot)), "outside_roots");
        assert_eq!(state(refused(ErrorCode::InvalidPath)), "outside_roots");
        for code in [
            ErrorCode::PermissionDenied,
            ErrorCode::RootReplaced,
            ErrorCode::Io,
        ] {
            assert_eq!(state(refused(code)), "failed", "{code:?}");
        }
        assert_eq!(state(LinkError::Busy), "failed");
        assert_eq!(state(LinkError::NotConnected("gone".to_owned())), "failed");
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
            let read = Read {
                bytes: b"# title".to_vec(),
                total: 7,
                page: None,
            };
            chunks(
                "r-3",
                &looked,
                Shown::Document,
                read,
                &frames,
                &paced("r-3"),
            )
            .await;
            assert_eq!(sent_next(&mut sent).await["mode"], mode, "{name}");
        }
    }

    /// B20: the picture of a page leaves as chunks like an image, and its head
    /// says the address, when it was taken and by which machine's desktop.
    #[tokio::test]
    async fn a_pages_picture_is_sent_with_its_address_time_and_machine() {
        let looked = view("localhost", DisplayKind::Browser);
        let (frames, mut sent) = mpsc::channel(8);
        let read = Read {
            bytes: vec![0xff, 0xd8, 0xff, 0xe0, 1, 2, 3],
            total: 7,
            page: Some(PageFacts {
                url: "http://localhost:5173/".to_owned(),
                taken_unix_ms: 1_700_000_000_000,
                device_id: "mac-1".to_owned(),
            }),
        };
        chunks("r-5", &looked, Shown::Page, read, &frames, &paced("r-5")).await;
        let head = sent_next(&mut sent).await;
        assert_eq!(head["type"], "look_head");
        assert_eq!(head["mode"], "page");
        assert_eq!(head["url"], "http://localhost:5173/");
        assert_eq!(head["taken_at"], 1_700_000_000_000_u64);
        assert_eq!(head["taken_by"], "mac-1");
        assert_eq!(head["size"], 7);
        assert_eq!(sent_next(&mut sent).await["type"], "look_chunk");
        assert_eq!(sent_next(&mut sent).await["type"], "look_end");
    }

    fn change(path: &str, status: ChangedFileStatus, counts: Option<(u32, u32)>) -> LookChange {
        LookChange {
            path: path.to_owned(),
            status,
            added: counts.map(|counts| counts.0),
            removed: counts.map(|counts| counts.1),
        }
    }

    /// B14: the desktop's list shows no number for a file Git does not track
    /// yet, so the phone shows `새 파일` alone; `binary` is only for a tracked
    /// file Git could not count.
    #[test]
    fn an_untracked_file_has_no_counts_and_is_no_binary() {
        use ChangedFileStatus::{Modified, Untracked};
        let files = [
            change("src/old.rs", Modified, Some((6, 2))),
            change("src/fresh.ts", Untracked, None),
            change("assets/logo.png", Modified, None),
        ];
        let frame = changes_frame("r-1", "d-1", &files);
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
                ("src/fresh.ts", Value::Null, Value::Null, false),
                ("assets/logo.png", Value::Null, Value::Null, true),
            ]
        );
        assert_eq!(
            (frame["added"].as_u64(), frame["removed"].as_u64()),
            (Some(6), Some(2))
        );
        assert_eq!(frame["count"], 3);
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
        let read = Read {
            bytes: vec![0x50, 0x4b, 0, 3, 4],
            total: 5,
            page: None,
        };
        chunks(
            "r-2",
            &looked,
            Shown::Document,
            read,
            &frames,
            &paced("r-2"),
        )
        .await;
        let answer = sent_next(&mut sent).await;
        assert_eq!(
            (answer["type"].as_str(), answer["state"].as_str()),
            (Some("look_result"), Some("other"))
        );
        assert!(sent.try_recv().is_err(), "nothing follows the answer");
    }
}
