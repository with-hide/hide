//! What each `hide browser` command does on an attached page.

use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};
use tokio::time::{Instant, sleep};

use super::cdp::CdpError;
use super::page::{Composite, DOM_JS, Frame, Lookup, Page, call, compare};
use super::{Command, DragMode, Failure, Output, Target, Verify, Wait};

const CONSOLE_LIMIT: usize = 50;
const NETWORK_LIMIT: usize = 100;
const EVAL_TIMEOUT: Duration = Duration::from_secs(9);
const WAIT_POLL: Duration = Duration::from_millis(150);
/// Three actions in a row that changed nothing usually mean a dead control,
/// an overlay taking the click, or a loop (chromux STALL_STREAK_THRESHOLD).
const STALL_STREAK: u64 = 3;
const DIALOG_NEXT: &str = "The page opened a dialog; the operator must answer it in hide, and the next command fails dialog_open until then";

/// The keys `press` sends (chromux KEY_DEFS).
const KEYS: &[(&str, u32, Option<&str>)] = &[
    ("Enter", 13, Some("\r")),
    ("Tab", 9, None),
    ("Escape", 27, None),
    ("Backspace", 8, None),
    ("Delete", 46, None),
    ("ArrowUp", 38, None),
    ("ArrowDown", 40, None),
    ("ArrowLeft", 37, None),
    ("ArrowRight", 39, None),
    ("Home", 36, None),
    ("End", 35, None),
    ("PageUp", 33, None),
    ("PageDown", 34, None),
];

/// A place an input event goes: the session of the frame that holds it, the
/// point in that frame's viewport, and the same point in the top viewport
/// when it is known (for the overlay).
struct Point {
    session: String,
    x: f64,
    y: f64,
    top: Option<(f64, f64)>,
    opaque_frame: bool,
    draggable: bool,
    label: Value,
}

fn selector(number: u64) -> String {
    format!("[data-ct-ref=\"{number}\"]")
}

fn ref_text(target: &Target) -> Value {
    match target {
        Target::Ref { tag: None, number } => json!(format!("@{number}")),
        Target::Ref {
            tag: Some(tag),
            number,
        } => json!(format!("@{tag}:{number}")),
        Target::Text(text) => json!({"text": text}),
        Target::Point { x, y, image } => {
            json!({"xy": [x, y], "space": if *image { "image" } else { "css" }})
        }
    }
}

