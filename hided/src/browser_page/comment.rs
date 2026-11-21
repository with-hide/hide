//! The operator's Comment mode on a browser View (PRD view-opener-feedback
//! D-07, B9-B11): a page is a native view the shell cannot draw over, so
//! while Comment is on, hided holds the View's debugger lease and draws
//! `comment.js` in an isolated world of the page, where the operator points
//! at an element or drags an area.
//!
//! The overlay only points. Each place it reports is taken as a screenshot
//! at once and kept here as the session's one pick; the screen then draws
//! the note over the page's still, in the operator's window, and its Add
//! names that pick (`take_pick`). The note is never in the page's document,
//! so the page's scripts can neither read nor change what the operator
//! writes, and nothing the page does can make a comment (D-17): the binding
//! the overlay reports through exists only in the `hide-comment` world, and
//! the overlay reads only trusted input.
//!
//! A session is the View's in its checkout (display ids repeat across
//! checkouts), belongs to the screen that turned it on and ends with it,
//! with Comment off or Esc, and when the page's lease ends. While a session
//! holds the lease, an agent's `hide browser` command on that page is
//! answered `display_busy` (the PRD's known effect).
//!
//! A comment's screenshot is the page clipped to what it points at; the
//! screen's feedback queue stages it on this machine's state folder, and
//! only its name reaches the core (D-23).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};

use super::Failure;
use super::cdp::Cdp;
use super::page::Page;

const COMMENT_JS: &str = include_str!("../../assets/browser/comment.js");
const WORLD: &str = "hide-comment";
const BINDING: &str = "__hideCommentSend";
/// The events a session reads besides a command's.
const EVENTS: &[&str] = &[
    "Runtime.bindingCalled",
    "Page.frameNavigated",
    "Page.navigatedWithinDocument",
    "Inspector.detached",
];
/// Comment sessions at once: one per View the operator comments on.
const MAX_SESSIONS: usize = 4;
/// What one overlay message may be: a place, its selector and its text.
const MAX_MESSAGE_BYTES: usize = 16 * 1024;
const CALL: Duration = Duration::from_secs(10);
/// The most pixels a screenshot clip keeps; a larger area is scaled down
/// so the capture stays well under the gateway's message cap.
const MAX_SHOT_PIXELS: f64 = 2_000_000.0;

/// What a session needs of the daemon.
#[derive(Clone)]
pub struct Context {
    pub control: Arc<crate::browser_control::BrowserControl>,
}

/// The page's pins again, while the mode is on.
struct Pins {
    pins: Value,
    strings: Value,
}

/// The place the operator picked last, waiting for its note: what the
/// overlay said it points at and the page's screenshot of it.
struct Pick {
    id: u64,
    anchor: Value,
    png: Option<Vec<u8>>,
}

/// A note the screen added on the session's pick: the core's `view_comment`
/// add, and the screenshot to stage for it.
pub struct Added {
    pub event: Value,
    pub png: Option<Vec<u8>>,
}

struct Session {
    connection: u64,
    generation: u64,
    pins: mpsc::Sender<Pins>,
    /// Ending the session never waits on the pins' queue: taking this ends
    /// it, so a full queue cannot keep a page held (`None` once it is ending).
    off: Option<oneshot::Sender<()>>,
    /// One at a time: a new pick replaces one never written.
    pick: Option<Pick>,
}

/// A View in its checkout: device, checkout path and display id.
type SessionKey = (String, String, String);

#[derive(Default)]
pub struct CommentSessions {
    sessions: Mutex<HashMap<SessionKey, Session>>,
    generation: std::sync::atomic::AtomicU64,
}

/// The screen's request, `browser_comment`.
struct Request {
    device_id: String,
    path: String,
    area_id: String,
    display_id: String,
    on: bool,
    /// Only the pins again: never starts a session.
    update: bool,
    pins: Value,
    strings: Value,
    /// A note on the session's pick: its id and the note.
    add: Option<(u64, String)>,
}

