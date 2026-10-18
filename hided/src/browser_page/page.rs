//! One display for the life of one command: the relay connection, the
//! display's page session, the cross-origin frames auto-attached under it,
//! and the page-side assets evaluated in each.

use std::time::Duration;

use serde_json::{Value, json};
use tokio::time::Instant;

use super::Failure;
use super::cdp::{Cdp, CdpError, Event};

pub const SNAPSHOT_JS: &str = include_str!("../../assets/browser/snapshot.js");
pub const DOM_JS: &str = include_str!("../../assets/browser/dom.js");
pub const RENDER_JS: &str = include_str!("../../assets/browser/render.js");
pub const OVERLAY_JS: &str = include_str!("../../assets/browser/overlay.js");

/// Below the gateway's ten-second command deadline, which would otherwise
/// release the whole debugger lease mid-command.
pub const STEP: Duration = Duration::from_secs(8);
const OVERLAY: Duration = Duration::from_secs(1);
/// Nested cross-origin frames a snapshot follows; the gateway admits 64
/// sessions per client.
const MAX_FRAMES: usize = 24;
/// How many rounds in a row may go unanswered by the same frame, for each
/// kind of thing it is asked (its `Page.enable`, tag and reads; its look for
/// its own frames), before that kind is left alone for the rest of the
/// command: the round that found it silent, and one more in case it was only
/// slow. Only an answer of the same kind starts the count again: the budget is
/// for silence, not for a frame's history, and a frame that answers a read
/// says nothing of whether it answers a look for its own frames. Every read of a page
/// asks the frames again, so without this a frame that never answers costs a
/// whole step on each read of a command.
pub const MISS_LIMIT: u8 = 2;

/// A cross-origin frame: its own session, the session of the document that
/// embeds it, and the tag its document carries for `@<tag>:N` refs.
#[derive(Debug, Clone)]
pub struct Frame {
    pub session: String,
    pub target: String,
    pub parent: String,
    pub tag: String,
    pub origin: String,
}

/// A cross-origin frame an attach event named, and what is known of it. Not
/// answering is something one read observed, not something the frame is: a
/// frame with no document yet is asked again by the next `frames()`, and one
/// that missed a read is asked again by the next, until it has missed
/// `MISS_LIMIT` rounds of this command.
struct Known {
    session: String,
    target: String,
    parent: String,
    /// Where the frame was attached, as its origin would read.
    attached_origin: String,
    /// Whether `Page.enable` has been answered on its session.
    enabled: bool,
    /// Whether the last attempt to enable it or read its tag went unanswered.
    silent: bool,
    /// Its document's tag and origin, once read.
    document: Option<(String, String)>,
    /// How many rounds in a row it left unanswered, asked to enable, tag or
    /// read.
    misses: u8,
    /// How many rounds in a row its look for its own frames went unanswered.
    scan_misses: u8,
    /// Whether its own frames could not be listed, because that look went
    /// unanswered or was not sent. The frame itself has been read; what is
    /// inside it is not known.
    unlisted: bool,
}

fn note(origin: &str) -> String {
    format!(
        "# OOPIF unresponsive origin={origin} - no answer in time; a script that never yields, or a dialog the operator has not answered"
    )
}

fn unlisted_note(origin: &str) -> String {
    format!(
        "# OOPIF unlisted origin={origin} - no answer in time to the look for the frames inside it; they are not shown, and not known to be absent"
    )
}

/// What `Page::lookup` found for a ref's frame tag.
pub enum Lookup {
    Found(Frame),
    /// No frame carries the tag, and every frame that could was read.
    Stale,
    /// No frame answered to the tag, but a frame that did not answer may be
    /// the one: absence is not known.
    Unknown,
}

pub struct Page {
    pub cdp: Cdp,
    pub display: String,
    pub top: String,
    /// Whether the display is its area's selected View, the only one shown.
    selected: bool,
    /// The address the core keeps for the display. A device's page loads from
    /// a loopback route on the core's machine, so the page itself reports
    /// that address; this is the one the operator and the caller know.
    pub source: Option<String>,
    /// How long a call waits for an answer: `STEP`, shorter in a test that
    /// lets one lapse.
    pub step: Duration,
    /// When the command's own time runs out, if it has less than the whole
    /// command: no call waits past it.
    deadline: Option<Instant>,
    /// The frames attach events have named, in attach order.
    known: Vec<Known>,
    /// Sessions whose own frames have been looked for.
    scanned: Vec<String>,
    /// Sessions of tagged frames that went unanswered in the last multi-frame
    /// read, for the notes and the timeout detail of a command.
    unread: Vec<String>,
}

/// What a snapshot of every frame read: the top document's text, one
/// section per cross-origin frame with the session it came from, each
/// already in its displayed form, and a note for each frame that did not
/// answer.
pub struct Composite {
    pub top: String,
    pub sections: Vec<(String, String)>,
    pub silent: Vec<String>,
    /// The session and origin of each frame this read could not read.
    pub unread: Vec<(String, String)>,
    /// The session and origin of each frame this read read but could not list
    /// the frames inside of.
    pub unlisted: Vec<(String, String)>,
    /// The session of the frame each section's frame is embedded in.
    pub lineage: Vec<(String, String)>,
}

/// Two reads of the page set side by side: what they can be compared on.
pub struct Comparison {
    pub before: String,
    pub after: String,
    /// The origins of the frames that could not be read, or looked into, in
    /// one read or the other.
    pub unread: Vec<String>,
}

/// Compares like with like. A frame one of the reads could not read is left
/// out of both, with its note, and so are the frames inside a frame that one
/// of the reads could not read or could not list the frames of; so a frame that
/// answered in one read and not the other is never a content change. A frame
/// that was read in both is compared, even when what is inside it is not known.
pub fn compare(before: &Composite, after: &Composite) -> Comparison {
    let parent_of = |session: &str| {
        before
            .lineage
            .iter()
            .chain(&after.lineage)
            .find(|(frame, _)| frame == session)
            .map(|(_, parent)| parent.as_str())
    };
    let unread_in = |session: &str| {
        before
            .unread
            .iter()
            .chain(&after.unread)
            .any(|(unread, _)| unread == session)
    };
    let unlisted_in = |session: &str| {
        before
            .unlisted
            .iter()
            .chain(&after.unlisted)
            .any(|(unlisted, _)| unlisted == session)
    };
    let excluded = |session: &str| {
        if unread_in(session) {
            return true;
        }
        let mut current = parent_of(session);
        // Frames nest at most `MAX_FRAMES` deep.
        for _ in 0..=MAX_FRAMES {
            let Some(frame) = current else { return false };
            if unread_in(frame) || unlisted_in(frame) {
                return true;
            }
            current = parent_of(frame);
        }
        false
    };
    let text = |composite: &Composite| {
        let sections: Vec<String> = composite
            .sections
            .iter()
            .filter(|(session, _)| !excluded(session))
            .map(|(_, text)| text.clone())
            .collect();
        compose(&composite.top, &sections)
    };
    let mut unread: Vec<String> = before
        .unread
        .iter()
        .chain(&after.unread)
        .chain(&before.unlisted)
        .chain(&after.unlisted)
        .map(|(_, origin)| origin.clone())
        .collect();
    unread.sort();
    unread.dedup();
    Comparison {
        before: text(before),
        after: text(after),
        unread,
    }
}

impl Composite {
    /// The top document and the notes of the frames that did not answer, the
    /// text its `--diff` baseline keeps: the notes read the same in
    /// consecutive snapshots, so a diff names a frame only when it goes
    /// silent or answers again.
    pub fn top_block(&self) -> String {
        if self.silent.is_empty() {
            return self.top.clone();
        }
        format!("{}\n\n{}\n", self.top.trim_end(), self.silent.join("\n"))
    }

    pub fn text(&self) -> String {
        let sections: Vec<String> = self.sections.iter().map(|(_, text)| text.clone()).collect();
        compose(&self.top_block(), &sections)
    }
}