pub async fn run(page: &mut Page, command: Command) -> Result<Output, Failure> {
    let display = page.display.clone();
    let next = json!(format!("hide browser snapshot {display} --diff"));
    match command {
        Command::Snapshot {
            interactive,
            diff,
            grep,
            ..
        } => {
            let filter = interactive.then_some("interactive");
            let mut composite = page.snapshot(filter, "auto").await?;
            composite.top =
                super::page::with_source_address(&composite.top, page.source.as_deref());
            let key = if interactive { "interactive" } else { "full" };
            let previous = page.swap_baseline(key, &mut composite).await?;
            let text = composite.text();
            let shown = if diff {
                page.render("diff", json!({"previous": previous, "current": text}))
                    .await?
            } else if let Some(pattern) = grep {
                page.render("grep", json!({"text": text, "pattern": pattern}))
                    .await?
            } else {
                json!(text)
            };
            Ok(Output::Text(shown.as_str().unwrap_or("").to_owned()))
        }
        Command::Click { target, verify, .. } => {
            page.require_visible().await?;
            let before = capture(page, &verify).await?;
            let point = locate(page, &target, true).await?;
            glide(page, &point).await;
            mouse(page, &point, "mouseMoved", "none", 0).await?;
            // Entering a cross-origin frame can take one renderer turn before
            // a press is routed into it (chromux).
            if point.opaque_frame {
                sleep(Duration::from_millis(100)).await;
            }
            mouse(page, &point, "mousePressed", "left", 1).await?;
            mouse(page, &point, "mouseReleased", "left", 0).await?;
            if let Some((x, y)) = point.top {
                page.overlay("click", json!({"x": x, "y": y})).await;
            }
            let mut answer = json!({"ok": true, "clicked": point.label});
            finish(page, &verify, before, acted(&target), &mut answer, next).await;
            Ok(Output::Json(answer))
        }
        Command::Fill {
            target,
            text,
            verify,
            ..
        } => {
            let (session, sel, frame) = element(page, &target).await?;
            let before = capture(page, &verify).await?;
            let filled = page
                .dom(&session, "fill", json!({"selector": sel, "text": text}))
                .await?;
            let mut answer = json!({"ok": true, "filled": ref_text(&target)});
            if filled["contenteditable"] == true {
                page.require_visible().await?;
                input(page, &session, "Input.insertText", json!({"text": text})).await?;
                if page.cdp.dialog.is_none() {
                    let observed = page
                        .dom(&session, "editableText", json!({"selector": sel}))
                        .await?;
                    // Editors keep a trailing newline and turn spaces they
                    // would collapse into no-break spaces.
                    let settled = |text: &str| text.replace('\u{a0}', " ").trim_end().to_owned();
                    if settled(observed["text"].as_str().unwrap_or("")) != settled(&text) {
                        return Err(Failure::new(
                            "fill_rejected",
                            Some(format!(
                                "expected {}, observed {}",
                                json!(text),
                                observed["text"]
                            )),
                        ));
                    }
                }
            } else if let Some(label) = filled.get("selectedLabel") {
                answer["selected"] = json!({"value": filled["value"], "label": label});
            }
            if let Ok(rect) = page.dom(&session, "box", json!({"selector": sel})).await {
                let offset = overlay_offset(page, frame.as_ref()).await;
                flash(page, &rect, offset).await;
            }
            finish(page, &verify, before, acted(&target), &mut answer, next).await;
            Ok(Output::Json(answer))
        }
        Command::Type { text, verify, .. } => {
            page.require_visible().await?;
            let before = capture(page, &verify).await?;
            let (session, offset) = focused(page).await?;
            input(page, &session, "Input.insertText", json!({"text": text})).await?;
            if let Ok(rect) = page.dom(&session, "focusRect", json!({})).await
                && !rect["rect"].is_null()
            {
                flash(page, &rect["rect"], offset).await;
            }
            let mut answer = json!({"ok": true, "typed": text});
            finish(page, &verify, before, None, &mut answer, next).await;
            Ok(Output::Json(answer))
        }
        Command::Press { key, verify, .. } => {
            let Some((name, code, text)) = KEYS.iter().find(|(name, ..)| *name == key) else {
                return Err(Failure::new(
                    "key_unsupported",
                    Some(
                        KEYS.iter()
                            .map(|(name, ..)| *name)
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                ));
            };
            page.require_visible().await?;
            let before = capture(page, &verify).await?;
            let (session, offset) = focused(page).await?;
            let mut down = json!({"type": "keyDown", "key": name, "code": name,
                "windowsVirtualKeyCode": code, "nativeVirtualKeyCode": code});
            if let Some(text) = text {
                down["text"] = json!(text);
            }
            input(page, &session, "Input.dispatchKeyEvent", down).await?;
            input(
                page,
                &session,
                "Input.dispatchKeyEvent",
                json!({"type": "keyUp", "key": name,
                "code": name, "windowsVirtualKeyCode": code, "nativeVirtualKeyCode": code}),
            )
            .await?;
            let rect = page.dom(&session, "focusRect", json!({})).await.ok();
            let rect = rect
                .and_then(|rect| (!rect["rect"].is_null()).then(|| shift(&rect["rect"], offset)));
            page.overlay("key", json!({"name": name, "rect": rect}))
                .await;
            let mut answer = json!({"ok": true, "pressed": name});
            finish(page, &verify, before, None, &mut answer, next).await;
            Ok(Output::Json(answer))
        }
        Command::Hover { target, verify, .. } => {
            page.require_visible().await?;
            let before = capture(page, &verify).await?;
            let point = locate(page, &target, true).await?;
            glide(page, &point).await;
            mouse(page, &point, "mouseMoved", "none", 0).await?;
            let mut answer = json!({"ok": true, "hovered": point.label});
            finish(page, &verify, before, None, &mut answer, next).await;
            Ok(Output::Json(answer))
        }
        Command::Drag {
            from,
            to,
            mode,
            verify,
            ..
        } => {
            page.require_visible().await?;
            let before = capture(page, &verify).await?;
            // Bring both ends into view first, then measure both again without
            // scrolling, so no point from before a scroll is dispatched.
            let both_refs = matches!(from, Target::Ref { .. }) && matches!(to, Target::Ref { .. });
            locate(page, &from, both_refs).await?;
            locate(page, &to, both_refs).await?;
            let start = locate(page, &from, false).await?;
            let end = locate(page, &to, false).await?;
            if start.session != end.session {
                return Err(Failure::new("drag_across_frames", None));
            }
            let html5 = match mode {
                DragMode::Auto => start.draggable,
                DragMode::Pointer => false,
                DragMode::Html5 => true,
            };
            glide(page, &start).await;
            const STEPS: u32 = 12;
            if let (Some(a), Some(b)) = (start.top, end.top) {
                page.overlay(
                    "drag",
                    json!({"x1": a.0, "y1": a.1, "x2": b.0, "y2": b.1, "ms": STEPS * 16 + 100}),
                )
                .await;
            }
            drag(page, &start, &end, STEPS, html5).await?;
            let mut answer = json!({"ok": true, "dragged": {"from": start.label, "to": end.label,
                "mode": if html5 { "html5" } else { "pointer" }}});
            finish(page, &verify, before, None, &mut answer, next).await;
            Ok(Output::Json(answer))
        }
        Command::Scroll {
            direction,
            target,
            verify,
            ..
        } => {
            let before = capture(page, &verify).await?;
            let mut answer = json!({"ok": true});
            match (direction, target) {
                (Some(up), _) => {
                    let probe = page.require_visible().await?;
                    let (width, height) = (
                        probe["width"].as_f64().unwrap_or(0.0),
                        probe["height"].as_f64().unwrap_or(0.0),
                    );
                    let top = page.top.clone();
                    let delta = (height * 0.85).round().max(1.0);
                    input(page, &top, "Input.dispatchMouseEvent", json!({"type": "mouseWheel",
                        "x": width / 2.0, "y": height / 2.0, "deltaX": 0, "deltaY": if up { -delta } else { delta }}))
                    .await?;
                    page.overlay(
                        "scroll",
                        json!({"direction": if up { "up" } else { "down" }}),
                    )
                    .await;
                    answer["scrolled"] = json!(if up { "up" } else { "down" });
                }
                (None, Some(target)) => {
                    let (session, sel, _) = element(page, &target).await?;
                    let moved = page
                        .dom(&session, "scrollIntoView", json!({"selector": sel}))
                        .await?;
                    let dy = moved["dy"].as_f64().unwrap_or(0.0);
                    if dy != 0.0 {
                        page.overlay(
                            "scroll",
                            json!({"direction": if dy < 0.0 { "up" } else { "down" }}),
                        )
                        .await;
                    }
                    answer["scrolled"] = ref_text(&target);
                    answer["moved"] = json!(dy);
                }
                (None, None) => unreachable!("parse requires a direction or a target"),
            }
            finish(page, &verify, before, None, &mut answer, next).await;
            Ok(Output::Json(answer))
        }
        Command::Wait { wait, timeout, .. } => wait_for(page, wait, timeout).await,
        Command::Screenshot { path, target, .. } => screenshot(page, &path, target.as_ref()).await,
        Command::Eval { expression, .. } => {
            let top = page.top.clone();
            let answer = page
                .cdp
                .call(
                    "Runtime.evaluate",
                    json!({"expression": expression, "returnByValue": true, "awaitPromise": true}),
                    Some(&top),
                    EVAL_TIMEOUT,
                )
                .await
                .map_err(|error| page.blocked(error))?;
            if let Some(details) = answer.get("exceptionDetails") {
                return Err(Failure::new(
                    "eval_error",
                    Some(super::page::first_line(details)),
                ));
            }
            let result = &answer["result"];
            let mut out =
                json!({"ok": true, "value": result.get("value").cloned().unwrap_or(Value::Null)});
            if result.get("value").is_none() {
                out["type"] = result["type"].clone();
            }
            Ok(Output::Json(out))
        }
        Command::Console { .. } => console(page).await,
        Command::Network { .. } => {
            let top = page.top.clone();
            let rows = page
                .dom(&top, "network", json!({"limit": NETWORK_LIMIT}))
                .await?;
            Ok(Output::Text(network_text(&rows)))
        }
    }
}

fn acted(target: &Target) -> Option<String> {
    match target {
        Target::Ref { .. } => ref_text(target).as_str().map(str::to_owned),
        _ => None,
    }
}

/// The frame session and selector an element ref names, and the
/// cross-origin frame that holds it, if any.
async fn element(
    page: &mut Page,
    target: &Target,
) -> Result<(String, String, Option<Frame>), Failure> {
    match target {
        Target::Ref { tag: None, number } => Ok((page.top.clone(), selector(*number), None)),
        Target::Ref {
            tag: Some(tag),
            number,
        } => {
            let frame = page.frame(tag).await?;
            Ok((frame.session.clone(), selector(*number), Some(frame)))
        }
        _ => unreachable!("fill, scroll and screenshot parse only refs"),
    }
}

/// Where a frame's viewport starts in the top viewport, for drawing; a frame
/// that cannot be placed is drawn as if at the origin rather than failing.
async fn overlay_offset(page: &mut Page, frame: Option<&Frame>) -> Option<(f64, f64)> {
    match frame {
        None => None,
        Some(frame) => page.frame_offset(frame).await.ok(),
    }
}

async fn locate(page: &mut Page, target: &Target, scroll: bool) -> Result<Point, Failure> {
    match target {
        Target::Ref { tag, number } => {
            let (session, frame) = match tag {
                None => (page.top.clone(), None::<Frame>),
                Some(tag) => {
                    let frame = page.frame(tag).await?;
                    (frame.session.clone(), Some(frame))
                }
            };
            let rect = page
                .dom(
                    &session,
                    "rect",
                    json!({"selector": selector(*number), "scroll": scroll}),
                )
                .await?;
            let (x, y) = (
                rect["centerX"].as_f64().unwrap_or(0.0),
                rect["centerY"].as_f64().unwrap_or(0.0),
            );
            let top = match &frame {
                None => Some((x, y)),
                Some(frame) => page
                    .frame_offset(frame)
                    .await
                    .ok()
                    .map(|(dx, dy)| (x + dx, y + dy)),
            };
            Ok(Point {
                session,
                x,
                y,
                top,
                opaque_frame: rect["opaqueFrame"] == true,
                draggable: rect["draggable"] == true,
                label: ref_text(target),
            })
        }
        Target::Text(label) => {
            let top = page.top.clone();
            let found = page.dom(&top, "textTarget", json!({"text": label})).await?;
            let number = found["ref"].as_u64().unwrap_or(0);
            let mut point =
                Box::pin(locate(page, &Target::Ref { tag: None, number }, scroll)).await?;
            point.label = json!({"text": label, "ref": format!("@{number}")});
            Ok(point)
        }
        Target::Point { x, y, image } => {
            let probe = page.probe().await?;
            let (x, y) = if *image {
                // A screenshot of the view has one pixel per device pixel of
                // the visible viewport, which `screenshot` reports as its css box.
                let vv = &probe["vv"];
                let pixel =
                    probe["dpr"].as_f64().unwrap_or(1.0) * vv["scale"].as_f64().unwrap_or(1.0);
                (
                    vv["left"].as_f64().unwrap_or(0.0) + x / pixel,
                    vv["top"].as_f64().unwrap_or(0.0) + y / pixel,
                )
            } else {
                (*x, *y)
            };
            let (width, height) = (
                probe["width"].as_f64().unwrap_or(0.0),
                probe["height"].as_f64().unwrap_or(0.0),
            );
            if x < 0.0 || y < 0.0 || x >= width || y >= height {
                return Err(Failure::new(
                    "outside_viewport",
                    Some(format!("css point {x},{y} outside {width}x{height}")),
                ));
            }
            Ok(Point {
                session: page.top.clone(),
                x,
                y,
                top: Some((x, y)),
                opaque_frame: false,
                draggable: false,
                label: ref_text(target),
            })
        }
    }
}

/// The frame session that holds the keyboard focus: the top document, or the
/// cross-origin frame its focused iframe leads to. Every frame is asked
/// together. A focus that cannot be placed because a frame did not answer is
/// not placed on the top document: the input would go to the wrong frame.
async fn focused(page: &mut Page) -> Result<(String, Option<(f64, f64)>), Failure> {
    let top = page.top.clone();
    let opaque = page.dom(&top, "activeOpaqueFrame", json!({})).await?;
    if opaque["opaque"] != true {
        return Ok((top, None));
    }
    let frames = page.frames().await?;
    let probe = call(DOM_JS, "hasFocus", &json!({}));
    let asks: Vec<(&str, String)> = frames
        .iter()
        .map(|frame| (frame.session.as_str(), probe.clone()))
        .collect();
    let answers = page.eval_all(&asks).await?;
    let mut unread = Vec::new();
    for (frame, answer) in frames.iter().zip(answers) {
        match answer {
            Ok(answer) if answer["focus"] == true => {
                let offset = page.frame_offset(frame).await.ok();
                return Ok((frame.session.clone(), offset));
            }
            Ok(_) => {}
            // A frame that navigated mid-read holds no focus now.
            Err(failure) if failure.page_side() => {}
            Err(failure) if failure.silent => unread.push(frame.session.clone()),
            Err(failure) => return Err(failure),
        }
    }
    page.record_unread(unread);
    let silent = page.unreadable();
    if silent.is_empty() {
        return Ok((top, None));
    }
    let origins: Vec<String> = silent.into_iter().map(|(_, origin)| origin).collect();
    Err(Failure {
        silent: true,
        detail: Some(format!(
            "the frame that holds the focus could not be told: no answer from {}",
            origins.join(", ")
        )),
        next_action: Some(format!(
            "No input was sent. Run hide browser snapshot {} to see which frames answer, then repeat the command",
            page.display
        )),
        ..Failure::new("page_unresponsive", None)
    })
}

/// Moves the operator's view of the cursor to the point and waits for the
/// glide, so the press lands where the cursor arrived.
async fn glide(page: &mut Page, point: &Point) {
    let Some((x, y)) = point.top else { return };
    if let Some(answer) = page.overlay("move", json!({"x": x, "y": y})).await {
        let ms = answer["ms"].as_u64().unwrap_or(0).min(500);
        sleep(Duration::from_millis(ms)).await;
    }
}

fn shift(rect: &Value, offset: Option<(f64, f64)>) -> Value {
    let (dx, dy) = offset.unwrap_or((0.0, 0.0));
    json!({"x": rect["x"].as_f64().unwrap_or(0.0) + dx, "y": rect["y"].as_f64().unwrap_or(0.0) + dy,
        "width": rect["width"], "height": rect["height"]})
}

async fn flash(page: &mut Page, rect: &Value, offset: Option<(f64, f64)>) {
    page.overlay("flash", shift(rect, offset)).await;
}

async fn mouse(
    page: &mut Page,
    point: &Point,
    kind: &str,
    button: &str,
    buttons: u32,
) -> Result<(), Failure> {
    let mut event =
        json!({"type": kind, "x": point.x, "y": point.y, "button": button, "pointerType": "mouse"});
    if kind != "mouseMoved" {
        event["buttons"] = json!(buttons);
        event["clickCount"] = json!(1);
    }
    let session = point.session.clone();
    input(page, &session, "Input.dispatchMouseEvent", event).await
}

/// Dispatches one input event. A dialog the event opened is reported by the
/// command, not treated as a failure.
async fn input(page: &mut Page, session: &str, method: &str, params: Value) -> Result<(), Failure> {
    match page
        .cdp
        .call_input(method, params, Some(session), page.budget())
        .await
    {
        Ok(_) => Ok(()),
        Err(error) => {
            let mut failure = unanswered(page, error).await;
            // The event was sent. A page held by a script takes it when the
            // script yields, so the failure does not say it was not delivered.
            if failure.reason == "page_unresponsive" {
                failure.next_action = Some(format!(
                    "The input was sent and the page did not answer in time, so it may already have taken effect; run hide browser snapshot {} --diff before repeating it",
                    page.display
                ));
            }
            Err(failure)
        }
    }
}

/// Input and pixels go unanswered on a page Chromium treats as hidden; any
/// other silence is the page's. A View the host hides, such as one of a
/// Workspace that is not in front, is not such a page: it answers at once.
async fn unanswered(page: &mut Page, error: CdpError) -> Failure {
    let held = || Failure {
        silent: true,
        ..Failure::new("page_unresponsive", None)
    };
    if !matches!(error, CdpError::Timeout) || page.cdp.dialog.is_some() {
        return page.blocked(error);
    }
    match page.probe().await {
        Ok(probe) if probe["visibility"] == "hidden" => Failure::new("display_hidden", None),
        // A probe that answers, or that only finds the document changed
        // under it, leaves the unanswered call as the cause.
        Ok(_) => held(),
        Err(failure) if failure.page_side() => held(),
        // Any other failure of the probe is the cause: the page held again
        // (silent), a dialog, a closed connection.
        Err(failure) => failure,
    }
}

async fn drag(
    page: &mut Page,
    start: &Point,
    end: &Point,
    steps: u32,
    html5: bool,
) -> Result<(), Failure> {
    let session = start.session.clone();
    if html5 {
        page.cdp
            .call(
                "Input.setInterceptDrags",
                json!({"enabled": true}),
                Some(&session),
                page.budget(),
            )
            .await
            .map_err(|error| page.blocked(error))?;
    }
    let result = drag_moves(page, start, end, steps, html5).await;
    // A page that answered nothing is sent nothing more, interception
    // switching included: it ends with the session.
    let held = matches!(&result, Err(failure) if failure.silent);
    if html5 && !held {
        let _ = page
            .cdp
            .call(
                "Input.setInterceptDrags",
                json!({"enabled": false}),
                Some(&session),
                page.budget(),
            )
            .await;
    }
    result
}

async fn drag_moves(
    page: &mut Page,
    start: &Point,
    end: &Point,
    steps: u32,
    html5: bool,
) -> Result<(), Failure> {
    let session = start.session.clone();
    mouse(page, start, "mouseMoved", "none", 0).await?;
    mouse(page, start, "mousePressed", "left", 1).await?;
    let mut pressed_at = (start.x, start.y);
    let moved = async {
        sleep(Duration::from_millis(100)).await;
        for index in 1..=steps {
            let progress = f64::from(index) / f64::from(steps);
            pressed_at = (
                start.x + (end.x - start.x) * progress,
                start.y + (end.y - start.y) * progress,
            );
            input(page, &session, "Input.dispatchMouseEvent", json!({"type": "mouseMoved",
                "x": pressed_at.0, "y": pressed_at.1, "button": "left", "buttons": 1, "pointerType": "mouse"}))
            .await?;
            sleep(Duration::from_millis(16)).await;
        }
        if html5 {
            let data = intercepted_drag(page, &session).await?;
            for kind in ["dragEnter", "dragOver", "drop"] {
                input(page, &session, "Input.dispatchDragEvent", json!({"type": kind, "x": end.x, "y": end.y, "data": data}))
                    .await?;
            }
        }
        Ok::<(), Failure>(())
    }
    .await;
    // The button is released on every path but one: a page that answered
    // nothing is held by a script, which would take the release when it
    // yields, long after the command reported its failure; Chromium releases
    // the button as the session ends. When a dialog holds the page the
    // release cannot be sent either.
    if matches!(&moved, Err(failure) if failure.silent) {
        return moved;
    }
    let release = input(page, &session, "Input.dispatchMouseEvent", json!({"type": "mouseReleased",
        "x": pressed_at.0, "y": pressed_at.1, "button": "left", "buttons": 0, "clickCount": 1, "pointerType": "mouse"}))
    .await;
    moved.and(release)
}

/// Chromium hands a native HTML5 drag to the client instead of starting it.
async fn intercepted_drag(page: &mut Page, session: &str) -> Result<Value, Failure> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let found = page
            .cdp
            .take_events(|event| event.method == "Input.dragIntercepted");
        if let Some(event) = found.into_iter().next() {
            let data = event.params["data"].clone();
            // The gateway refuses a drop that carries files, as it refuses an
            // upload; say so here rather than let the replay fail opaquely.
            if data["files"]
                .as_array()
                .is_some_and(|files| !files.is_empty())
            {
                return Err(Failure::new("drag_carries_files", None));
            }
            return Ok(data);
        }
        if Instant::now() >= deadline {
            return Err(Failure::new(
                "drag_not_started",
                Some("the source did not start a native drag; try --mode pointer".into()),
            ));
        }
        page.flush(session).await?;
        sleep(Duration::from_millis(50)).await;
    }
}