fn request(event: &Value) -> Result<Request, String> {
    let payload = event
        .get("payload")
        .ok_or("browser_comment has no payload")?;
    let text = |pointer: &str| {
        payload
            .pointer(pointer)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 4096)
            .map(str::to_owned)
            .ok_or_else(|| format!("browser_comment{pointer} is missing"))
    };
    Ok(Request {
        device_id: text("/workspace/device_id")?,
        path: text("/workspace/path")?,
        area_id: text("/area_id")?,
        display_id: text("/display_id")?,
        on: payload["on"].as_bool().unwrap_or(false),
        update: payload["update"].as_bool().unwrap_or(false),
        pins: payload.get("pins").cloned().unwrap_or(json!([])),
        strings: payload.get("strings").cloned().unwrap_or(json!({})),
        add: match payload.get("add") {
            None => None,
            Some(add) => Some((
                add["pick"]
                    .as_u64()
                    .ok_or("browser_comment/add/pick is missing")?,
                add["note"]
                    .as_str()
                    .ok_or("browser_comment/add/note is missing")?
                    .to_owned(),
            )),
        },
    })
}

fn frame((device_id, path, display_id): &SessionKey, on: bool, reason: Option<&str>) -> String {
    framed(
        (device_id, path, display_id),
        json!({"on": on, "reason": reason}),
    )
}

/// A frame for the session's screen while it is on: a pick to write a note
/// on, or a pin the operator pressed.
fn framed((device_id, path, display_id): (&str, &str, &str), mut payload: Value) -> String {
    payload["workspace"] = json!({"device_id": device_id, "path": path});
    payload["display_id"] = json!(display_id);
    json!({"type": "browser_comment", "payload": payload}).to_string()
}

impl Request {
    fn key(&self) -> SessionKey {
        (
            self.device_id.clone(),
            self.path.clone(),
            self.display_id.clone(),
        )
    }
}

impl CommentSessions {
    /// A screen turns Comment on or off for a View, sends the page's pins
    /// again, or adds a note on the session's pick, which it answers with
    /// the comment for the screen's feedback queue.
    pub fn handle(
        self: &Arc<Self>,
        context: Context,
        connection: u64,
        event: &Value,
        frames: mpsc::Sender<String>,
    ) -> Result<Option<Added>, String> {
        let request = request(event)?;
        let key = request.key();
        let mut sessions = self.sessions.lock().map_err(|_| "comment sessions")?;
        if let Some((pick, note)) = request.add {
            return Ok(take_pick(&mut sessions, &key, connection, pick, note));
        }
        if let Some(session) = sessions.get_mut(&key) {
            if !request.on {
                if session.connection == connection {
                    session.off.take();
                }
                return Ok(None);
            }
            // Another screen's, or one still letting the page go.
            if session.connection != connection || session.off.is_none() {
                let _ = frames.try_send(frame(&key, false, Some("display_busy")));
                return Ok(None);
            }
            // A full queue means the session is still drawing the last
            // pins; the next snapshot sends them again.
            let _ = session.pins.try_send(Pins {
                pins: request.pins,
                strings: request.strings,
            });
            return Ok(None);
        }
        if !request.on || request.update {
            return Ok(None);
        }
        if sessions.len() >= MAX_SESSIONS {
            let _ = frames.try_send(frame(&key, false, Some("comment_limit")));
            return Ok(None);
        }
        let (pins, pins_rx) = mpsc::channel(8);
        let (off, off_rx) = oneshot::channel();
        let generation = self
            .generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        sessions.insert(
            key.clone(),
            Session {
                connection,
                generation,
                pins,
                off: Some(off),
                pick: None,
            },
        );
        drop(sessions);
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            let display_id = request.display_id.clone();
            let own = Own {
                registry: &registry,
                key: &key,
                generation,
            };
            let outcome = run(&context, request, own, pins_rx, off_rx, &frames).await;
            if let Err(failure) = &outcome {
                hide_core::diagnostic!(json!({
                    "component": "browser_comment",
                    "kind": "session.ended",
                    "display_id": display_id,
                    "reason": failure.reason,
                }));
            }
            if let Ok(mut sessions) = registry.sessions.lock()
                && sessions
                    .get(&key)
                    .is_some_and(|session| session.generation == generation)
            {
                sessions.remove(&key);
            }
            let reason = outcome.err().map(|failure| failure.reason);
            let _ = frames.send(frame(&key, false, reason.as_deref())).await;
        });
        Ok(None)
    }

    /// The screen went: its sessions end and their leases with them.
    pub fn release(&self, connection: u64) {
        if let Ok(mut sessions) = self.sessions.lock() {
            for session in sessions
                .values_mut()
                .filter(|session| session.connection == connection)
            {
                session.off.take();
            }
        }
    }
}