/// The origin of an address a page attached a frame at, spelled as the
/// frame's own `location.origin` would be: scheme and host in lower case, a
/// default port dropped. `opaque` for an address with none (`about:blank`,
/// `data:`) or one that is not `http(s)`.
fn origin_of(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return "opaque".to_owned();
    };
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "http" => "80",
        "https" => "443",
        _ => return "opaque".to_owned(),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let (name, port) = match host.rsplit_once(':') {
        Some((name, port)) if !host.ends_with(']') => (name, Some(port)),
        _ => (host.as_str(), None),
    };
    if name.is_empty() {
        return "opaque".to_owned();
    }
    match port {
        Some(port) if port != default_port && !port.is_empty() => {
            format!("{scheme}://{name}:{port}")
        }
        _ => format!("{scheme}://{name}"),
    }
}

/// The displayed text of a top document and its frames' sections.
fn compose(top: &str, sections: &[String]) -> String {
    if sections.is_empty() {
        return top.to_owned();
    }
    format!("{}\n\n{}\n", top.trim_end(), sections.join("\n\n"))
}

/// The four characters `dom.js` draws a frame tag from.
fn valid_tag(tag: &str) -> bool {
    tag.len() == 4
        && tag
            .bytes()
            .all(|b| b"abcdefghijkmnpqrstuvwxyz23456789".contains(&b))
}

pub fn call(asset: &str, op: &str, args: &Value) -> String {
    format!("({asset})({},{args})", json!(op))
}

impl Page {
    #[cfg(test)]
    pub fn for_test(mut cdp: Cdp, step: Duration) -> Self {
        cdp.set_top("top");
        Self {
            cdp,
            display: "browser-1".into(),
            top: "top".into(),
            selected: true,
            source: None,
            step,
            deadline: None,
            known: Vec::new(),
            scanned: Vec::new(),
            unread: Vec::new(),
        }
    }

    pub async fn attach(cdp: Cdp, display: &str, selected: bool) -> Result<Self, Failure> {
        Self::attach_within(cdp, display, selected, STEP).await
    }

    async fn attach_within(
        cdp: Cdp,
        display: &str,
        selected: bool,
        step: Duration,
    ) -> Result<Self, Failure> {
        let mut page = Self {
            cdp,
            display: display.to_owned(),
            top: String::new(),
            selected,
            source: None,
            step,
            deadline: None,
            known: Vec::new(),
            scanned: Vec::new(),
            unread: Vec::new(),
        };
        let targets = page
            .cdp
            .call("Target.getTargets", json!({}), None, page.step)
            .await
            .map_err(|error| page.failure(error))?;
        let target = targets["targetInfos"]
            .as_array()
            .and_then(|targets| targets.iter().find(|target| target["type"] == "page"))
            .and_then(|target| target["targetId"].as_str())
            .ok_or_else(|| Failure::new("display_missing", None))?
            .to_owned();
        let attached = page
            .cdp
            .call(
                "Target.attachToTarget",
                json!({"targetId": target, "flatten": true}),
                None,
                page.step,
            )
            .await;
        page.top = match attached {
            Ok(answer) => answer["sessionId"]
                .as_str()
                .ok_or_else(|| Failure::new("display_missing", None))?
                .to_owned(),
            Err(CdpError::Protocol(message)) if message.contains("already has a debugger") => {
                return Err(Failure::new("display_busy", None));
            }
            Err(error) => return Err(page.failure(error)),
        };
        let top = page.top.clone();
        page.cdp.set_top(&top);
        page.enable(&top).await?;
        Ok(page)
    }

    /// How long a call may wait now: a step, or less when the command's own
    /// time runs out sooner.
    pub fn budget(&self) -> Duration {
        match self.deadline {
            Some(deadline) => self
                .step
                .min(deadline.saturating_duration_since(Instant::now())),
            None => self.step,
        }
    }

    /// Ends every later wait of this command at `deadline`.
    pub fn end_at(&mut self, deadline: Instant) {
        self.deadline = Some(deadline);
    }

    /// Whether this command has stopped asking the frame, because it has
    /// left too many rounds unanswered.
    pub fn given_up(&self, session: &str) -> bool {
        self.known
            .iter()
            .any(|frame| frame.session == session && frame.misses >= MISS_LIMIT)
    }

    /// Records that a round of this command got no answer from the frame.
    fn missed(&mut self, session: &str) {
        if let Some(frame) = self.known.iter_mut().find(|frame| frame.session == session) {
            frame.misses = frame.misses.saturating_add(1);
        }
    }

    /// Records that the frame answered, in whatever way: its silence so far
    /// no longer counts against it.
    fn answered(&mut self, session: &str) {
        if let Some(frame) = self.known.iter_mut().find(|frame| frame.session == session) {
            frame.misses = 0;
        }
    }

    /// What a read of a frame came to, and what it does to the frame's budget.
    /// A call that was never sent says nothing of the frame: it stays unread
    /// this round and is asked in a later one.
    fn read_outcome(
        &mut self,
        session: &str,
        answer: Result<Value, CdpError>,
    ) -> Result<Value, Failure> {
        let sent = !matches!(answer, Err(CdpError::NotSent));
        let outcome = self.value_of(answer);
        if sent {
            match &outcome {
                Err(failure) if failure.silent => self.missed(session),
                _ => self.answered(session),
            }
        }
        outcome
    }

    /// Turns on the dialog events of the top document. An open JavaScript
    /// dialog holds the renderer, so nothing on a new session answers and the
    /// dialog event is not sent again; a top document silent for a whole step
    /// is taken to be held by one. A cross-origin frame has its own renderer,
    /// so its silence is only its own (`frames`).
    async fn enable(&mut self, session: &str) -> Result<(), Failure> {
        match self
            .cdp
            .call("Page.enable", json!({}), Some(session), self.budget())
            .await
        {
            Ok(_) => Ok(()),
            Err(CdpError::Timeout) => Err(Failure::new("dialog_open", None)),
            Err(error) => Err(self.failure(error)),
        }
    }

    /// The reason a transport or protocol error stands for.
    pub fn failure(&self, error: CdpError) -> Failure {
        match error {
            CdpError::Timeout | CdpError::NotSent => Failure {
                silent: true,
                ..Failure::new("page_unresponsive", None)
            },
            CdpError::Protocol(message) => Failure::new("cdp_error", Some(message)),
            CdpError::Closed { code, reason } => match code {
                crate::browser_relay::CLOSE_MESSAGE_LIMIT => {
                    Failure::new("browser_relay_message_limit", None)
                }
                // The relay saw no answer for a minute, or outlived any command.
                crate::browser_relay::CLOSE_IDLE => {
                    Failure::new("page_unresponsive", (!reason.is_empty()).then_some(reason))
                }
                crate::browser_relay::CLOSE_GATEWAY_LOST => {
                    Failure::new("browser_control_unavailable", None)
                }
                // The gateway's resource limit (an oversized reply).
                1013 => Failure::new("browser_limit", Some(reason)),
                _ => Failure::new("display_closed", (!reason.is_empty()).then_some(reason)),
            },
        }
    }

    /// Evaluates a page-side expression and returns its value; an asset that
    /// returns `{error}` becomes that reason.
    pub async fn eval(&mut self, session: &str, expression: &str) -> Result<Value, Failure> {
        if self.given_up(session) {
            return Err(self.blocked(CdpError::Timeout));
        }
        let answer = self
            .cdp
            .call(
                "Runtime.evaluate",
                json!({"expression": expression, "returnByValue": true}),
                Some(session),
                self.budget(),
            )
            .await;
        self.read_outcome(session, answer)
    }

    /// Evaluates one expression in each of several sessions, all sent before
    /// any answer is read and read under one deadline, so frames a page holds
    /// cost one wait between them. Each session's outcome is its own; only
    /// the connection ending fails the batch. A frame this command has given
    /// up on is not asked: its outcome is the silence it already showed.
    pub async fn eval_all(
        &mut self,
        calls: &[(&str, String)],
    ) -> Result<Vec<Result<Value, Failure>>, Failure> {
        let skipped: Vec<bool> = calls
            .iter()
            .map(|(session, _)| self.given_up(session))
            .collect();
        let requests: Vec<(&str, Value, &str)> = calls
            .iter()
            .zip(&skipped)
            .filter(|(_, skipped)| !**skipped)
            .map(|(call, _)| call)
            .map(|(session, expression)| {
                (
                    "Runtime.evaluate",
                    json!({"expression": expression, "returnByValue": true}),
                    *session,
                )
            })
            .collect();
        let answers = self
            .cdp
            .call_all(&requests, self.budget())
            .await
            .map_err(|error| self.failure(error))?;
        let mut answers = answers.into_iter();
        let mut outcomes = Vec::with_capacity(calls.len());
        for ((session, _), skipped_now) in calls.iter().zip(skipped) {
            let outcome = match skipped_now {
                true => Err(self.blocked(CdpError::Timeout)),
                false => {
                    let answer = answers.next().unwrap_or(Err(CdpError::Timeout));
                    self.read_outcome(session, answer)
                }
            };
            outcomes.push(outcome);
        }
        Ok(outcomes)
    }