/// The page before an action, read the way `changed` compares it: every
/// frame, with clickable detection capped in document order so scrolling
/// alone never reads as a change.
async fn capture(page: &mut Page, verify: &Verify) -> Result<Option<Composite>, Failure> {
    match verify {
        Verify::Off => Ok(None),
        Verify::After(_) => Ok(Some(page.snapshot(None, "stable").await?)),
    }
}

/// Adds what the action changed (or a dialog it opened) to its answer.
async fn finish(
    page: &mut Page,
    verify: &Verify,
    before: Option<Composite>,
    acted: Option<String>,
    answer: &mut Value,
    next: Value,
) {
    if let Some(dialog) = page.cdp.dialog.clone() {
        answer["dialog"] = dialog;
        answer["next_action"] = json!(DIALOG_NEXT);
        return;
    }
    answer["next"] = next;
    let (Verify::After(wait), Some(before)) = (verify, before) else {
        return;
    };
    match changed(page, *wait, &before, acted).await {
        Ok(text) => answer["changed"] = json!(text),
        Err(_) if page.cdp.dialog.is_some() => {
            if let Some(object) = answer.as_object_mut() {
                object.remove("next");
            }
            answer["dialog"] = page.cdp.dialog.clone().unwrap_or(Value::Null);
            answer["next_action"] = json!(DIALOG_NEXT);
        }
        Err(failure) => {
            answer["changed"] = json!(format!(
                "# changed: the page could not be read after the action ({}); run snapshot --diff before repeating it",
                failure.reason
            ));
        }
    }
}