/// The screen's note on its session's pick: the comment it makes, once.
/// A pick another one replaced, or a session gone, adds nothing; the screen
/// drew the newer pick's note or heard the session end.
fn take_pick(
    sessions: &mut HashMap<SessionKey, Session>,
    key: &SessionKey,
    connection: u64,
    pick: u64,
    note: String,
) -> Option<Added> {
    let taken = sessions
        .get_mut(key)
        .filter(|session| session.connection == connection && session.off.is_some())
        .and_then(|session| session.pick.take_if(|held| held.id == pick));
    let Some(Pick { anchor, png, .. }) = taken else {
        hide_core::diagnostic!(json!({
            "component": "browser_comment",
            "kind": "pick.gone",
            "display_id": key.2,
            "pick": pick,
        }));
        return None;
    };
    let (device_id, path, display_id) = key;
    let event = json!({"schema_version": 2, "kind": "view_comment", "payload": {
        "workspace": {"device_id": device_id, "path": path},
        "action": "add", "display_id": display_id, "anchor": anchor, "note": note,
    }});
    Some(Added { event, png })
}

/// The running session's own entry: where its picks are kept.
struct Own<'a> {
    registry: &'a CommentSessions,
    key: &'a SessionKey,
    generation: u64,
}

impl Own<'_> {
    /// Holds `pick` for the screen's note, or lets the held one go.
    fn keep(&self, pick: Option<Pick>) {
        if let Ok(mut sessions) = self.registry.sessions.lock()
            && let Some(session) = sessions.get_mut(self.key)
            && session.generation == self.generation
        {
            session.pick = pick;
        }
    }
}