    /// What a `Runtime.evaluate` answer says: its value, or the reason it
    /// stands for.
    fn value_of(&self, answer: Result<Value, CdpError>) -> Result<Value, Failure> {
        let answer = answer.map_err(|error| self.blocked(error))?;
        if let Some(details) = answer.get("exceptionDetails") {
            return Err(Failure::new(
                "page_script_failed",
                Some(first_line(details)),
            ));
        }
        let value = answer["result"]
            .get("value")
            .cloned()
            .unwrap_or(Value::Null);
        if let Some(reason) = value.get("error").and_then(Value::as_str) {
            return Err(Failure::new(
                reason,
                value["detail"].as_str().map(str::to_owned),
            ));
        }
        Ok(value)
    }

    /// A step that times out after the attach probe answered is held by a
    /// dialog the page opened meanwhile, or by a script that never yields.
    pub fn blocked(&self, error: CdpError) -> Failure {
        match error {
            CdpError::Timeout if self.cdp.dialog.is_some() => Failure::new("dialog_open", None),
            error => self.failure(error),
        }
    }

    pub async fn dom(&mut self, session: &str, op: &str, args: Value) -> Result<Value, Failure> {
        self.eval(session, &call(DOM_JS, op, &args)).await
    }

    pub async fn probe(&mut self) -> Result<Value, Failure> {
        let top = self.top.clone();
        self.dom(&top, "probe", json!({})).await
    }

    /// Input and pixels go only to a display on screen, so the operator sees
    /// the action. A View behind another tab of its area is hidden by the
    /// host but still answers, so this refusal is hided's own rule, read
    /// from the area's selection. The page's own visibility cannot tell: a
    /// selected View in a window covered by another app reads hidden and
    /// still takes input.
    pub async fn require_visible(&mut self) -> Result<Value, Failure> {
        if !self.selected {
            return Err(Failure::new("display_hidden", None));
        }
        self.probe().await
    }

    /// Draws on the operator's view of the page. It never fails a command.
    pub async fn overlay(&mut self, op: &str, args: Value) -> Option<Value> {
        if self.cdp.dialog.is_some() {
            return None;
        }
        let top = self.top.clone();
        let answer = self
            .cdp
            .call(
                "Runtime.evaluate",
                json!({"expression": call(OVERLAY_JS, op, &args), "returnByValue": true}),
                Some(&top),
                OVERLAY,
            )
            .await
            .ok()?;
        answer["result"].get("value").cloned()
    }

    /// The cross-origin frames under the page whose documents have been
    /// read, found by auto-attach. Nested frames attach under their parent
    /// frame. Every call folds in frames that attached or detached since and
    /// asks again the frames that gave no answer before, all of one kind
    /// together under one deadline, so the frames a page holds cost one wait
    /// per round between them. A frame that has left `MISS_LIMIT` rounds of
    /// this command unanswered is not asked again in it: it stays known,
    /// unread, and the next command asks it afresh.
    pub async fn frames(&mut self) -> Result<Vec<Frame>, Failure> {
        let detached: Vec<String> = self
            .cdp
            .take_events(|event| event.method == "Target.detachedFromTarget")
            .into_iter()
            .filter_map(|event| event.params["sessionId"].as_str().map(str::to_owned))
            .collect();
        self.known
            .retain(|frame| !detached.contains(&frame.session));
        self.scanned.retain(|session| !detached.contains(session));
        let mut tried: Vec<String> = Vec::new();
        // A look that went unanswered waits for the next read of the page, so
        // the frame's budget is not spent in one call.
        let mut scanned_now: Vec<String> = Vec::new();
        loop {
            let mut parents = Vec::new();
            if !self.scanned.contains(&self.top) {
                parents.push(self.top.clone());
            }
            parents.extend(
                self.known
                    .iter()
                    .filter(|frame| {
                        frame.document.is_some()
                            && !self.scanned.contains(&frame.session)
                            && !scanned_now.contains(&frame.session)
                            && frame.scan_misses < MISS_LIMIT
                    })
                    .map(|frame| frame.session.clone()),
            );
            scanned_now.extend(parents.iter().cloned());
            self.scan(&parents).await?;
            self.collect_attached();
            let todo: Vec<String> = self
                .known
                .iter()
                .filter(|frame| {
                    frame.document.is_none()
                        && frame.misses < MISS_LIMIT
                        && !tried.contains(&frame.session)
                })
                .map(|frame| frame.session.clone())
                .collect();
            if todo.is_empty() {
                break;
            }
            tried.extend(todo.iter().cloned());
            self.attach_round(&todo).await?;
            let unscanned = self.known.iter().any(|frame| {
                frame.document.is_some()
                    && !self.scanned.contains(&frame.session)
                    && !scanned_now.contains(&frame.session)
                    && frame.scan_misses < MISS_LIMIT
            });
            if !unscanned {
                break;
            }
        }
        // The tag lives in the frame's own document, so a hostile frame can
        // set any: a malformed one, or one that two frames share, names no
        // frame, and refs never reach a frame by guesswork.
        let tags: Vec<&str> = self
            .known
            .iter()
            .filter_map(|frame| frame.document.as_ref().map(|(tag, _)| tag.as_str()))
            .collect();
        Ok(self
            .known
            .iter()
            .filter_map(|frame| {
                let (tag, origin) = frame.document.as_ref()?;
                (valid_tag(tag) && tags.iter().filter(|known| **known == tag).count() == 1).then(
                    || Frame {
                        session: frame.session.clone(),
                        target: frame.target.clone(),
                        parent: frame.parent.clone(),
                        tag: tag.clone(),
                        origin: origin.clone(),
                    },
                )
            })
            .collect())
    }

    /// Looks for the frames attached under each of the sessions, all
    /// together: the auto-attach calls, then a round trip each to see that the
    /// attach events they sent have arrived. A frame that stops answering is
    /// looked into by a later call, within its budget; the top document's
    /// silence is the page's.
    async fn scan(&mut self, parents: &[String]) -> Result<(), Failure> {
        let attach: Vec<(&str, Value, &str)> = parents
            .iter()
            .map(|parent| {
                (
                    "Target.setAutoAttach",
                    json!({"autoAttach": true, "waitForDebuggerOnStart": false, "flatten": true,
                        "filter": [{"type": "iframe", "exclude": false}, {"exclude": true}]}),
                    parent.as_str(),
                )
            })
            .collect();
        let budget = self.budget();
        let answers = self
            .cdp
            .call_all(&attach, budget)
            .await
            .map_err(|error| self.failure(error))?;
        let mut attached = Vec::new();
        for (parent, answer) in parents.iter().zip(answers) {
            match answer {
                Ok(_) => attached.push(parent.as_str()),
                Err(error) => {
                    let asked = !matches!(error, CdpError::NotSent);
                    let failure = self.blocked(error);
                    self.scan_missed(parent, failure, asked)?;
                }
            }
        }
        let flushes: Vec<(&str, Value, &str)> = attached
            .iter()
            .map(|parent| {
                (
                    "Runtime.evaluate",
                    json!({"expression": "0", "returnByValue": true}),
                    *parent,
                )
            })
            .collect();
        let budget = self.budget();
        let answers = self
            .cdp
            .call_all(&flushes, budget)
            .await
            .map_err(|error| self.failure(error))?;
        for (parent, answer) in attached.into_iter().zip(answers) {
            match answer {
                Ok(_) => {
                    self.scanned.push(parent.to_owned());
                    if let Some(frame) = self.known.iter_mut().find(|f| f.session == parent) {
                        frame.scan_misses = 0;
                        frame.unlisted = false;
                    }
                }
                Err(error) => {
                    let asked = !matches!(error, CdpError::NotSent);
                    let failure = self.blocked(error);
                    self.scan_missed(parent, failure, asked)?;
                }
            }
        }
        Ok(())
    }