/// What changed between the page before the action and now, compared on the
/// frames both reads could read. A frame either read could not read is named,
/// never counted as a change or as no change.
async fn changed(
    page: &mut Page,
    wait: Duration,
    before: &Composite,
    acted: Option<String>,
) -> Result<String, Failure> {
    sleep(wait).await;
    let mut unread = std::collections::BTreeSet::new();
    let mut current = page.snapshot(None, "stable").await?;
    let mut seen = compare(before, &current);
    unread.extend(seen.unread.iter().cloned());
    let first = page
        .render(
            "changes",
            json!({"previous": seen.before, "current": seen.after, "ref": acted}),
        )
        .await?;
    let none = first["count"].as_u64() == Some(0);
    if none || first["selfEchoOnly"] == true {
        sleep(Duration::from_millis(700)).await;
        current = page.snapshot(None, "stable").await?;
        seen = compare(before, &current);
        unread.extend(seen.unread.iter().cloned());
    }
    if none {
        let again = page
            .render(
                "changes",
                json!({"previous": seen.before, "current": seen.after}),
            )
            .await?;
        if again["count"].as_u64() == Some(0) {
            // Slow pages land their update seconds later; say so in time
            // terms, since a bare "no change" pushes a retry (a double submit).
            sleep(Duration::from_millis(1200)).await;
            current = page.snapshot(None, "stable").await?;
            seen = compare(before, &current);
            unread.extend(seen.unread.iter().cloned());
            let late = page
                .render(
                    "changes",
                    json!({"previous": seen.before, "current": seen.after}),
                )
                .await?;
            if late["count"].as_u64() == Some(0) {
                // A frame one of the reads could not read may hold the change,
                // so this is neither "changed nothing" nor a stall.
                if !unread.is_empty() {
                    return Ok(format!(
                        "# changed: no visible change within ~2s in what could be read, but {} frame(s) did not answer ({}), so a change inside them could not be seen; the action was dispatched; look with hide browser snapshot {} --diff once they answer",
                        unread.len(),
                        unread.iter().cloned().collect::<Vec<_>>().join(", "),
                        page.display
                    ));
                }
                let top = page.top.clone();
                let streak = page.dom(&top, "streak", json!({"changed": false})).await?;
                let streak = streak["streak"].as_u64().unwrap_or(1);
                let mut message = "# changed: no visible change within ~2s - the action was dispatched, but the page may still be updating or the result may be in another display or a dialog; confirm with hide browser wait or snapshot --diff BEFORE repeating the action".to_owned();
                if streak >= STALL_STREAK {
                    message.push_str(&format!("\n# stalled: {streak} actions in a row changed nothing - you are likely stuck (a dead control, an overlay taking the click, or a loop). Do not repeat it; try another element, dismiss the overlay, wait for the state you expect, or hand off to the operator."));
                }
                return Ok(message);
            }
        }
    }
    let top = page.top.clone();
    page.dom(&top, "streak", json!({"changed": true})).await?;
    let text = page
        .render(
            "changed",
            json!({"previous": seen.before, "current": seen.after}),
        )
        .await?;
    let mut text = text.as_str().unwrap_or("").to_owned();
    if !unread.is_empty() {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&format!(
            "# unread: {} frame(s) did not answer in one of the two reads ({}); changes inside them are not shown\n",
            unread.len(),
            unread.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(text)
}

/// One poll of a wait: whether the condition holds now, how many sessions were
/// asked, and whether the top document went unanswered because the wait ran out
/// of time, which leaves the condition unknown, not false.
struct Poll {
    found: bool,
    polled: usize,
    page_silent: bool,
}

async fn poll(page: &mut Page, wait: &Wait, deadline: Option<Instant>) -> Result<Poll, Failure> {
    let mut polled = 1;
    let mut page_silent = false;
    let found = match wait {
        Wait::Text(text) => {
            // The top document and every frame are asked together; one
            // that does not answer is asked again at the next poll, until
            // the command has given up on it.
            let mut sessions = vec![page.top.clone()];
            sessions.extend(page.frames().await?.into_iter().map(|frame| frame.session));
            // One command per session that is asked: a frame the command has
            // given up on costs the gateway nothing.
            polled = sessions
                .iter()
                .filter(|session| !page.given_up(session))
                .count();
            let probe = call(DOM_JS, "waitText", &json!({"text": text}));
            let asks: Vec<(&str, String)> = sessions
                .iter()
                .map(|session| (session.as_str(), probe.clone()))
                .collect();
            let answers = page.eval_all(&asks).await?;
            let mut unread = Vec::new();
            let mut found = false;
            for (session, answer) in sessions.iter().zip(answers) {
                match answer {
                    Ok(answer) if answer["found"] == true => {
                        found = true;
                        break;
                    }
                    Ok(_) => {}
                    // A frame that navigated mid-poll is read again next time.
                    Err(failure) if failure.page_side() => {}
                    // The top document is silent only for a wait that has run
                    // out of time: the wait's own deadline cut the read short.
                    Err(failure)
                        if failure.silent
                            && (*session != page.top
                                || deadline.is_some_and(|deadline| Instant::now() >= deadline)) =>
                    {
                        page_silent |= *session == page.top;
                        unread.push(session.clone());
                    }
                    Err(failure) => return Err(failure),
                }
            }
            page.record_unread(unread);
            found
        }
        Wait::Selector { selector, gone } => {
            // None: this poll could not tell, because the element's frame
            // navigated or did not answer; the next poll asks again. A
            // frame that stays silent never satisfies the wait, `--gone`
            // included.
            page.record_unread(Vec::new());
            let visible = match super::parse_ref(selector) {
                Some(Target::Ref {
                    tag: Some(tag),
                    number,
                }) => match page.lookup(&tag).await? {
                    Lookup::Found(frame) => match page
                        .dom(
                            &frame.session,
                            "waitSelector",
                            json!({"selector": self::selector(number)}),
                        )
                        .await
                    {
                        Ok(answer) => Some(answer["found"] == true),
                        Err(failure) if failure.silent => {
                            page.record_unread(vec![frame.session]);
                            None
                        }
                        Err(failure) if failure.page_side() => None,
                        Err(failure) => return Err(failure),
                    },
                    // The frame's document is gone, so is its element.
                    Lookup::Stale => Some(false),
                    Lookup::Unknown => None,
                },
                Some(Target::Ref { tag: None, number }) => {
                    let top = page.top.clone();
                    Some(
                        page.dom(
                            &top,
                            "waitSelector",
                            json!({"selector": self::selector(number)}),
                        )
                        .await?["found"]
                            == true,
                    )
                }
                _ => {
                    let top = page.top.clone();
                    Some(
                        page.dom(&top, "waitSelector", json!({"selector": selector}))
                            .await?["found"]
                            == true,
                    )
                }
            };
            visible.is_some_and(|visible| visible != *gone)
        }
    };
    Ok(Poll {
        found,
        polled,
        page_silent,
    })
}

async fn wait_for(page: &mut Page, wait: Wait, timeout: Duration) -> Result<Output, Failure> {
    let started = Instant::now();
    let deadline = started + timeout;
    // The first poll is a whole read, each call as long as a step, so a short
    // timeout still finds what is already there. Every later read waits no
    // longer than the wait has left, so the last poll ends at the deadline,
    // not a step after it.
    let mut first = true;
    loop {
        // Silence that ends a poll at the wait's deadline is the wait running
        // out of time; silence that lasted a whole step is not.
        let cut = (!first).then_some(deadline);
        let Poll {
            found,
            polled,
            page_silent,
        } = match poll(page, &wait, cut).await {
            Ok(outcome) => outcome,
            // The poll ran out the wait's own time before something answered:
            // that is the wait timing out, not the page failing.
            Err(failure) if failure.silent && cut.is_some_and(|cut| Instant::now() >= cut) => {
                Poll {
                    found: false,
                    polled: 1,
                    page_silent: true,
                }
            }
            Err(failure) => return Err(failure),
        };
        page.end_at(deadline);
        first = false;
        let waited = started.elapsed().as_millis() as u64;
        if found {
            let answer = match &wait {
                Wait::Text(text) => json!({"ok": true, "found_text": text, "waited_ms": waited}),
                Wait::Selector {
                    selector,
                    gone: false,
                } => {
                    json!({"ok": true, "found_selector": selector, "waited_ms": waited})
                }
                Wait::Selector {
                    selector,
                    gone: true,
                } => {
                    json!({"ok": true, "gone_selector": selector, "waited_ms": waited})
                }
            };
            return Ok(Output::Json(answer));
        }
        // The gateway admits 600 commands a minute; one poll costs one per
        // frame read.
        let interval = WAIT_POLL * polled as u32;
        let left = deadline.saturating_duration_since(Instant::now());
        // A poll that would start at the deadline has no time to read
        // anything, so the wait ends there instead.
        if !left.is_zero() {
            sleep(interval.min(left)).await;
        }
        if Instant::now() < deadline {
            continue;
        }
        let what = match &wait {
            Wait::Text(text) => format!("text {} not found", json!(text)),
            Wait::Selector {
                selector,
                gone: false,
            } => format!("{selector} not visible"),
            Wait::Selector {
                selector,
                gone: true,
            } => format!("{selector} still visible"),
        };
        let mut detail = format!("{what} after {} ms", timeout.as_millis());
        let mut silent: Vec<String> = page
            .unreadable()
            .into_iter()
            .map(|(_, origin)| origin)
            .collect();
        silent.sort();
        silent.dedup();
        if !silent.is_empty() {
            detail.push_str(&format!(
                "; a frame that did not answer was not read ({})",
                silent.join(", ")
            ));
        }
        let unlisted: Vec<String> = page
            .unlisted()
            .into_iter()
            .map(|(_, origin)| origin)
            .collect();
        if !unlisted.is_empty() {
            detail.push_str(&format!(
                "; the frames inside {} were not listed",
                unlisted.join(", ")
            ));
        }
        // Not read is not absent: the last poll was cut short at the deadline
        // before the page answered, so the condition may hold.
        if page_silent {
            detail.push_str("; the last poll ended before the page answered, so it may hold");
        }
        return Err(Failure::new("timeout", Some(detail)));
    }
}

async fn capture_png(page: &mut Page, clip: Option<Value>) -> Result<Vec<u8>, Failure> {
    let top = page.top.clone();
    let mut params = json!({"format": "png"});
    if let Some(clip) = clip {
        params["clip"] = clip;
    }
    let shot = page
        .cdp
        .call("Page.captureScreenshot", params, Some(&top), page.budget())
        .await;
    let shot = match shot {
        Ok(shot) => shot,
        Err(error) => return Err(unanswered(page, error).await),
    };
    base64::engine::general_purpose::STANDARD
        .decode(shot["data"].as_str().unwrap_or(""))
        .map_err(|_| Failure::new("screenshot_failed", None))
}

pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[1..4] != b"PNG" {
        return None;
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let height = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((width, height))
}

async fn screenshot(
    page: &mut Page,
    path: &str,
    target: Option<&Target>,
) -> Result<Output, Failure> {
    let file = std::env::current_dir()
        .map(|cwd| cwd.join(path))
        .unwrap_or_else(|_| PathBuf::from(path));
    page.require_visible().await?;
    let mut crop = None;
    if let Some(target) = target {
        let (session, sel, frame) = element(page, target).await?;
        page.dom(&session, "scrollIntoView", json!({"selector": sel}))
            .await?;
        // A crop placed at the wrong offset would be a wrong picture.
        let offset = match &frame {
            None => None,
            Some(frame) => Some(page.frame_offset(frame).await?),
        };
        let rect = shift(
            &page.dom(&session, "box", json!({"selector": sel})).await?,
            offset,
        );
        crop = Some(rect);
    }
    // Measured after any scroll the ref needed.
    let probe = page.probe().await?;
    let vv = &probe["vv"];
    let view = (
        vv["left"].as_f64().unwrap_or(0.0),
        vv["top"].as_f64().unwrap_or(0.0),
        vv["width"].as_f64().unwrap_or(0.0),
        vv["height"].as_f64().unwrap_or(0.0),
    );
    let css = match &crop {
        None => view,
        Some(rect) => {
            let (x, y) = (
                rect["x"].as_f64().unwrap_or(0.0),
                rect["y"].as_f64().unwrap_or(0.0),
            );
            let (w, h) = (
                rect["width"].as_f64().unwrap_or(0.0),
                rect["height"].as_f64().unwrap_or(0.0),
            );
            let left = x.max(view.0);
            let top = y.max(view.1);
            let right = (x + w).min(view.0 + view.2);
            let bottom = (y + h).min(view.1 + view.3);
            if right <= left || bottom <= top {
                return Err(Failure::new("target_outside_viewport", None));
            }
            (left, top, right - left, bottom - top)
        }
    };
    let clip = crop.as_ref().map(|_| {
        json!({"x": probe["scroll"]["x"].as_f64().unwrap_or(0.0) + css.0,
            "y": probe["scroll"]["y"].as_f64().unwrap_or(0.0) + css.1,
            "width": css.2, "height": css.3, "scale": 1})
    });
    let bytes = capture_png(page, clip).await?;
    let (width, height) =
        png_size(&bytes).ok_or_else(|| Failure::new("screenshot_failed", None))?;
    std::fs::write(&file, &bytes).map_err(|error| {
        Failure::new(
            "screenshot_unwritable",
            Some(format!("{}: {error}", file.display())),
        )
    })?;
    let mut answer = json!({
        "ok": true,
        "path": file.display().to_string(),
        "image": {"width": width, "height": height},
        "css": {"x": css.0, "y": css.1, "width": css.2, "height": css.3},
        "css_to_image": {"scale_x": f64::from(width) / css.2, "scale_y": f64::from(height) / css.3,
            "offset_css_x": css.0, "offset_css_y": css.1},
    });
    if let Some(target) = target {
        answer["ref"] = ref_text(target);
    }
    Ok(Output::Json(answer))
}

/// Chrome replays a document's stored console messages to a session that
/// enables the Runtime domain, so a stateless command still reads them.
async fn console(page: &mut Page) -> Result<Output, Failure> {
    let top = page.top.clone();
    page.cdp
        .call("Runtime.enable", json!({}), Some(&top), page.budget())
        .await
        .map_err(|error| page.blocked(error))?;
    page.flush(&top).await?;
    let events = page.cdp.take_events(|event| {
        event.session.as_deref() == Some(top.as_str())
            && matches!(
                event.method.as_str(),
                "Runtime.consoleAPICalled" | "Runtime.exceptionThrown"
            )
    });
    let lines: Vec<String> = events
        .iter()
        .rev()
        .map(|event| console_line(&event.params, &event.method))
        .collect();
    Ok(Output::Text(console_text(&lines)))
}

fn console_line(params: &Value, method: &str) -> String {
    let (level, text, frame) = if method == "Runtime.exceptionThrown" {
        let details = &params["exceptionDetails"];
        (
            "exception".to_owned(),
            super::page::first_line(details),
            details["stackTrace"]["callFrames"][0].clone(),
        )
    } else {
        let text = params["args"]
            .as_array()
            .map(|args| {
                args.iter()
                    .map(|arg| match &arg["value"] {
                        Value::String(text) => text.clone(),
                        Value::Null => arg["description"]
                            .as_str()
                            .or_else(|| arg["unserializableValue"].as_str())
                            .or_else(|| arg["type"].as_str())
                            .unwrap_or("")
                            .to_owned(),
                        value => value.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        (
            params["type"].as_str().unwrap_or("log").to_owned(),
            text,
            params["stackTrace"]["callFrames"][0].clone(),
        )
    };
    let text: String = text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(500)
        .collect();
    let location = frame["url"]
        .as_str()
        .filter(|url| !url.is_empty())
        .map(|url| {
            format!(
                "  ({url}:{}:{})",
                frame["lineNumber"].as_u64().unwrap_or(0) + 1,
                frame["columnNumber"].as_u64().unwrap_or(0) + 1
            )
        })
        .unwrap_or_default();
    format!("[{level}] {text}{location}")
}

fn console_text(lines: &[String]) -> String {
    if lines.is_empty() {
        return "# console: no messages in the current document\n".to_owned();
    }
    let shown = lines.len().min(CONSOLE_LIMIT);
    let mut out = format!(
        "# console: {} message{} in the current document, newest first{}\n\n",
        lines.len(),
        if lines.len() == 1 { "" } else { "s" },
        if shown < lines.len() {
            format!(", showing {shown}")
        } else {
            String::new()
        }
    );
    for line in &lines[..shown] {
        out.push_str(line);
        out.push('\n');
    }
    out
}

fn network_text(rows: &Value) -> String {
    let total = rows["total"].as_u64().unwrap_or(0);
    let list = rows["rows"].as_array().cloned().unwrap_or_default();
    let mut out = format!(
        "# network: {total} resource{} loaded by the current document (resource timing), newest first{}\n# request and response headers and bodies are not available\n",
        if total == 1 { "" } else { "s" },
        if (list.len() as u64) < total {
            format!(", showing {}", list.len())
        } else {
            String::new()
        }
    );
    if list.is_empty() {
        return out;
    }
    out.push('\n');
    for row in list {
        let status = match row["status"].as_u64() {
            Some(0) | None => "-".to_owned(),
            Some(status) => status.to_string(),
        };
        out.push_str(&format!(
            "{status} {}ms {}B {} {}\n",
            row["ms"].as_u64().unwrap_or(0),
            row["bytes"].as_u64().unwrap_or(0),
            row["type"].as_str().unwrap_or(""),
            row["url"].as_str().unwrap_or("")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_png_header_gives_its_pixel_size() {
        let mut bytes = vec![
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
        ];
        bytes.extend_from_slice(&2048u32.to_be_bytes());
        bytes.extend_from_slice(&1280u32.to_be_bytes());
        assert_eq!(png_size(&bytes), Some((2048, 1280)));
        assert_eq!(png_size(b"GIF89a................."), None);
        assert_eq!(png_size(&bytes[..20]), None);
    }

    #[test]
    fn console_lines_name_level_text_and_place_newest_first() {
        let log = json!({"type": "warning", "args": [{"type": "string", "value": "slow\n  load"}, {"type": "number", "value": 3}],
            "stackTrace": {"callFrames": [{"url": "http://127.0.0.1:3000/app.js", "lineNumber": 9, "columnNumber": 4}]}});
        assert_eq!(
            console_line(&log, "Runtime.consoleAPICalled"),
            "[warning] slow load 3  (http://127.0.0.1:3000/app.js:10:5)"
        );
        let thrown = json!({"exceptionDetails": {"text": "Uncaught", "exception": {"description": "TypeError: x is undefined\n    at f"}}});
        assert_eq!(
            console_line(&thrown, "Runtime.exceptionThrown"),
            "[exception] TypeError: x is undefined"
        );
        assert_eq!(
            console_text(&[]),
            "# console: no messages in the current document\n"
        );
        let many: Vec<String> = (0..60).map(|n| format!("[log] {n}")).collect();
        let text = console_text(&many);
        assert!(text.starts_with(
            "# console: 60 messages in the current document, newest first, showing 50\n"
        ));
        assert_eq!(text.lines().count(), 52);
    }

    #[test]
    fn network_lines_never_claim_headers_or_bodies() {
        let rows = json!({"total": 2, "rows": [
            {"url": "http://127.0.0.1:3000/app.js", "type": "script", "status": 200, "ms": 12, "bytes": 3400},
            {"url": "https://cdn.example/x.png", "type": "img", "status": 0, "ms": 40, "bytes": 0}]});
        let text = network_text(&rows);
        assert!(text.contains("# request and response headers and bodies are not available\n"));
        assert!(text.contains("200 12ms 3400B script http://127.0.0.1:3000/app.js\n"));
        assert!(text.contains("- 40ms 0B img https://cdn.example/x.png\n"));
    }

    use super::super::fake_gateway::{
        Frames, page, page_with_frames, page_within, recording, reply,
    };
    use std::sync::{Arc, Mutex};

    type EventLog = Arc<Mutex<Vec<String>>>;

    fn point() -> Point {
        Point {
            session: "top".into(),
            x: 1.0,
            y: 1.0,
            top: None,
            opaque_frame: false,
            draggable: false,
            label: Value::Null,
        }
    }

    /// A page that answers every mouse event but the `silent_from`th, and
    /// every probe; the kinds of event it received are kept.
    fn mouse_page(
        silent_from: Option<usize>,
    ) -> (impl FnMut(&Value) -> Vec<Value> + Send + 'static, EventLog) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&seen);
        let script = move |request: &Value| match request["method"].as_str().unwrap() {
            "Input.setInterceptDrags" => {
                let on = request["params"]["enabled"] == true;
                log.lock().unwrap().push(format!("intercept:{on}"));
                vec![reply(request, json!({}))]
            }
            "Input.dispatchMouseEvent" => {
                let mut seen = log.lock().unwrap();
                seen.push(request["params"]["type"].as_str().unwrap().to_owned());
                if silent_from.is_some_and(|n| seen.len() >= n) {
                    Vec::new()
                } else {
                    vec![reply(request, json!({}))]
                }
            }
            _ => vec![reply(
                request,
                json!({"result": {"value": {"visibility": "visible"}}}),
            )],
        };
        (script, seen)
    }

    #[tokio::test]
    async fn a_drag_the_page_stops_answering_sends_no_release_after_it() {
        // The page answers the move and the press, then holds the next move.
        let (script, seen) = mouse_page(Some(3));
        let mut page = page(script).await;
        let failure = drag_moves(&mut page, &point(), &point(), 3, false)
            .await
            .unwrap_err();
        assert!(failure.silent, "{failure:?}");
        assert_eq!(
            *seen.lock().unwrap(),
            ["mouseMoved", "mousePressed", "mouseMoved"]
        );
    }

    #[tokio::test]
    async fn a_drag_the_page_answers_ends_with_the_release() {
        let (script, seen) = mouse_page(None);
        let mut page = page(script).await;
        drag_moves(&mut page, &point(), &point(), 2, false)
            .await
            .unwrap();
        assert_eq!(seen.lock().unwrap().last().unwrap(), "mouseReleased");
    }

    #[tokio::test]
    async fn a_native_drag_that_carries_files_is_refused_by_name() {
        for (files, refused) in [(json!(["/outside/private.txt"]), true), (json!([]), false)] {
            let mut page = page(move |request| {
                vec![
                    json!({"method": "Input.dragIntercepted", "sessionId": "top",
                        "params": {"data": {"items": [], "files": files}}}),
                    reply(request, json!({"result": {"value": 0}})),
                ]
            })
            .await;
            let found = intercepted_drag(&mut page, "top").await;
            match (refused, found) {
                (true, Err(failure)) => assert_eq!(failure.reason, "drag_carries_files"),
                (false, Ok(data)) => assert_eq!(data["items"], json!([])),
                (_, other) => panic!("{other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn a_wait_asks_a_slow_frame_again_and_names_a_silent_one_when_it_times_out() {
        // A frame that misses one poll is read in the next.
        let slow = Frames {
            healthy: 1,
            slow_once_on_wait: true,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(slow, None)).await;
        let found = wait_for(&mut page, Wait::Text("Done".into()), Duration::from_secs(5)).await;
        assert!(matches!(found, Ok(Output::Json(_))), "{:?}", found.err());

        let silent = Frames {
            hung: 1,
            healthy: 1,
            stops_on_read: true,
            ..Frames::default()
        };
        let mut page = super::super::fake_gateway::page(page_with_frames(silent, None)).await;
        let failure = wait_for(
            &mut page,
            Wait::Text("never shown".into()),
            Duration::from_millis(100),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(failure.reason, "timeout");
        let detail = failure.detail.unwrap();
        assert!(
            detail.contains(
                "a frame that did not answer was not read (http://hung1.test, http://ok.test:9)"
            ),
            "{detail}"
        );
    }

    #[tokio::test]
    async fn a_wait_for_a_frame_element_never_reads_a_silent_frame_as_gone() {
        let silent = Frames {
            healthy: 1,
            stops_on_read: true,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(silent, None)).await;
        let failure = wait_for(
            &mut page,
            Wait::Selector {
                selector: "@k7q2:3".into(),
                gone: true,
            },
            Duration::from_millis(100),
        )
        .await
        .err()
        .unwrap();
        assert_eq!(failure.reason, "timeout");
        assert!(failure.detail.unwrap().contains("http://ok.test:9"));
    }

    #[tokio::test]
    async fn an_input_the_relay_ends_keeps_its_hint_but_is_not_silence() {
        let mut page = page(|request| match request["method"].as_str().unwrap() {
            "Input.dispatchMouseEvent" => Vec::new(),
            _ => vec![super::super::fake_gateway::close(
                crate::browser_relay::CLOSE_IDLE,
            )],
        })
        .await;
        let failure = input(&mut page, "top", "Input.dispatchMouseEvent", json!({}))
            .await
            .unwrap_err();
        assert_eq!(failure.reason, "page_unresponsive");
        assert!(!failure.silent);
        assert!(
            failure
                .next_action
                .unwrap()
                .contains("may already have taken effect")
        );
    }

    #[tokio::test]
    async fn an_html5_drag_the_page_stops_answering_leaves_interception_to_the_session() {
        let (script, seen) = mouse_page(Some(3));
        let mut page = page(script).await;
        let failure = drag(&mut page, &point(), &point(), 3, true)
            .await
            .unwrap_err();
        assert!(failure.silent, "{failure:?}");
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first().unwrap(), "intercept:true");
        assert!(!seen.contains(&"intercept:false".to_owned()), "{seen:?}");
    }

    #[tokio::test]
    async fn a_wait_asks_every_frame_in_one_round_per_poll() {
        // The frames answer only once all three have been asked, and only the
        // first holds the text: a poll that asks them one at a time hears
        // nothing from the first and never finds it.
        let frames = Frames {
            healthy: 3,
            together: true,
            frame_wait_found: true,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(frames, None)).await;
        let found = wait_for(&mut page, Wait::Text("Done".into()), Duration::from_secs(5)).await;
        assert!(matches!(found, Ok(Output::Json(_))), "{:?}", found.err());
    }

    #[tokio::test]
    async fn a_wait_asks_again_a_frame_that_was_slow_to_attach() {
        // The first frame leaves its first `Page.enable` unanswered, so it has
        // no document at the first poll; the next poll enables and reads it.
        let frames = Frames {
            healthy: 1,
            slow_enable_once: true,
            frame_wait_found: true,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(frames, None)).await;
        let found = wait_for(&mut page, Wait::Text("Done".into()), Duration::from_secs(5)).await;
        assert!(matches!(found, Ok(Output::Json(_))), "{:?}", found.err());
    }

    #[tokio::test]
    async fn a_wait_for_a_frame_element_to_go_is_not_met_by_a_frame_that_has_not_answered_yet() {
        // At the first poll the frame that carries the ref has no document, so
        // the ref names nothing yet; that is not the element being gone. The
        // element is there once the frame answers, so the wait times out.
        let frames = Frames {
            healthy: 1,
            slow_enable_once: true,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(frames, None)).await;
        let outcome = wait_for(
            &mut page,
            Wait::Selector {
                selector: "@k7q2:3".into(),
                gone: true,
            },
            Duration::from_millis(200),
        )
        .await;
        assert_eq!(outcome.err().unwrap().reason, "timeout");
    }

    #[tokio::test]
    async fn a_wait_for_a_missing_frame_ref_to_go_is_met() {
        let mut page = page(page_with_frames(Frames::default(), None)).await;
        let outcome = wait_for(
            &mut page,
            Wait::Selector {
                selector: "@k7q2:3".into(),
                gone: true,
            },
            Duration::from_secs(5),
        )
        .await;
        assert!(matches!(outcome, Ok(Output::Json(_))));
    }

    #[tokio::test]
    async fn a_change_check_that_could_not_read_a_frame_says_so_and_counts_no_stall() {
        let silent = Frames {
            hung: 1,
            ..Frames::default()
        };
        let mut script = page_with_frames(silent, None);
        let mut page = page(move |request| match request["method"].as_str().unwrap() {
            "Page.getFrameTree" => vec![reply(
                request,
                json!({"frameTree": {"frame": {"id": "main"}}}),
            )],
            "Page.createIsolatedWorld" => vec![reply(request, json!({"executionContextId": 1}))],
            "Runtime.evaluate" if request["params"]["contextId"].is_number() => {
                vec![reply(request, json!({"result": {"value": {"count": 0}}}))]
            }
            _ => script(request),
        })
        .await;
        let before = page.snapshot(None, "stable").await.unwrap();
        let message = changed(&mut page, Duration::ZERO, &before, None)
            .await
            .unwrap();
        assert!(
            message.contains("1 frame(s) did not answer (http://hung1.test)"),
            "{message}"
        );
        assert!(!message.contains("stalled"), "{message}");
    }

    /// Answers the change check's page-side comparison the way the page
    /// would: no change when the two texts it is given are equal.
    fn comparing(
        mut script: impl FnMut(&Value) -> Vec<Value> + Send + 'static,
    ) -> impl FnMut(&Value) -> Vec<Value> + Send + 'static {
        move |request| match request["method"].as_str().unwrap() {
            "Page.getFrameTree" => vec![reply(
                request,
                json!({"frameTree": {"frame": {"id": "main"}}}),
            )],
            "Page.createIsolatedWorld" => vec![reply(request, json!({"executionContextId": 1}))],
            "Runtime.evaluate" if request["params"]["contextId"].is_number() => {
                let expression = request["params"]["expression"].as_str().unwrap();
                let args = expression
                    .split_once(",{")
                    .map(|(_, args)| format!("{{{}", args.trim_end_matches(')')))
                    .unwrap_or_default();
                let args: Value = serde_json::from_str(&args).unwrap_or(Value::Null);
                let same = args["previous"] == args["current"];
                let value = if expression.contains("\"changed\"") {
                    json!("# changed: something\n")
                } else {
                    json!({"count": u64::from(!same), "selfEchoOnly": false})
                };
                vec![reply(request, json!({"result": {"value": value}}))]
            }
            "Runtime.evaluate"
                if request["params"]["expression"]
                    .as_str()
                    .unwrap()
                    .contains("\"streak\"") =>
            {
                vec![reply(request, json!({"result": {"value": {"streak": 1}}}))]
            }
            _ => script(request),
        }
    }

    #[tokio::test]
    async fn a_frame_that_answered_before_the_action_and_not_after_is_no_change() {
        // Before the action the child frame was read; after it the frame
        // holds a script and gives no answer. Its section is not "removed
        // content": the page shows no change, and the frame is named.
        let silent = Frames {
            healthy: 1,
            stops_on_read: true,
            ..Frames::default()
        };
        let mut page = page(comparing(page_with_frames(silent, None))).await;
        let before = Composite {
            top: "# T\n# http://a/\n\n@1 button \"A\"\n".into(),
            sections: vec![(
                "ok1".into(),
                "# OOPIF k7q2 origin=http://ok.test:9\n@k7q2:1 link \"B\"".into(),
            )],
            silent: Vec::new(),
            unread: Vec::new(),
            unlisted: Vec::new(),
            lineage: Vec::new(),
        };
        let message = changed(&mut page, Duration::ZERO, &before, None)
            .await
            .unwrap();
        assert!(
            message.contains("1 frame(s) did not answer (http://ok.test:9)"),
            "{message}"
        );
        assert!(!message.contains("stalled"), "{message}");
    }

    #[tokio::test]
    async fn a_change_the_page_made_names_the_frames_that_could_not_be_compared() {
        let silent = Frames {
            healthy: 1,
            stops_on_read: true,
            ..Frames::default()
        };
        let mut page = page(comparing(page_with_frames(silent, None))).await;
        let before = Composite {
            top: "# T\n# http://a/\n\n@1 button \"Different\"\n".into(),
            sections: Vec::new(),
            silent: Vec::new(),
            unread: Vec::new(),
            unlisted: Vec::new(),
            lineage: Vec::new(),
        };
        let message = changed(&mut page, Duration::ZERO, &before, None)
            .await
            .unwrap();
        assert!(message.starts_with("# changed: something"), "{message}");
        assert!(
            message.contains(
                "# unread: 1 frame(s) did not answer in one of the two reads (http://ok.test:9)"
            ),
            "{message}"
        );
    }

    #[tokio::test]
    async fn a_focus_that_a_frame_may_hold_is_never_placed_on_the_top_document() {
        // The focused element is in a cross-origin frame; one frame did not
        // answer and the frame that did holds no focus: it could be the
        // silent one, so no input goes to the top document.
        let frames = Frames {
            hung: 1,
            healthy: 1,
            opaque_focus: true,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(frames, None)).await;
        let failure = focused(&mut page).await.unwrap_err();
        assert_eq!(failure.reason, "page_unresponsive");
        assert!(failure.silent);
        assert!(failure.detail.unwrap().contains("http://hung1.test"));

        let all_silent = Frames {
            healthy: 1,
            stops_on_read: true,
            opaque_focus: true,
            ..Frames::default()
        };
        let mut page = page_with(all_silent).await;
        let failure = focused(&mut page).await.unwrap_err();
        assert!(failure.silent, "{failure:?}");
        assert!(failure.detail.unwrap().contains("http://ok.test:9"));
    }

    /// A snapshot of a device's page, from the core's view list to the line
    /// the CLI prints: each link of the chain hands on the next one's key, so
    /// a renamed field or a dropped step shows the routed address.
    #[tokio::test]
    async fn a_snapshot_of_a_device_page_prints_the_address_the_core_keeps() {
        let views = vec![herdr_core::workspace_control::View {
            area_id: "area-a".into(),
            view_id: "browser-1".into(),
            kind: "browser",
            target: "http://localhost:8765/docs/index.html".into(),
            selected: true,
            active_area: true,
            page: None,
        }];
        // The daemon's relay answer, as the CLI reads it.
        let facts = crate::server::relay_facts(&views, "browser-1").unwrap();
        let ready = crate::server::relay_ready("browser-1", &facts);
        let (selected, source) = crate::workspace_cli::relay_facts(&ready);
        assert!(selected);

        // The page itself reports the loopback route it loaded from.
        let mut script = page_with_frames(Frames::default(), None);
        let mut page = page(move |request| {
            script(request)
                .into_iter()
                .map(|mut reply| {
                    if let Some(value) = reply["result"]["result"]["value"].as_str() {
                        reply["result"]["result"]["value"] = json!(
                            value
                                .replace("# http://a/", "# http://127.0.0.1:64423/docs/index.html")
                        );
                    }
                    reply
                })
                .collect()
        })
        .await;
        page.source = source;
        let Ok(Output::Text(text)) = run(
            &mut page,
            Command::Snapshot {
                display: "browser-1".into(),
                interactive: false,
                diff: false,
                grep: None,
            },
        )
        .await
        else {
            panic!("the snapshot did not print");
        };
        assert!(
            text.contains("# http://localhost:8765/docs/index.html\n"),
            "{text}"
        );
        assert!(!text.contains("64423"), "{text}");
    }

    async fn page_with(frames: Frames) -> Page {
        page(page_with_frames(frames, None)).await
    }

    #[tokio::test]
    async fn a_focus_a_frame_reports_is_found_beside_one_that_did_not_answer() {
        let frames = Frames {
            hung: 1,
            healthy: 1,
            opaque_focus: true,
            focus_in: 1,
            ..Frames::default()
        };
        let mut page = page_with(frames).await;
        let (session, _) = focused(&mut page).await.unwrap();
        assert_eq!(session, "ok1");
    }

    #[tokio::test]
    async fn a_focus_every_frame_denies_stays_on_the_top_document() {
        let frames = Frames {
            healthy: 2,
            opaque_focus: true,
            ..Frames::default()
        };
        let mut page = page_with(frames).await;
        let (session, offset) = focused(&mut page).await.unwrap();
        assert_eq!((session.as_str(), offset), ("top", None));
    }

    #[tokio::test]
    async fn a_frame_whose_own_frames_could_not_be_looked_for_may_hold_the_ref() {
        // The frame answers its tag but not the look for its own frames, so a
        // ref no frame answers to may be a child of it: not known to be gone.
        let frames = Frames {
            healthy: 1,
            scan_silent: true,
            ..Frames::default()
        };
        let mut page = page_with(frames).await;
        let failure = page.frame("zz9z").await.unwrap_err();
        assert_eq!(failure.reason, "page_unresponsive");
        assert!(failure.silent);
        let failure = element(
            &mut page,
            &Target::Ref {
                tag: Some("zz9z".into()),
                number: 3,
            },
        )
        .await
        .err()
        .unwrap();
        assert_eq!(failure.reason, "page_unresponsive");

        let mut page = page_with(Frames {
            healthy: 1,
            scan_silent: true,
            ..Frames::default()
        })
        .await;
        let outcome = wait_for(
            &mut page,
            Wait::Selector {
                selector: "@zz9z:3".into(),
                gone: true,
            },
            Duration::from_millis(100),
        )
        .await;
        assert_eq!(outcome.err().unwrap().reason, "timeout");
    }

    #[tokio::test]
    async fn a_ref_no_frame_answers_to_is_stale_when_every_frame_was_looked_through() {
        let mut page = page_with(Frames {
            healthy: 1,
            ..Frames::default()
        })
        .await;
        assert_eq!(page.frame("zz9z").await.unwrap_err().reason, "ref_stale");
    }

    #[tokio::test]
    async fn a_ref_no_frame_carries_beside_a_silent_frame_answers_with_the_fresh_snapshot_guidance()
    {
        let hung = Frames {
            hung: 1,
            ..Frames::default()
        };
        let mut page = page_with(hung).await;
        let failure = run(
            &mut page,
            Command::Click {
                display: "browser-1".into(),
                target: Target::Ref {
                    tag: Some("old1".into()),
                    number: 3,
                },
                verify: Verify::Off,
            },
        )
        .await
        .err()
        .unwrap();
        assert_eq!(failure.reason, "page_unresponsive");
        assert!(
            failure
                .next_action
                .unwrap()
                .contains("take a fresh snapshot (hide browser snapshot browser-1)")
        );
    }

    /// A top document that answers the first `waitText` and no other.
    fn answers_once(
        mut script: impl FnMut(&Value) -> Vec<Value> + Send + 'static,
    ) -> impl FnMut(&Value) -> Vec<Value> + Send + 'static {
        let mut asked = 0;
        move |request| {
            let waiting = request["params"]["expression"]
                .as_str()
                .is_some_and(|expression| expression.contains("\"waitText\""));
            if waiting {
                asked += 1;
                if asked > 1 {
                    return Vec::new();
                }
            }
            script(request)
        }
    }

    #[tokio::test]
    async fn a_polls_reads_after_the_first_end_with_the_wait_and_say_the_text_is_not_known_absent()
    {
        // The step is a minute: a second poll that waited it out on the top
        // document, silent now, would be cut off by the hang guard, not by the
        // wait.
        let mut page = page_within(
            answers_once(page_with_frames(Frames::default(), None)),
            Duration::from_secs(60),
        )
        .await;
        let failure = tokio::time::timeout(
            Duration::from_secs(20),
            wait_for(
                &mut page,
                Wait::Text("never shown".into()),
                Duration::from_secs(1),
            ),
        )
        .await
        .expect("the wait ended by its own deadline")
        .err()
        .unwrap();
        assert_eq!(failure.reason, "timeout");
        assert!(
            failure
                .detail
                .unwrap()
                .contains("ended before the page answered")
        );
    }

    #[tokio::test]
    async fn a_wait_shorter_than_a_read_still_gets_one_whole_read() {
        // The page shows the text and a frame holds the page's attention for
        // several round trips: a read cut to the wait's 1 ms would give up
        // before the first answer.
        let shown = Frames {
            healthy: 2,
            wait_found: true,
            ..Frames::default()
        };
        let mut page = page_with(shown).await;
        let outcome = wait_for(
            &mut page,
            Wait::Text("Ready".into()),
            Duration::from_millis(1),
        )
        .await;
        assert!(
            matches!(outcome, Ok(Output::Json(_))),
            "{:?}",
            outcome.err()
        );
    }

    #[tokio::test]
    async fn a_poll_counts_only_the_frames_it_asks() {
        // Three frames answer nothing to a read. After two polls the command
        // has given them up, and a poll asks the top document alone.
        let silent = Frames {
            healthy: 3,
            stops_on_read: true,
            ..Frames::default()
        };
        let mut page = page_with(silent).await;
        let wait = Wait::Text("never shown".into());
        let mut asked = Vec::new();
        for _ in 0..3 {
            asked.push(poll(&mut page, &wait, None).await.unwrap().polled);
        }
        assert_eq!(asked, [4, 4, 1]);
    }

    #[tokio::test]
    async fn a_frame_that_does_not_answer_the_look_for_its_own_frames_is_asked_within_the_budget() {
        let frames = Frames {
            healthy: 1,
            scan_silent: true,
            ..Frames::default()
        };
        let (script, seen) = recording(page_with_frames(frames, None));
        let mut page = page(script).await;
        for _ in 0..4 {
            page.frames().await.unwrap();
        }
        assert_eq!(
            seen.lock().unwrap().sent("Target.setAutoAttach", "ok1"),
            super::super::page::MISS_LIMIT as usize
        );
    }

    #[tokio::test]
    async fn a_click_with_verify_asks_a_frame_that_never_answers_within_the_commands_budget() {
        let hung = Frames {
            hung: 1,
            ..Frames::default()
        };
        let (script, seen) = recording(acting(page_with_frames(hung, None)));
        let mut page = page(script).await;
        let Ok(Output::Json(answer)) = run(
            &mut page,
            Command::Click {
                display: "browser-1".into(),
                target: Target::Ref {
                    tag: None,
                    number: 1,
                },
                verify: Verify::After(Duration::ZERO),
            },
        )
        .await
        else {
            panic!("the click failed");
        };
        // The capture and the three change reads each look at the page; the
        // frame is asked on two of them, not on every one.
        assert_eq!(
            seen.lock().unwrap().sent("Page.enable", "f1"),
            super::super::page::MISS_LIMIT as usize
        );
        // And it is still named, not counted as a change.
        let changed = answer["changed"].as_str().unwrap();
        assert!(
            changed.contains("1 frame(s) did not answer (http://hung1.test)"),
            "{changed}"
        );
    }

    #[tokio::test]
    async fn a_drag_ends_with_its_release_whatever_the_gateway_holds_of_frame_reads() {
        // 24 frames never answer, and the gateway holds what was asked of
        // them. The drag's events, the release last, are the page's own and
        // are never refused for it, so the button is not left held down.
        let hung = Frames {
            hung: 24,
            ..Frames::default()
        };
        let (script, seen) = recording(acting(page_with_frames(hung, None)));
        let mut page = page(script).await;
        page.frames().await.unwrap();
        let sessions: Vec<String> = (1..=24).map(|n| format!("f{n}")).collect();
        let asks: Vec<(&str, String)> = sessions
            .iter()
            .map(|session| (session.as_str(), "1".to_owned()))
            .collect();
        page.eval_all(&asks).await.unwrap();
        drag_moves(&mut page, &point(), &point(), 2, false)
            .await
            .unwrap();
        let seen = seen.lock().unwrap();
        // Move, press, two moves, release.
        assert_eq!(seen.sent("Input.dispatchMouseEvent", "top"), 5);
        assert!(seen.most_pending <= 32, "{}", seen.most_pending);
    }

    /// The page answers the input events of an action.
    fn acting(
        mut script: impl FnMut(&Value) -> Vec<Value> + Send + 'static,
    ) -> impl FnMut(&Value) -> Vec<Value> + Send + 'static {
        let mut script = comparing(move |request: &Value| script(request));
        move |request| match request["method"].as_str().unwrap() {
            "Input.dispatchMouseEvent" => vec![reply(request, json!({}))],
            _ => script(request),
        }
    }
}