/// One session: lease, overlay, and the loop that reads the operator's
/// picks and the screen's pins until either side ends it.
async fn run(
    context: &Context,
    request: Request,
    own: Own<'_>,
    mut controls: mpsc::Receiver<Pins>,
    mut off: oneshot::Receiver<()>,
    frames: &mpsc::Sender<String>,
) -> Result<(), Failure> {
    let scope = json!({
        "workspace": format!("{}\0{}", request.device_id, request.path),
        "area_id": request.area_id,
        "display_id": request.display_id,
    });
    let control = Arc::clone(&context.control);
    let (_, ws) = tokio::task::spawn_blocking(move || control.comment_capability(&scope))
        .await
        .map_err(|_| Failure::new("browser_control_unavailable", None))?
        .map_err(|(reason, _)| Failure::new(reason, None))?;
    let socket = crate::browser_relay::connect(&ws)
        .await
        .map_err(|(reason, _)| Failure::new(reason, None))?;
    let mut page = Page::attach(Cdp::listening(socket, EVENTS), &request.display_id, true).await?;
    let top = page.top.clone();
    call(&mut page, "Runtime.enable", json!({})).await?;
    call(
        &mut page,
        "Runtime.addBinding",
        json!({"name": BINDING, "executionContextName": WORLD}),
    )
    .await?;
    let (mut world, mut frame_id) = inject(&mut page, &request.strings).await?;
    let mut shown = (request.pins.clone(), request.strings.clone());
    draw(&mut page, world, &shown).await;
    let key = request.key();
    let _ = frames.send(frame(&key, true, None)).await;
    let mut picks = 0_u64;
    let ended = loop {
        tokio::select! {
            // Taken or dropped alike: Comment off, or the screen went.
            _ = &mut off => break Ok(()),
            control = controls.recv() => match control {
                Some(Pins { pins, strings }) => {
                    if strings != shown.1 {
                        eval(&mut page, world, &format!("__hideComment.start({strings})")).await;
                    }
                    shown = (pins, strings);
                    draw(&mut page, world, &shown).await;
                }
                None => break Ok(()),
            },
            event = page.cdp.next_event() => {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => break Err(page.failure(error)),
                };
                match event.method.as_str() {
                    "Page.frameNavigated"
                        if event.session.as_deref() == Some(top.as_str())
                            && event.params["frame"].get("parentId").is_none() =>
                    {
                        // A new document: a place picked on the old one is
                        // gone with it, and so is its note in the window; the
                        // overlay goes in again, with the pins of the address
                        // it is at now.
                        own.keep(None);
                        let line = (key.0.as_str(), key.1.as_str(), key.2.as_str());
                        let _ = frames.send(framed(line, json!({"on": true, "reason": null, "dropped": true}))).await;
                        match inject(&mut page, &shown.1).await {
                            Ok((id, frame)) => {
                                (world, frame_id) = (id, frame);
                                draw(&mut page, world, &shown).await;
                            }
                            Err(failure) => break Err(failure),
                        }
                    }
                    // A route, a fragment or a step back within the same
                    // document leaves the place picked at the old address:
                    // the overlay stays, the pick and its note go.
                    "Page.navigatedWithinDocument"
                        if event.session.as_deref() == Some(top.as_str())
                            && event.params["frameId"].as_str() == Some(frame_id.as_str()) =>
                    {
                        own.keep(None);
                        let line = (key.0.as_str(), key.1.as_str(), key.2.as_str());
                        let _ = frames.send(framed(line, json!({"on": true, "reason": null, "dropped": true}))).await;
                    }
                    "Runtime.bindingCalled"
                        if event.params["name"] == BINDING
                            && event.params["executionContextId"].as_i64() == Some(world) =>
                    {
                        let message = event.params["payload"].as_str().unwrap_or("");
                        if message.len() > MAX_MESSAGE_BYTES {
                            continue;
                        }
                        let Ok(message) = serde_json::from_str::<Value>(message) else {
                            continue;
                        };
                        match message["type"].as_str() {
                            Some("off") => break Ok(()),
                            Some("pick") => {
                                let Some((anchor, at)) = picked(&message) else { continue };
                                picks += 1;
                                let png = capture(&mut page, world, &anchor["rect"]).await;
                                own.keep(Some(Pick { id: picks, anchor: anchor.clone(), png }));
                                let pick = json!({"id": picks, "anchor": anchor, "at": at});
                                let line = (key.0.as_str(), key.1.as_str(), key.2.as_str());
                                let _ = frames.send(framed(line, json!({"on": true, "reason": null, "pick": pick}))).await;
                            }
                            Some("open") => {
                                let (Some(id), Some(at)) = (message["id"].as_str(), fraction(&message["at"])) else { continue };
                                let line = (key.0.as_str(), key.1.as_str(), key.2.as_str());
                                let _ = frames.send(framed(line, json!({"on": true, "reason": null, "open": {"id": id, "at": at}}))).await;
                            }
                            _ => {}
                        }
                    }
                    "Inspector.detached" => break Err(Failure::new("display_missing", None)),
                    _ => {}
                }
            }
        }
    };
    eval(&mut page, world, "__hideComment.stop()").await;
    page.close().await;
    ended
}

async fn call(page: &mut Page, method: &str, params: Value) -> Result<Value, Failure> {
    let top = page.top.clone();
    page.cdp
        .call(method, params, Some(&top), CALL)
        .await
        .map_err(|error| page.failure(error))
}