    /// A look for a frame's own frames that got no result. Asked and not
    /// answered it counts against the frame's budget for that look; not sent it
    /// does not, and is made again by a later read. Either way what is inside
    /// the frame is unlisted, not absent, and the frame, which has been read, is
    /// not silent.
    fn scan_missed(&mut self, parent: &str, failure: Failure, asked: bool) -> Result<(), Failure> {
        if failure.silent && parent != self.top {
            if let Some(frame) = self.known.iter_mut().find(|f| f.session == parent) {
                frame.unlisted = true;
                if asked {
                    frame.scan_misses = frame.scan_misses.saturating_add(1);
                }
            }
            Ok(())
        } else {
            Err(failure)
        }
    }

    /// Enables the dialog events of the given frames and reads their tags,
    /// each together. A frame that answers neither stays as it was and is
    /// asked again by the next call; only a dialog event this command saw
    /// is a dialog the operator must answer.
    async fn attach_round(&mut self, sessions: &[String]) -> Result<(), Failure> {
        let enables: Vec<(&str, Value, &str)> = self
            .known
            .iter()
            .filter(|frame| !frame.enabled && sessions.contains(&frame.session))
            .map(|frame| ("Page.enable", json!({}), frame.session.as_str()))
            .collect();
        let enabling: Vec<String> = enables
            .iter()
            .map(|(_, _, session)| (*session).to_owned())
            .collect();
        let budget = self.budget();
        let answers = self
            .cdp
            .call_all(&enables, budget)
            .await
            .map_err(|error| self.failure(error))?;
        for (session, answer) in enabling.iter().zip(answers) {
            // A call that was never sent says nothing of the frame.
            if matches!(answer, Err(CdpError::NotSent)) {
                continue;
            }
            let outcome = match answer {
                Ok(_) => Ok(()),
                Err(error) => Err(self.blocked(error)),
            };
            let Some(frame) = self
                .known
                .iter_mut()
                .find(|frame| frame.session == *session)
            else {
                continue;
            };
            match outcome {
                Ok(()) => {
                    frame.enabled = true;
                    frame.misses = 0;
                }
                Err(failure) if failure.silent => {
                    frame.silent = true;
                    frame.misses = frame.misses.saturating_add(1);
                }
                Err(failure) if failure.page_side() => {
                    frame.silent = false;
                    frame.misses = 0;
                }
                Err(failure) => return Err(failure),
            }
        }
        let tag_call = call(DOM_JS, "tag", &json!({}));
        let reading: Vec<String> = self
            .known
            .iter()
            .filter(|frame| {
                frame.enabled && frame.document.is_none() && sessions.contains(&frame.session)
            })
            .map(|frame| frame.session.clone())
            .collect();
        let calls: Vec<(&str, String)> = reading
            .iter()
            .map(|session| (session.as_str(), tag_call.clone()))
            .collect();
        let tags = self.eval_all(&calls).await?;
        for (session, tag) in reading.iter().zip(tags) {
            let Some(frame) = self
                .known
                .iter_mut()
                .find(|frame| frame.session == *session)
            else {
                continue;
            };
            match tag {
                Ok(tag) => {
                    frame.silent = false;
                    frame.document = Some((
                        tag["tag"].as_str().unwrap_or("").to_owned(),
                        tag["origin"].as_str().unwrap_or("opaque").to_owned(),
                    ));
                }
                Err(failure) if failure.silent => frame.silent = true,
                // A frame that navigated or closed while attaching is no
                // longer part of the page.
                Err(failure) if failure.page_side() => frame.silent = false,
                Err(failure) => return Err(failure),
            }
        }
        Ok(())
    }

    fn collect_attached(&mut self) {
        let attached: Vec<Event> = self
            .cdp
            .take_events(|event| event.method == "Target.attachedToTarget");
        for event in attached {
            let info = &event.params["targetInfo"];
            if info["type"] != "iframe" {
                continue;
            }
            if let (Some(session), Some(target), Some(parent)) = (
                event.params["sessionId"].as_str(),
                info["targetId"].as_str(),
                event.session.as_deref(),
            ) && self.known.len() < MAX_FRAMES
                && !self.known.iter().any(|frame| frame.session == session)
            {
                self.known.push(Known {
                    session: session.to_owned(),
                    target: target.to_owned(),
                    parent: parent.to_owned(),
                    attached_origin: origin_of(info["url"].as_str().unwrap_or("")),
                    enabled: false,
                    silent: false,
                    document: None,
                    misses: 0,
                    scan_misses: 0,
                    unlisted: false,
                });
            }
        }
    }

    /// Events sent before this round trip's answer have all arrived.
    pub async fn flush(&mut self, session: &str) -> Result<(), Failure> {
        self.cdp
            .call(
                "Runtime.evaluate",
                json!({"expression": "0", "returnByValue": true}),
                Some(session),
                self.budget(),
            )
            .await
            .map(|_| ())
            .map_err(|error| self.blocked(error))
    }

    /// The frame an `@<tag>:N` ref names. No frame answering to the tag is
    /// `Stale` only when every frame that could carry it was read; a frame
    /// that did not answer may be the one, which is `Unknown`, never absence.
    pub async fn lookup(&mut self, tag: &str) -> Result<Lookup, Failure> {
        if let Some(frame) = self
            .frames()
            .await?
            .into_iter()
            .find(|frame| frame.tag == tag)
        {
            return Ok(Lookup::Found(frame));
        }
        // A frame with no document may be the one, and so may the children of
        // a frame whose own frames could not be looked for.
        let unreadable = self.known.iter().any(|frame| {
            frame.document.is_none() && frame.silent
                || frame.document.is_some() && !self.scanned.contains(&frame.session)
        });
        Ok(if unreadable {
            Lookup::Unknown
        } else {
            Lookup::Stale
        })
    }

    /// The frame an `@<tag>:N` ref names, or `ref_stale` when no current
    /// frame document carries the tag (it navigated or closed), or a
    /// silent failure when a frame that did not answer may carry it.
    pub async fn frame(&mut self, tag: &str) -> Result<Frame, Failure> {
        match self.lookup(tag).await? {
            Lookup::Found(frame) => Ok(frame),
            Lookup::Stale => Err(Failure::new("ref_stale", Some(format!("frame {tag}")))),
            // A frame that did not answer cannot be told from the one that
            // carries the tag, so the ref is not known stale; a ref from an
            // older snapshot is the likelier cause, and its remedy is the same
            // whether or not the silent frame was ever the ref's.
            Lookup::Unknown => Err(Failure {
                silent: true,
                next_action: Some(format!(
                    "A frame that did not answer may hold frame {tag}, so the ref cannot be told stale. If it came from an earlier snapshot, take a fresh snapshot (hide browser snapshot {}) and use its refs; otherwise retry, or ask the operator to check the page",
                    self.display
                )),
                ..Failure::new(
                    "page_unresponsive",
                    Some(format!("a frame that did not answer may hold frame {tag}")),
                )
            }),
        }
    }

    /// Where a cross-origin frame's viewport starts in the top viewport, so
    /// its points can be drawn and cropped in top-page coordinates.
    pub async fn frame_offset(&mut self, frame: &Frame) -> Result<(f64, f64), Failure> {
        let frames = self.frames().await?;
        let mut offset = (0.0, 0.0);
        let mut current = frame.clone();
        loop {
            let owner = self
                .cdp
                .call(
                    "DOM.getFrameOwner",
                    json!({"frameId": current.target}),
                    Some(&current.parent),
                    self.budget(),
                )
                .await
                .map_err(|error| self.blocked(error))?;
            let model = self
                .cdp
                .call(
                    "DOM.getBoxModel",
                    json!({"backendNodeId": owner["backendNodeId"]}),
                    Some(&current.parent),
                    self.budget(),
                )
                .await
                .map_err(|error| self.blocked(error))?;
            let content = &model["model"]["content"];
            offset.0 += content[0].as_f64().unwrap_or(0.0);
            offset.1 += content[1].as_f64().unwrap_or(0.0);
            match frames.iter().find(|frame| frame.session == current.parent) {
                Some(parent) => current = parent.clone(),
                None => return Ok(offset),
            }
        }
    }

    /// Reads every frame. `baseline` names the --diff baseline kind to swap
    /// in each frame's own document; cross-origin frames are read with their
    /// values and paths reduced to origins.
    pub async fn snapshot(
        &mut self,
        filter: Option<&str>,
        clickable: &str,
    ) -> Result<Composite, Failure> {
        let top = self.top.clone();
        let text = self
            .eval(
                &top,
                &format!(
                    "({SNAPSHOT_JS})({},{},false)",
                    json!(filter),
                    json!(clickable)
                ),
            )
            .await?;
        let top_text = text.as_str().unwrap_or("").to_owned();
        // Every frame is read together, then each document's tag is read
        // together again to see that it is still the one that was read.
        // A frame that did not answer is noted in this snapshot only: the
        // next one asks it again.
        let frames = self.frames().await?;
        let read = format!(
            "({SNAPSHOT_JS})({},{},true)",
            json!(filter),
            json!(clickable)
        );
        let reads: Vec<(&str, String)> = frames
            .iter()
            .map(|frame| (frame.session.as_str(), read.clone()))
            .collect();
        let texts = self.eval_all(&reads).await?;
        let mut unread = Vec::new();
        let mut read_frames = Vec::new();
        for (frame, text) in frames.iter().zip(texts) {
            match text {
                Ok(text) => read_frames.push((frame, text)),
                Err(failure) if failure.silent => unread.push(frame.session.clone()),
                // A frame that navigated mid-read has a new document, and its
                // refs a new tag; the next snapshot reads it again.
                Err(failure) if failure.page_side() => {}
                Err(failure) => return Err(failure),
            }
        }
        let tag_call = call(DOM_JS, "tag", &json!({}));
        let checks: Vec<(&str, String)> = read_frames
            .iter()
            .map(|(frame, _)| (frame.session.as_str(), tag_call.clone()))
            .collect();
        let current = self.eval_all(&checks).await?;
        let mut sections = Vec::new();
        for ((frame, text), current) in read_frames.into_iter().zip(current) {
            match current {
                Ok(current) if current["tag"] == frame.tag.as_str() => {}
                Ok(_) => continue,
                Err(failure) if failure.silent => {
                    unread.push(frame.session.clone());
                    continue;
                }
                Err(failure) if failure.page_side() => continue,
                Err(failure) => return Err(failure),
            }
            let body = section_body(text.as_str().unwrap_or(""), &frame.tag);
            if body.is_empty() {
                continue;
            }
            sections.push((
                frame.session.clone(),
                format!("# OOPIF {} origin={}\n{body}", frame.tag, frame.origin),
            ));
        }
        self.unread = unread;
        Ok(Composite {
            top: top_text,
            sections,
            silent: self.notes(),
            unread: self.unreadable(),
            unlisted: self.unlisted(),
            lineage: frames
                .iter()
                .map(|frame| (frame.session.clone(), frame.parent.clone()))
                .collect(),
        })
    }

    /// Records which tagged frames the last multi-frame read got no answer
    /// from.
    pub fn record_unread(&mut self, sessions: Vec<String>) {
        self.unread = sessions;
    }

    /// The notes of the frames that cannot be read now: frames that gave no
    /// document, and frames the last read got no answer from.
    pub fn notes(&self) -> Vec<String> {
        let unreadable = self.unreadable();
        let unlisted = self.unlisted();
        unreadable
            .iter()
            .map(|(_, origin)| note(origin))
            .chain(unlisted.iter().map(|(_, origin)| unlisted_note(origin)))
            .collect()
    }