/// A fresh isolated world on the page's document, with the overlay in it:
/// the world's context id and the page's main frame id.
async fn inject(page: &mut Page, strings: &Value) -> Result<(i64, String), Failure> {
    let tree = call(page, "Page.getFrameTree", json!({})).await?;
    let frame_id = tree["frameTree"]["frame"]["id"]
        .as_str()
        .ok_or_else(|| Failure::new("display_missing", None))?
        .to_owned();
    let world = call(
        page,
        "Page.createIsolatedWorld",
        json!({"frameId": frame_id, "worldName": WORLD}),
    )
    .await?;
    let id = world["executionContextId"]
        .as_i64()
        .ok_or_else(|| Failure::new("display_missing", None))?;
    let answer = call(
        page,
        "Runtime.evaluate",
        json!({"expression": format!("{COMMENT_JS};__hideComment.start({strings})"), "contextId": id, "returnByValue": true}),
    )
    .await?;
    if answer.get("exceptionDetails").is_some() {
        return Err(Failure::new("page_script_failed", None));
    }
    Ok((id, frame_id))
}

async fn eval(page: &mut Page, world: i64, expression: &str) {
    let _ = call(
        page,
        "Runtime.evaluate",
        json!({"expression": expression, "contextId": world, "returnByValue": true}),
    )
    .await;
}

async fn draw(page: &mut Page, world: i64, shown: &(Value, Value)) {
    eval(page, world, &format!("__hideComment.pins({})", shown.0)).await;
}

/// The overlay's pick: an element or an area of the page in document
/// pixels, and where it shows in the page's viewport. The core checks the
/// anchor again when the note adds it.
fn picked(message: &Value) -> Option<(Value, Value)> {
    let anchor = &message["anchor"];
    if !matches!(anchor["kind"].as_str(), Some("element" | "area")) {
        return None;
    }
    let rect = &anchor["rect"];
    let finite = |key: &str| rect[key].as_f64().is_some_and(f64::is_finite);
    if !["x", "y", "width", "height"].into_iter().all(finite) {
        return None;
    }
    Some((anchor.clone(), fraction(&message["at"])?))
}

/// A place as fractions of the page's viewport, which the screen scales to
/// the page's slot.
fn fraction(at: &Value) -> Option<Value> {
    let read = |key: &str| at[key].as_f64().filter(|value| value.is_finite());
    Some(json!({
        "left": read("left")?, "top": read("top")?,
        "width": read("width")?, "height": read("height")?,
    }))
}

/// The page clipped to `rect` with the overlay out of the picture, which
/// shows again once the shot is taken.
async fn capture(page: &mut Page, world: i64, rect: &Value) -> Option<Vec<u8>> {
    let hidden = call(
        page,
        "Runtime.evaluate",
        json!({"expression": "__hideComment.shot(true)", "contextId": world, "awaitPromise": true, "returnByValue": true}),
    )
    .await;
    let png = if hidden.is_ok() {
        shot(page, rect).await
    } else {
        None
    };
    let _ = call(
        page,
        "Runtime.evaluate",
        json!({"expression": "__hideComment.shot(false)", "contextId": world, "awaitPromise": true, "returnByValue": true}),
    )
    .await;
    png
}

/// The page clipped to `rect` (document CSS pixels), scaled down past
/// `MAX_SHOT_PIXELS`; none when the page gives none.
async fn shot(page: &mut Page, rect: &Value) -> Option<Vec<u8>> {
    let read = |key: &str| rect[key].as_f64().filter(|value| value.is_finite());
    let (x, y, width, height) = (read("x")?, read("y")?, read("width")?, read("height")?);
    if width < 1.0 || height < 1.0 {
        return None;
    }
    let scale = (MAX_SHOT_PIXELS / (width * height)).sqrt().min(1.0);
    let answer = call(
        page,
        "Page.captureScreenshot",
        json!({"format": "png", "clip": {"x": x, "y": y, "width": width, "height": height, "scale": scale}}),
    )
    .await;
    let data = match answer {
        Ok(answer) => answer["data"].as_str().map(str::to_owned)?,
        Err(failure) => {
            hide_core::diagnostic!(json!({
                "component": "browser_comment",
                "kind": "shot.capture_failed",
                "reason": failure.reason,
            }));
            return None;
        }
    };
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}