    /// The session and origin of the frames that were read and whose own
    /// frames could not be listed, in attach order. What is inside them is
    /// unknown, not absent; the frames themselves are as readable as any.
    pub fn unlisted(&self) -> Vec<(String, String)> {
        self.known
            .iter()
            .filter_map(|frame| match &frame.document {
                Some((_, origin)) if frame.unlisted => {
                    Some((frame.session.clone(), origin.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// The session and origin of those frames, in attach order.
    pub fn unreadable(&self) -> Vec<(String, String)> {
        self.known
            .iter()
            .filter_map(|frame| match &frame.document {
                None if frame.silent => {
                    Some((frame.session.clone(), frame.attached_origin.clone()))
                }
                Some((_, origin)) if self.unread.contains(&frame.session) => {
                    Some((frame.session.clone(), origin.clone()))
                }
                _ => None,
            })
            .collect()
    }

    /// Swaps the composite into each frame's document as the new baseline
    /// of `key` and returns the previous composite, or none when the top
    /// document has none (a first snapshot or a new document).
    ///
    /// A frame that does not answer while its baseline is swapped has none to
    /// give back, so it leaves the composite for a note, as one that did not
    /// answer the snapshot does: the diff then shows its note, not every line
    /// of it as new. The top document's baseline is swapped last, with those
    /// notes in it.
    pub async fn swap_baseline(
        &mut self,
        key: &str,
        composite: &mut Composite,
    ) -> Result<Option<String>, Failure> {
        let calls: Vec<(&str, String)> = composite
            .sections
            .iter()
            .map(|(session, section)| {
                (
                    session.as_str(),
                    call(DOM_JS, "baseline", &json!({"key": key, "text": section})),
                )
            })
            .collect();
        let answers = self.eval_all(&calls).await?;
        let mut previous_sections = Vec::new();
        let mut silent = Vec::new();
        for ((session, _), answer) in composite.sections.iter().zip(answers) {
            match answer {
                Ok(answer) => {
                    if let Some(previous) = answer["previous"].as_str() {
                        previous_sections.push(previous.to_owned());
                    }
                }
                Err(failure) if failure.silent => silent.push(session.clone()),
                // A frame that navigated has no baseline to swap either.
                Err(failure) if failure.page_side() => {}
                Err(failure) => return Err(failure),
            }
        }
        composite
            .sections
            .retain(|(session, _)| !silent.contains(session));
        self.unread.extend(silent);
        composite.silent = self.notes();
        composite.unread = self.unreadable();
        composite.unlisted = self.unlisted();
        let top = self.top.clone();
        let previous_top = self
            .dom(
                &top,
                "baseline",
                json!({"key": key, "text": composite.top_block()}),
            )
            .await?["previous"]
            .as_str()
            .map(str::to_owned);
        Ok(previous_top.map(|top| compose(&top, &previous_sections)))
    }

    /// Runs render.js in a fresh isolated world of the top frame: it sees
    /// only its arguments, and page scripts cannot see them.
    pub async fn render(&mut self, op: &str, args: Value) -> Result<Value, Failure> {
        let top = self.top.clone();
        let tree = self
            .cdp
            .call("Page.getFrameTree", json!({}), Some(&top), self.budget())
            .await
            .map_err(|error| self.blocked(error))?;
        let world = self
            .cdp
            .call(
                "Page.createIsolatedWorld",
                json!({"frameId": tree["frameTree"]["frame"]["id"], "worldName": "hide-browser"}),
                Some(&top),
                self.budget(),
            )
            .await
            .map_err(|error| self.blocked(error))?;
        let answer = self
            .cdp
            .call(
                "Runtime.evaluate",
                json!({"expression": call(RENDER_JS, op, &args), "contextId": world["executionContextId"], "returnByValue": true}),
                Some(&top),
                self.budget(),
            )
            .await
            .map_err(|error| self.blocked(error))?;
        if let Some(details) = answer.get("exceptionDetails") {
            return Err(Failure::new(
                "page_script_failed",
                Some(first_line(details)),
            ));
        }
        Ok(answer["result"]
            .get("value")
            .cloned()
            .unwrap_or(Value::Null))
    }

    pub async fn close(self) {
        self.cdp.close().await;
    }
}

/// `top`, a snapshot's top document, with its address line naming the display's
/// `source` address in place of the loopback route the page loaded from. Only a
/// loopback http(s) address on another origin than `source`'s is a route; its
/// path, query and fragment are the page's own and stay.
pub fn with_source_address(top: &str, source: Option<&str>) -> String {
    let Some(source) = source.and_then(origin_split) else {
        return top.to_owned();
    };
    let mut lines = top.splitn(3, '\n');
    let (Some(title), Some(address)) = (lines.next(), lines.next()) else {
        return top.to_owned();
    };
    let Some((origin, tail)) = address.strip_prefix("# ").and_then(origin_split) else {
        return top.to_owned();
    };
    if origin == source.0 || !loopback_origin(origin) {
        return top.to_owned();
    }
    let mut out = format!("{title}\n# {}{tail}", source.0);
    if let Some(rest) = lines.next() {
        out.push('\n');
        out.push_str(rest);
    }
    out
}

/// An http(s) address as its origin and the rest (path, query, fragment).
fn origin_split(url: &str) -> Option<(&str, &str)> {
    let (scheme, after) = url.split_once("://")?;
    if !matches!(scheme, "http" | "https") {
        return None;
    }
    let end = after.find(['/', '?', '#']).unwrap_or(after.len());
    if after[..end].contains('@') || end == 0 {
        return None;
    }
    let split = scheme.len() + 3 + end;
    Some((&url[..split], &url[split..]))
}

fn loopback_origin(origin: &str) -> bool {
    let authority = origin
        .split_once("://")
        .map_or("", |(_, authority)| authority);
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split_once(']').map_or("", |(host, _)| host),
        None => authority.split(':').next().unwrap_or(""),
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.to_canonical().is_loopback())
}

/// A frame's snapshot without its title and address lines, with each ref
/// renamed into the frame's namespace (`@3` -> `@k7q2:3`).
pub fn section_body(text: &str, tag: &str) -> String {
    text.lines()
        .skip(2)
        .map(|line| {
            let indent = line.len() - line.trim_start_matches(' ').len();
            let rest = &line[indent..];
            match rest.strip_prefix('@') {
                Some(after)
                    if after.split_once(' ').is_some_and(|(digits, _)| {
                        !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
                    }) =>
                {
                    format!("{}@{tag}:{after}", &line[..indent])
                }
                _ => line.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_owned()
}

pub fn first_line(details: &Value) -> String {
    details["exception"]["description"]
        .as_str()
        .or_else(|| details["text"].as_str())
        .unwrap_or("script failed")
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(300)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routed_pages_address_line_names_its_source_address() {
        let top = "# Title\n# http://127.0.0.1:64423/docs/index.html?x=1#top\n\n@1 button \"Go\"\n";
        assert_eq!(
            with_source_address(top, Some("http://localhost:8765/index.html")),
            "# Title\n# http://localhost:8765/docs/index.html?x=1#top\n\n@1 button \"Go\"\n"
        );
        // The page already at its source address, a page that is not a route,
        // and a display with no known source stay as they are.
        for (top, source) in [
            (
                "# T\n# http://localhost:8765/a\n\n",
                Some("http://localhost:8765/"),
            ),
            (
                "# T\n# https://example.com/a\n\n",
                Some("http://localhost:8765/"),
            ),
            ("# T\n# http://127.0.0.1:1/a\n\n", Some("about:blank")),
            (
                "# T\n# http://127.evil.example:1/a\n\n",
                Some("http://localhost:8765/"),
            ),
            ("# T\n# http://127.0.0.1:1/a\n\n", None),
        ] {
            assert_eq!(with_source_address(top, source), top);
        }
    }

    #[test]
    fn a_frame_section_names_its_refs_in_its_own_namespace() {
        let text = "# Child\n# http://localhost:1/child\n\n@1 textbox \"Name\" [text]\n  @12 button \"Go\"\n  heading \"@3 is text\"\n";
        assert_eq!(
            section_body(text, "k7q2"),
            "@k7q2:1 textbox \"Name\" [text]\n  @k7q2:12 button \"Go\"\n  heading \"@3 is text\""
        );
    }

    #[test]
    fn a_composite_puts_each_frame_section_after_the_top_document() {
        let composite = Composite {
            top: "# T\n# http://a/\n\n@1 button \"A\"\n".into(),
            sections: vec![(
                "s1".into(),
                "# OOPIF k7q2 origin=http://b\n@k7q2:1 link \"B\"".into(),
            )],
            silent: Vec::new(),
            unread: Vec::new(),
            unlisted: Vec::new(),
            lineage: Vec::new(),
        };
        assert_eq!(
            composite.text(),
            "# T\n# http://a/\n\n@1 button \"A\"\n\n# OOPIF k7q2 origin=http://b\n@k7q2:1 link \"B\"\n"
        );
    }

    fn composite(top: &str, sections: &[(&str, &str)], unread: &[(&str, &str)]) -> Composite {
        Composite {
            top: top.into(),
            sections: sections
                .iter()
                .map(|(session, text)| ((*session).into(), (*text).into()))
                .collect(),
            silent: Vec::new(),
            unread: unread
                .iter()
                .map(|(session, origin)| ((*session).into(), (*origin).into()))
                .collect(),
            unlisted: Vec::new(),
            lineage: Vec::new(),
        }
    }

    #[test]
    fn two_reads_are_compared_on_the_frames_both_could_read() {
        let top = "# T\n# http://a/\n\n@1 button \"A\"\n";
        let read = composite(top, &[("s1", "# OOPIF k7q2\n@k7q2:1 link \"B\"")], &[]);
        let unread = composite(top, &[], &[("s1", "http://b.test")]);
        // The frame answered in one read and not the other, in either order:
        // it is in neither text, and it is named.
        for (before, after) in [(&read, &unread), (&unread, &read)] {
            let seen = compare(before, after);
            assert_eq!(seen.before, seen.after);
            assert_eq!(seen.unread, ["http://b.test"]);
        }
        // Two reads that both answered compare in full.
        let other = composite(top, &[("s1", "# OOPIF k7q2\n@k7q2:1 link \"C\"")], &[]);
        let seen = compare(&read, &other);
        assert_ne!(seen.before, seen.after);
        assert!(seen.unread.is_empty());
    }

    use super::super::fake_gateway::{self as fake, Frames, page, page_with_frames};

    #[tokio::test]
    async fn a_frame_that_answers_nothing_is_noted_and_the_rest_of_the_page_is_read() {
        let mut page = page(page_with_frames(
            Frames {
                hung: 1,
                healthy: 1,
                ..Frames::default()
            },
            None,
        ))
        .await;
        let text = page.snapshot(None, "auto").await.unwrap().text();
        assert!(text.contains("@1 button \"A\""), "{text}");
        assert!(
            text.contains("# OOPIF k7q2 origin=http://ok.test:9\n@k7q2:1 link \"B\""),
            "{text}"
        );
        // Its origin is spelled as the frame's own would be, and the note
        // follows the top document.
        assert!(
            text.contains("@1 button \"A\"\n\n# OOPIF unresponsive origin=http://hung1.test - no answer in time"),
            "{text}"
        );
        // It has no tag, so no ref reaches it and no baseline waits on it.
        assert_eq!(page.frames().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn every_request_of_a_kind_goes_out_before_any_answer_is_read() {
        // A frame answers only once all three have asked, so a client that
        // waits for each answer before the next request is told they are
        // silent: this holds for the frames' `Page.enable`, their tags, their
        // snapshot reads, the tag checks after them and their baselines.
        let together = Frames {
            healthy: 3,
            together: true,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(together, None)).await;
        let mut composite = page.snapshot(None, "auto").await.unwrap();
        assert_eq!(composite.sections.len(), 3);
        assert!(composite.silent.is_empty(), "{:?}", composite.silent);
        let previous = page
            .swap_baseline("full", &mut composite)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(composite.sections.len(), 3);
        assert_eq!(previous.matches("\nold").count(), 3, "{previous}");
    }

    #[tokio::test]
    async fn a_frame_that_missed_one_read_is_read_by_the_next() {
        // The first frame leaves its first snapshot read unanswered, as the
        // read before an action and the one after it are separate reads.
        let slow = Frames {
            healthy: 2,
            slow_read_once: true,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(slow, None)).await;
        let before = page.snapshot(None, "stable").await.unwrap();
        assert_eq!(before.sections.len(), 1);
        assert_eq!(before.silent.len(), 1);
        let after = page.snapshot(None, "stable").await.unwrap();
        assert_eq!(after.sections.len(), 2);
        assert!(after.silent.is_empty(), "{:?}", after.silent);
    }

    #[tokio::test]
    async fn a_frame_that_was_slow_to_attach_is_enabled_and_tagged_by_the_next_call() {
        let slow = Frames {
            healthy: 1,
            slow_enable_once: true,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(slow, None)).await;
        assert!(page.frames().await.unwrap().is_empty());
        // Until it answers, a ref cannot be said to name nothing.
        assert!(matches!(page.lookup("k7q2").await, Ok(Lookup::Found(_))));
    }

    #[tokio::test]
    async fn a_ref_is_unknown_while_a_frame_that_may_carry_it_has_not_answered() {
        let silent = Frames {
            hung: 1,
            ..Frames::default()
        };
        let mut page = page(page_with_frames(silent, None)).await;
        assert!(matches!(page.lookup("k7q2").await, Ok(Lookup::Unknown)));
        let failure = page.frame("k7q2").await.unwrap_err();
        assert_eq!(failure.reason, "page_unresponsive");
        assert!(failure.silent);
        let none = Frames::default();
        let mut page = super::super::fake_gateway::page(page_with_frames(none, None)).await;
        assert!(matches!(page.lookup("k7q2").await, Ok(Lookup::Stale)));
    }

    #[tokio::test]
    async fn a_batch_larger_than_the_gateway_keeps_in_flight_goes_in_rounds() {
        let mut cdp =
            fake::gateway(|request| vec![fake::reply(request, json!({"echo": request["id"]}))])
                .await;
        let calls: Vec<(&str, Value, &str)> = (0..70)
            .map(|_| ("Runtime.evaluate", json!({}), "top"))
            .collect();
        let answers = cdp.call_all(&calls, fake::STEP).await.unwrap();
        assert_eq!(answers.len(), 70);
        assert!(answers.iter().all(Result::is_ok));
    }

    #[tokio::test]
    async fn commands_the_gateway_still_holds_never_pass_its_cap_and_never_hold_back_the_top_document()
     {
        // 24 frames never answer `Page.enable`. The step lapses and the
        // gateway still holds the 24 until its own, longer deadline: sent
        // again at once that is 48 pending, and the gateway closes the
        // connection. The ask of a session that has not answered its last one
        // is not sent; the page's own calls are, whatever is held.
        let (script, seen) = fake::recording(page_with_frames(
            Frames {
                hung: 24,
                ..Frames::default()
            },
            None,
        ));
        let mut page = page(script).await;
        page.frames().await.unwrap();
        page.frames().await.unwrap();
        let mut asks: Vec<(&str, String)> = Vec::new();
        let sessions: Vec<String> = (1..=24).map(|n| format!("f{n}")).collect();
        asks.extend(
            sessions
                .iter()
                .map(|session| (session.as_str(), "1".to_owned())),
        );
        page.eval_all(&asks).await.unwrap();
        let seen = seen.lock().unwrap();
        assert!(seen.most_pending >= 24, "{}", seen.most_pending);
        assert!(seen.most_pending <= 32, "{}", seen.most_pending);
        assert_eq!(seen.sent("Page.enable", "f1"), 1);
        // The look for frames' flush, once.
        assert_eq!(seen.sent("Runtime.evaluate", "top"), 1);
    }

    #[tokio::test]
    async fn the_top_document_is_asked_when_frame_reads_have_filled_their_room() {
        // 28 sessions the gateway holds a command of, as many as frame reads
        // take: the top document is asked beside them all the same, and
        // answers.
        let (script, seen) = fake::recording(page_with_frames(Frames::default(), None));
        let mut page = page(script).await;
        let sessions: Vec<String> = (1..=28).map(|n| format!("g{n}")).collect();
        let asks: Vec<(&str, String)> = sessions
            .iter()
            .map(|session| (session.as_str(), "1".to_owned()))
            .collect();
        page.eval_all(&asks).await.unwrap();
        assert_eq!(seen.lock().unwrap().most_pending, 28);
        let top = page.top.clone();
        let outcomes = page
            .eval_all(&[(top.as_str(), "1".to_owned())])
            .await
            .unwrap();
        assert!(outcomes[0].is_ok(), "{:?}", outcomes[0]);
        assert!(seen.lock().unwrap().most_pending <= 32);
    }

    #[tokio::test]
    async fn frames_that_answer_are_read_beside_frames_the_gateway_still_holds_commands_of() {
        // 16 of 24 frames never answer. Reads that include them go on reading
        // the 8 that do, and no read charges those a miss.
        let (script, seen) = fake::recording(page_with_frames(
            Frames {
                hung: 16,
                healthy: 8,
                ..Frames::default()
            },
            None,
        ));
        let mut page = page(script).await;
        for _ in 0..4 {
            let composite = page.snapshot(None, "auto").await.unwrap();
            assert_eq!(composite.sections.len(), 8);
            assert_eq!(composite.silent.len(), 16, "{:?}", composite.silent);
        }
        assert!(seen.lock().unwrap().most_pending <= 32);
    }

    #[tokio::test]
    async fn a_frame_that_answers_starts_its_budget_again() {
        // The frame leaves every other read unanswered. Two silent rounds in a
        // row give a frame up; two in the whole command do not.
        let mut page = page(page_with_frames(
            Frames {
                healthy: 1,
                flaky_reads: true,
                ..Frames::default()
            },
            None,
        ))
        .await;
        let mut read = Vec::new();
        for _ in 0..6 {
            read.push(page.snapshot(None, "stable").await.unwrap().sections.len());
        }
        assert_eq!(read, [0, 1, 0, 1, 0, 1]);
    }

    #[tokio::test]
    async fn a_frame_whose_own_frames_could_not_be_listed_keeps_its_section_and_is_not_called_unresponsive()
     {
        let mut page = page(page_with_frames(
            Frames {
                healthy: 1,
                scan_silent: true,
                ..Frames::default()
            },
            None,
        ))
        .await;
        let composite = page.snapshot(None, "auto").await.unwrap();
        // It was read: its section and its refs are there, and it is not an
        // unread frame.
        assert_eq!(composite.sections.len(), 1);
        assert!(composite.sections[0].1.contains("@k7q2:1 link"));
        assert!(composite.unread.is_empty(), "{:?}", composite.unread);
        // What is inside it is named as not listed, not as a frame that did
        // not answer.
        assert_eq!(
            composite.unlisted,
            [("ok1".to_owned(), "http://ok.test:9".to_owned())]
        );
        assert_eq!(composite.silent.len(), 1);
        assert!(composite.silent[0].contains("OOPIF unlisted"));
        assert!(!composite.silent[0].contains("unresponsive"));
        // Given up after two looks, it is still named by the later reads.
        for _ in 0..3 {
            let later = page.snapshot(None, "auto").await.unwrap();
            assert_eq!(later.silent, composite.silent);
        }
    }

    #[tokio::test]
    async fn a_frame_that_answers_reads_but_not_the_look_for_its_own_frames_is_given_up_for_that_look()
     {
        // A read that is answered is not an answer to the look: every snapshot
        // of the command must not pay a step for it again.
        let (script, seen) = fake::recording(page_with_frames(
            Frames {
                healthy: 1,
                scan_silent: true,
                ..Frames::default()
            },
            None,
        ));
        let mut page = page(script).await;
        for _ in 0..5 {
            assert_eq!(page.snapshot(None, "auto").await.unwrap().sections.len(), 1);
        }
        assert_eq!(
            seen.lock().unwrap().sent("Target.setAutoAttach", "ok1"),
            MISS_LIMIT as usize
        );
    }

    #[tokio::test]
    async fn a_call_the_gateways_room_kept_back_is_not_a_miss_and_is_made_once_the_held_commands_end()
     {
        // 16 frames hold a command each, and so does the one read of the
        // slow frame: no room for a second ask of it. The read that cannot be
        // sent leaves it unread, and does not count against it.
        let (script, seen) = fake::recording_held(page_with_frames(
            Frames {
                hung: 16,
                healthy: 1,
                slow_read_once: true,
                ..Frames::default()
            },
            None,
        ));
        let mut page = page(script).await;
        assert_eq!(page.snapshot(None, "auto").await.unwrap().sections.len(), 0);
        assert_eq!(page.snapshot(None, "auto").await.unwrap().sections.len(), 0);
        assert!(!page.given_up("ok1"));
        // The gateway ends what it holds; the frame is read.
        seen.lock().unwrap().release = true;
        assert_eq!(page.snapshot(None, "auto").await.unwrap().sections.len(), 1);
        assert!(!page.given_up("ok1"));
    }

    #[test]
    fn a_frame_read_in_both_reads_is_compared_and_the_frames_inside_it_that_could_not_be_listed_are_not()
     {
        let top = "# T\n# http://a/\n\n@1 button \"A\"\n";
        let mut before = composite(
            top,
            &[
                ("s1", "# OOPIF k7q2\n@k7q2:1 link \"B\""),
                ("s2", "# OOPIF k7q3\n@k7q3:1 link \"C\""),
            ],
            &[],
        );
        before.lineage = vec![("s1".into(), "top".into()), ("s2".into(), "s1".into())];
        // The later read could not list what is inside s1: s2 is not there,
        // and is not gone. s1 itself was read, and what it says changed.
        let mut after = composite(top, &[("s1", "# OOPIF k7q2\n@k7q2:1 link \"D\"")], &[]);
        after.unlisted = vec![("s1".into(), "http://b.test".into())];
        let seen = compare(&before, &after);
        assert!(seen.before.contains("link \"B\""));
        assert!(seen.after.contains("link \"D\""));
        assert!(!seen.before.contains("link \"C\""), "{}", seen.before);
        assert_eq!(seen.unread, ["http://b.test"]);
    }

    #[test]
    fn frames_inside_a_frame_one_read_could_not_read_are_left_out_of_both_texts() {
        let top = "# T\n# http://a/\n\n@1 button \"A\"\n";
        let mut before = composite(
            top,
            &[
                ("s1", "# OOPIF k7q2\n@k7q2:1 link \"B\""),
                ("s2", "# OOPIF k7q3\n@k7q3:1 link \"C\""),
            ],
            &[],
        );
        before.lineage = vec![("s1".into(), "top".into()), ("s2".into(), "s1".into())];
        // The later read could not read s1 at all, so neither it nor what is
        // inside it is compared.
        let after = composite(top, &[], &[("s1", "http://b.test")]);
        let seen = compare(&before, &after);
        assert_eq!(seen.before, seen.after);
        assert_eq!(seen.unread, ["http://b.test"]);
    }

    #[tokio::test]
    async fn a_frame_is_asked_again_only_within_the_budget_of_the_command() {
        let (script, seen) = fake::recording(page_with_frames(
            Frames {
                hung: 1,
                ..Frames::default()
            },
            None,
        ));
        let mut page = page(script).await;
        for _ in 0..5 {
            assert!(page.frames().await.unwrap().is_empty());
        }
        assert_eq!(
            seen.lock().unwrap().sent("Page.enable", "f1"),
            MISS_LIMIT as usize
        );
        // Not asking is not knowing: the frame stays unread, and no ref is
        // stale while it may carry one.
        assert!(matches!(page.lookup("k7q2").await, Ok(Lookup::Unknown)));
        assert_eq!(page.notes().len(), 1);
    }

    #[tokio::test]
    async fn a_ref_no_frame_carries_beside_a_silent_frame_says_to_take_a_fresh_snapshot() {
        let mut page = page(page_with_frames(
            Frames {
                hung: 1,
                ..Frames::default()
            },
            None,
        ))
        .await;
        let failure = page.frame("old1").await.unwrap_err();
        assert_eq!(failure.reason, "page_unresponsive");
        assert!(failure.silent);
        let next = failure.next_action.unwrap();
        assert!(
            next.contains("take a fresh snapshot (hide browser snapshot browser-1)"),
            "{next}"
        );
    }

    #[tokio::test]
    async fn a_closed_connection_is_not_a_frame_that_answers_nothing() {
        let script = page_with_frames(
            Frames {
                hung: 1,
                ..Frames::default()
            },
            Some(fake::close(crate::browser_relay::CLOSE_IDLE)),
        );
        let mut page = page(script).await;
        let failure = page.snapshot(None, "auto").await.err().unwrap();
        assert_eq!(failure.reason, "page_unresponsive");
        assert!(!failure.silent);
    }

    #[tokio::test]
    async fn a_frame_that_stops_answering_mid_read_is_noted_too() {
        let mut page = page(page_with_frames(
            Frames {
                healthy: 1,
                stops_on_read: true,
                ..Frames::default()
            },
            None,
        ))
        .await;
        let composite = page.snapshot(None, "auto").await.unwrap();
        assert_eq!(
            composite.silent,
            vec![
                "# OOPIF unresponsive origin=http://ok.test:9 - no answer in time; a script that never yields, or a dialog the operator has not answered"
            ]
        );
    }

    #[tokio::test]
    async fn a_frame_that_stops_answering_while_its_baseline_is_swapped_is_noted_not_shown_as_new()
    {
        let mut page = page(page_with_frames(
            Frames {
                healthy: 2,
                stops_on_baseline: true,
                ..Frames::default()
            },
            None,
        ))
        .await;
        let mut composite = page.snapshot(None, "auto").await.unwrap();
        assert_eq!(composite.sections.len(), 2);
        let previous = page
            .swap_baseline("full", &mut composite)
            .await
            .unwrap()
            .unwrap();
        // The first frame leaves the composite for a note; the diff then
        // reads the other frame's section against its own previous one.
        assert_eq!(composite.sections.len(), 1);
        assert!(composite.sections[0].1.starts_with("# OOPIF k7q3 "));
        assert_eq!(composite.silent.len(), 1);
        assert!(
            composite
                .text()
                .contains("# OOPIF unresponsive origin=http://ok.test:9")
        );
        assert_eq!(
            previous,
            "# T\n# http://a/\n\n# OOPIF k7q3 origin=http://ok.test:9\nold\n"
        );
        // The note is part of what the top document's baseline keeps.
        assert!(composite.top_block().contains("# OOPIF unresponsive"));
    }

    #[tokio::test]
    async fn a_dialog_event_in_the_same_command_still_fails_dialog_open() {
        let dialog = json!({"method": "Page.javascriptDialogOpening", "sessionId": "f1",
            "params": {"type": "alert", "message": "Hello"}});
        let mut page = page(page_with_frames(
            Frames {
                hung: 1,
                ..Frames::default()
            },
            Some(dialog),
        ))
        .await;
        let failure = page.snapshot(None, "auto").await.err().unwrap();
        assert_eq!(failure.reason, "dialog_open");
    }

    #[tokio::test]
    async fn a_top_document_that_answers_nothing_still_fails_dialog_open() {
        let cdp = fake::gateway(|request| match request["method"].as_str().unwrap() {
            "Target.getTargets" => vec![fake::reply(
                request,
                json!({"targetInfos": [{"type": "page", "targetId": "p1"}]}),
            )],
            "Target.attachToTarget" => vec![fake::reply(request, json!({"sessionId": "top"}))],
            _ => Vec::new(),
        })
        .await;
        let failure = Page::attach_within(cdp, "browser-1", true, fake::STEP)
            .await
            .err()
            .unwrap();
        assert_eq!(failure.reason, "dialog_open");
    }

    #[test]
    fn a_frames_origin_is_spelled_as_its_own_would_be() {
        for (url, origin) in [
            (
                "https://user:pw@ads.test:8443/a?b#c",
                "https://ads.test:8443",
            ),
            ("http://localhost:3/", "http://localhost:3"),
            ("HTTP://Ads.Test:80/x", "http://ads.test"),
            ("https://ads.test:443", "https://ads.test"),
            ("http://[::1]:8080/", "http://[::1]:8080"),
            ("http://[::1]/", "http://[::1]"),
        ] {
            assert_eq!(origin_of(url), origin, "{url}");
        }
        for url in [
            "",
            "about:blank",
            "data:text/html,x",
            "file:///etc/passwd",
            "http:///x",
        ] {
            assert_eq!(origin_of(url), "opaque", "{url}");
        }
    }
}
