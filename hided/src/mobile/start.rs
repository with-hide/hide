//! Starting an agent from the phone (PRD home-device-rail D-24, D-25).
//!
//! The start sheet reads a catalog built from the core's snapshot: every
//! device's Home and checkouts as targets, the two agent kinds with the
//! models the provider catalog lists, and the choice the last start
//! remembered. A target travels as an id; the folder it names stays here in
//! `Catalog::routes` and never reaches the phone. `start_agent` resolves the
//! id, dispatches the same `agent_start_in_checkout` event the desktop sends
//! with `stay_behind`, so the desktop's screen does not follow it, and
//! answers with what the core's task slot or last error said about that
//! request id.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use serde::Serialize;
use serde_json::{Value, json};

/// The agent kinds a phone may start; `terminal` is a desktop-only start.
pub const KINDS: [&str; hide_agent_adapter::START_KINDS.len()] = hide_agent_adapter::START_KINDS;
/// How long a start may wait for the core's creation receipt: longer than a
/// device's Home sync (30 s) plus opening its tab, so a slow start is not
/// reported as lost while it still runs.
pub const ANSWER_LIMIT: Duration = Duration::from_secs(90);
/// How many start request ids per phone are remembered to refuse a repeat.
const REMEMBERED_STARTS: usize = 64;
/// How many answers the follower keeps for waiting requests.
const KEPT_ANSWERS: usize = 32;
/// The most targets one sheet lists.
const MAX_TARGETS: usize = 256;
const MAX_PROMPT_CHARS: usize = 4000;
const MAX_MODEL_CHARS: usize = 128;
const MAX_TARGET_CHARS: usize = 512;
/// How often a repeated request waits again for the first one's answer.
const REPEAT_POLL: Duration = Duration::from_millis(100);

/// One place a start can go, as the phone sees it.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Target {
    /// `home:<device_id>` for a device's Home, else the checkout's id.
    pub id: String,
    pub device_id: String,
    /// The place on the device (`project · branch`, or a folder's name);
    /// `None` is the device's Home, which the phone names in its language.
    pub place: Option<String>,
    /// The device's name; `None` is the core's own machine, which the phone
    /// names in its language from the catalog's `core_platform`.
    pub device_label: Option<String>,
    pub connected: bool,
}

/// Where a target id leads, kept on the Mac.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Route {
    /// `None` is this Mac.
    device_id: Option<String>,
    /// `None` is the device's Home.
    checkout_path: Option<String>,
}

/// What the start sheet reads; equal catalogs are not sent twice.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Catalog {
    targets: Vec<Target>,
    /// The core's machine's system as its device row says it (`macos
    /// aarch64`), from which the phone says This Mac, This PC or This
    /// computer in its own language.
    core_platform: Option<String>,
    kinds: Vec<Value>,
    remembered: Value,
    routes: HashMap<String, Route>,
}

fn str_of<'a>(value: &'a Value, field: &str) -> &'a str {
    value.get(field).and_then(Value::as_str).unwrap_or("")
}

fn array_of(value: Option<&Value>) -> &[Value] {
    value
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

impl Catalog {
    /// The catalog of a merged `rest` section; `node` is the core's own
    /// machine.
    pub fn of(rest: &Value, node: &str) -> Self {
        let mut catalog = Self {
            remembered: rest
                .pointer("/ui_state/agent_start")
                .filter(|choice| choice.is_object())
                .cloned()
                .unwrap_or_else(|| json!({"kind": null, "models": {}})),
            ..Self::default()
        };
        let devices = array_of(rest.pointer("/navigator/devices"));
        catalog.core_platform = devices
            .iter()
            .find(|device| str_of(device, "id") == node)
            .and_then(|device| device.pointer("/host/platform"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let statuses = array_of(rest.pointer("/status/remote"));
        let local_workspaces: Vec<&Value> = array_of(rest.pointer("/navigator/workspaces"))
            .iter()
            .filter(|workspace| workspace.get("remote_target_id").is_none_or(Value::is_null))
            .collect();
        catalog.add_device(node, None, true, true, &local_workspaces);
        for device in devices {
            let id = str_of(device, "id");
            if id == node || id.is_empty() || str_of(device, "kind") != "remote" {
                continue;
            }
            if str_of(device, "state") == "disabled" {
                continue;
            }
            let status = statuses
                .iter()
                .find(|status| str_of(status, "target_id") == id);
            let connected = status.is_some_and(|status| str_of(status, "state") == "connected");
            let workspaces: Vec<&Value> = array_of(
                status
                    .and_then(|status| status.get("session"))
                    .and_then(|session| session.get("workspaces")),
            )
            .iter()
            .collect();
            let label = Some(str_of(device, "label"))
                .filter(|label| !label.is_empty())
                .unwrap_or(id);
            catalog.add_device(id, Some(label), false, connected, &workspaces);
        }
        for kind in KINDS {
            let provider = array_of(rest.pointer("/status/background_ai/providers"))
                .iter()
                .find(|provider| str_of(provider, "id") == kind);
            let models: Vec<&str> = array_of(provider.and_then(|provider| provider.get("models")))
                .iter()
                .filter_map(Value::as_str)
                .collect();
            // The provider's own reason for an empty list can carry a path
            // or stderr, which never reaches a phone; an empty list is all
            // the sheet needs to offer the CLI default alone.
            catalog.kinds.push(json!({"id": kind, "models": models}));
        }
        catalog
    }

    fn add_device(
        &mut self,
        device_id: &str,
        device_label: Option<&str>,
        local: bool,
        connected: bool,
        workspaces: &[&Value],
    ) {
        let route_device = (!local).then(|| device_id.to_owned());
        let push = |catalog: &mut Self, id: String, place: Option<String>, path: Option<String>| {
            if catalog.targets.len() >= MAX_TARGETS {
                herdr_core::diagnostic!(json!({
                    "component": "mobile_phone", "kind": "start.targets_capped", "cap": MAX_TARGETS,
                }));
                return;
            }
            if catalog.routes.contains_key(&id) {
                return;
            }
            catalog.routes.insert(
                id.clone(),
                Route {
                    device_id: route_device.clone(),
                    checkout_path: path,
                },
            );
            catalog.targets.push(Target {
                id,
                device_id: device_id.to_owned(),
                place,
                device_label: device_label.map(str::to_owned),
                connected,
            });
        };
        push(self, home_id(device_id), None, None);
        for workspace in workspaces {
            if workspace.get("is_home").and_then(Value::as_bool) == Some(true) {
                continue;
            }
            let checkouts = array_of(workspace.get("checkouts"));
            let folder = workspace.get("is_git").and_then(Value::as_bool) != Some(true)
                && checkouts.len() == 1;
            for checkout in checkouts {
                let id = str_of(checkout, "id");
                let path = str_of(checkout, "path");
                if id.is_empty()
                    || path.is_empty()
                    || checkout.get("temporary").and_then(Value::as_bool) == Some(true)
                    || checkout.get("exists").and_then(Value::as_bool) == Some(false)
                {
                    continue;
                }
                let label = if folder {
                    str_of(workspace, "label").to_owned()
                } else {
                    let branch = checkout
                        .get("branch")
                        .and_then(Value::as_str)
                        .filter(|branch| !branch.is_empty())
                        .unwrap_or_else(|| str_of(checkout, "label"));
                    format!("{} · {branch}", str_of(workspace, "label"))
                };
                push(self, id.to_owned(), Some(label), Some(path.to_owned()));
            }
        }
    }

    /// The frame the phone's sheet reads.
    pub fn frame(&self) -> Value {
        json!({
            "type": "start_catalog",
            "targets": self.targets,
            "core_platform": self.core_platform,
            "kinds": self.kinds,
            "remembered": self.remembered,
        })
    }
}

fn home_id(device_id: &str) -> String {
    format!("home:{device_id}")
}

/// How the core answered one start request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Answer {
    Started { device_id: String, pane_id: String },
    Failed { reason: String },
}

/// The answers a snapshot carries: the task slot's finished agent start and
/// the last error, each with the request id it answers.
pub fn answers_of(rest: &Value, node: &str) -> Vec<(String, Answer)> {
    let mut answers = Vec::new();
    if let Some(operation) = rest.get("task_operation").filter(|op| op.is_object())
        && str_of(operation, "kind") == "agent_start"
    {
        let request_id = str_of(operation, "request_id");
        let answer = match (str_of(operation, "phase"), str_of(operation, "agent_phase")) {
            ("failed", _) => Some(Answer::Failed {
                reason: "start_failed".to_owned(),
            }),
            ("ready", "failed") => Some(Answer::Failed {
                reason: "agent_failed".to_owned(),
            }),
            ("ready", _) => Some(Answer::Started {
                device_id: Some(str_of(operation, "device_id"))
                    .filter(|device| !device.is_empty())
                    .unwrap_or(node)
                    .to_owned(),
                pane_id: str_of(operation, "pane_id").to_owned(),
            }),
            _ => None,
        };
        if let Some(answer) = answer.filter(|_| !request_id.is_empty()) {
            answers.push((request_id.to_owned(), answer));
        }
    }
    if let Some(error) = rest
        .pointer("/status/last_error")
        .filter(|error| error.is_object())
    {
        let request_id = str_of(error, "request_id");
        if !request_id.is_empty() {
            answers.push((
                request_id.to_owned(),
                Answer::Failed {
                    reason: str_of(error, "kind").to_owned(),
                },
            ));
        }
    }
    answers
}

/// Keeps the newest answers; true when `kept` changed.
pub fn record(kept: &mut Vec<(String, Answer)>, seen: Vec<(String, Answer)>) -> bool {
    let mut changed = false;
    for (request_id, answer) in seen {
        match kept.iter_mut().find(|(id, _)| *id == request_id) {
            Some((_, old)) if *old == answer => {}
            Some((_, old)) => {
                *old = answer;
                changed = true;
            }
            None => {
                if kept.len() >= KEPT_ANSWERS {
                    kept.remove(0);
                }
                kept.push((request_id, answer));
                changed = true;
            }
        }
    }
    changed
}

/// One start request, as the phone sent it.
pub struct Request<'a> {
    pub request_id: &'a str,
    pub text: &'a str,
    pub target: &'a str,
    pub kind: &'a str,
    pub model: Option<&'a str>,
}

fn prompt_problem(text: &str) -> Option<&'static str> {
    if text.trim().is_empty() {
        return Some("empty");
    }
    if text.chars().count() > MAX_PROMPT_CHARS {
        return Some("too_long");
    }
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
    {
        return Some("control_characters");
    }
    None
}

/// The core event for one request, or why it is not one.
fn event(request: &Request<'_>, catalog: &Catalog) -> Result<Value, &'static str> {
    if let Some(problem) = prompt_problem(request.text) {
        return Err(problem);
    }
    let kind = hide_agent_adapter::start_kind(request.kind).ok_or("unknown_kind")?;
    if request.target.chars().count() > MAX_TARGET_CHARS {
        return Err("unknown_target");
    }
    let Some(route) = catalog.routes.get(request.target) else {
        return Err("unknown_target");
    };
    if let Some(model) = request.model
        && (model.is_empty()
            || model.chars().count() > MAX_MODEL_CHARS
            || model.chars().any(char::is_control))
    {
        return Err("unknown_model");
    }
    let mut payload = json!({
        "provider": kind,
        "prompt": request.text,
        "request_id": request.request_id,
        // The phone is a separate surface: its start never moves the
        // desktop's visible tab or keyboard.
        "stay_behind": true,
    });
    if let Some(model) = request.model {
        payload["model"] = json!(model);
    }
    if let Some(device_id) = &route.device_id {
        payload["device_id"] = json!(device_id);
    }
    match &route.checkout_path {
        Some(path) => payload["checkout_path"] = json!(path),
        None => payload["home"] = json!(true),
    }
    Ok(json!({
        "schema_version": crate::state_file::SCHEMA_VERSION,
        "kind": "agent_start_in_checkout",
        "payload": payload,
    }))
}

/// Whether a request id is 1-64 characters of `[A-Za-z0-9_-]`.
pub fn request_id_valid(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The answers the follower keeps, newest last.
pub type Answers = std::sync::Arc<Vec<(String, Answer)>>;

/// Each phone's start request ids, with the answer each got once it had one.
type Seen = HashMap<String, VecDeque<(String, Option<Value>)>>;

/// The start request ids each phone sent, and the answer each got; a repeat
/// of an id is answered from here and never starts a second agent (B43).
#[derive(Default)]
pub struct Desk {
    seen: std::sync::Mutex<Seen>,
}

enum Claim {
    Fresh,
    Seen(Option<Value>),
}

impl Desk {
    fn lock(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.seen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn claim(&self, phone_id: &str, request_id: &str) -> Claim {
        let mut seen = self.lock();
        let ids = seen.entry(phone_id.to_owned()).or_default();
        if let Some((_, answer)) = ids.iter().find(|(id, _)| id == request_id) {
            return Claim::Seen(answer.clone());
        }
        if ids.len() >= REMEMBERED_STARTS {
            ids.pop_front();
        }
        ids.push_back((request_id.to_owned(), None));
        Claim::Fresh
    }

    /// Settles a claimed id with its answer; `None` forgets it, because
    /// nothing reached the core and the phone may send it again.
    fn settle(&self, phone_id: &str, request_id: &str, answer: Option<&Value>) {
        let mut seen = self.lock();
        let Some(ids) = seen.get_mut(phone_id) else {
            return;
        };
        match answer {
            Some(answer) => {
                if let Some(entry) = ids.iter_mut().find(|(id, _)| id == request_id) {
                    entry.1 = Some(answer.clone());
                }
            }
            None => ids.retain(|(id, _)| id != request_id),
        }
    }

    /// The start itself: resolve, claim, dispatch, then wait for the core's
    /// answer to this request id for at most `limit`.
    pub async fn start(
        &self,
        phone_id: &str,
        request: Request<'_>,
        catalog: &Catalog,
        answers: tokio::sync::watch::Receiver<Answers>,
        limit: Duration,
        dispatch: impl FnOnce(Vec<u8>) -> Result<(), String> + Send + 'static,
    ) -> Value {
        let request_id = request.request_id;
        let reply = |ok: bool, reason: Option<&str>| json!({"type": "start_result", "request_id": request_id, "ok": ok, "reason": reason});
        let event = match event(&request, catalog) {
            Ok(event) => event,
            Err(problem) => return reply(false, Some(problem)),
        };
        match self.claim(phone_id, request_id) {
            Claim::Fresh => {}
            // The first start may have landed after its wait ended: a repeat
            // waits for that answer again and never dispatches a second start.
            Claim::Seen(Some(answer)) if answer["reason"] == "timeout" => {
                return self.follow(phone_id, request_id, answers, limit).await;
            }
            Claim::Seen(Some(answer)) => return answer,
            Claim::Seen(None) => return self.repeat(phone_id, request_id, limit).await,
        }
        let bytes = event.to_string().into_bytes();
        let sent = tokio::task::spawn_blocking(move || dispatch(bytes))
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
        if let Err(message) = sent {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "start.dispatch_failed", "phone_id": phone_id, "message": message,
            }));
            self.settle(phone_id, request_id, None);
            return reply(false, Some("unavailable"));
        }
        self.follow(phone_id, request_id, answers, limit).await
    }

    /// Waits for the core's answer to `request_id` for at most `limit`, and
    /// settles the id with the frame the phone gets.
    async fn follow(
        &self,
        phone_id: &str,
        request_id: &str,
        mut answers: tokio::sync::watch::Receiver<Answers>,
        limit: Duration,
    ) -> Value {
        let reply = |ok: bool, reason: Option<&str>| json!({"type": "start_result", "request_id": request_id, "ok": ok, "reason": reason});
        let deadline = tokio::time::Instant::now() + limit;
        let answer = loop {
            let found = answers
                .borrow_and_update()
                .iter()
                .find(|(id, _)| id == request_id)
                .map(|(_, answer)| answer.clone());
            if let Some(found) = found {
                break found;
            }
            match tokio::time::timeout_at(deadline, answers.changed()).await {
                Ok(Ok(())) => {}
                // The follower ended, or the core never answered in time.
                Ok(Err(_)) | Err(_) => {
                    break Answer::Failed {
                        reason: "timeout".to_owned(),
                    };
                }
            }
        };
        let frame = match &answer {
            Answer::Started { device_id, pane_id } => {
                let mut frame = reply(true, None);
                frame["device_id"] = json!(device_id);
                frame["pane_id"] = json!(pane_id);
                frame
            }
            Answer::Failed { reason } => reply(false, Some(reason)),
        };
        herdr_core::diagnostic!(json!({
            "component": "mobile_phone", "kind": "start.answered", "phone_id": phone_id,
            "request_id": request_id, "ok": frame["ok"],
            "reason": frame["reason"],
        }));
        self.settle(phone_id, request_id, Some(&frame));
        frame
    }

    /// Where a claimed id stands, without claiming it.
    fn peek(&self, phone_id: &str, request_id: &str) -> Option<Option<Value>> {
        self.lock()
            .get(phone_id)?
            .iter()
            .find(|(id, _)| id == request_id)
            .map(|(_, answer)| answer.clone())
    }

    /// A repeat that arrives while the first is still waiting joins its answer.
    async fn repeat(&self, phone_id: &str, request_id: &str, limit: Duration) -> Value {
        let deadline = tokio::time::Instant::now() + limit + REPEAT_POLL;
        loop {
            match self.peek(phone_id, request_id) {
                Some(Some(answer)) => return answer,
                Some(None) if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(REPEAT_POLL).await;
                }
                // Forgotten (the first never reached the core) or still
                // pending past the limit: nothing more to wait for.
                _ => {
                    return json!({
                        "type": "start_result", "request_id": request_id, "ok": false, "reason": "in_flight",
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn rest() -> Value {
        json!({
            "navigator": {
                "devices": [
                    {"id": "local", "label": null, "kind": "local", "state": "local", "host": {"platform": "windows x86_64"}},
                    {"id": "mini", "label": "mini", "kind": "remote", "state": "ready"},
                    {"id": "off", "label": "off", "kind": "remote", "state": "disabled"},
                ],
                "workspaces": [
                    {"id": "w-home", "label": "hide", "is_home": true, "path": "/Users/example/hide",
                     "checkouts": [{"id": "c-home", "path": "/Users/example/hide", "label": "hide"}]},
                    {"id": "w1", "label": "hide", "is_git": true, "path": "/Users/example/projects/hide",
                     "checkouts": [
                        {"id": "c1", "path": "/Users/example/projects/hide", "label": "main", "branch": "main"},
                        {"id": "c2", "path": "/Users/example/projects/hide.worktrees/a", "label": "a", "branch": "prd/a"},
                        {"id": "c3", "path": "/tmp/gone", "label": "gone", "branch": "gone", "exists": false},
                     ]},
                    {"id": "w2", "label": "notes", "is_git": false, "path": "/Users/example/notes",
                     "checkouts": [{"id": "c4", "path": "/Users/example/notes", "label": "notes"}]},
                ],
            },
            "status": {
                "remote": [{"target_id": "mini", "state": "connected", "session": {"workspaces": [
                    {"id": "rw1", "label": "contong", "is_git": true, "path": "/srv/contong",
                     "checkouts": [{"id": "rc1", "path": "/srv/contong", "label": "main", "branch": "main"}]},
                ]}}],
                "background_ai": {"providers": [
                    {"id": "claude", "models": ["opus", "sonnet"], "models_unavailable_reason": null},
                    {"id": "codex", "models": [], "models_unavailable_reason": "codex app-server failed: /Users/example/.codex/config.toml"},
                ]},
            },
            "ui_state": {"agent_start": {"kind": "codex", "models": {"claude": "opus"}}},
        })
    }

    #[test]
    fn targets_are_this_mac_first_then_devices_with_home_leading_each() {
        let catalog = Catalog::of(&rest(), "local");
        let ids: Vec<_> = catalog.targets.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["home:local", "c1", "c2", "c4", "home:mini", "rc1"]);
        let places: Vec<_> = catalog.targets.iter().map(|t| t.place.as_deref()).collect();
        assert_eq!(
            places,
            [
                None,
                Some("hide · main"),
                Some("hide · prd/a"),
                Some("notes"),
                None,
                Some("contong · main")
            ]
        );
        assert_eq!(catalog.targets[0].device_label, None);
        assert_eq!(catalog.core_platform.as_deref(), Some("windows x86_64"));
        assert_eq!(catalog.frame()["core_platform"], "windows x86_64");
        assert_eq!(catalog.targets[4].device_label.as_deref(), Some("mini"));
        assert!(catalog.targets.iter().all(|target| target.connected));
    }

    #[test]
    fn a_disconnected_device_is_listed_unconnected() {
        let mut value = rest();
        value["status"]["remote"][0]["state"] = json!("unreachable");
        let catalog = Catalog::of(&value, "local");
        let mini = catalog
            .targets
            .iter()
            .find(|t| t.id == "home:mini")
            .unwrap();
        assert!(!mini.connected);
    }

    #[test]
    fn the_frame_never_carries_a_path() {
        let text = Catalog::of(&rest(), "local").frame().to_string();
        assert!(!text.contains("/Users"), "{text}");
        assert!(!text.contains("/srv"), "{text}");
        assert!(!text.contains("path"), "{text}");
    }

    #[test]
    fn kinds_carry_the_catalog_and_the_remembered_choice() {
        let frame = Catalog::of(&rest(), "local").frame();
        assert_eq!(
            frame["kinds"][0],
            json!({"id": "claude", "models": ["opus", "sonnet"]})
        );
        assert_eq!(frame["kinds"][1], json!({"id": "codex", "models": []}));
        assert!(!frame.to_string().contains("/Users/example"), "{frame}");
        assert_eq!(frame["remembered"]["kind"], "codex");
        let empty = Catalog::of(&json!({}), "local").frame();
        assert_eq!(empty["remembered"], json!({"kind": null, "models": {}}));
        assert_eq!(empty["kinds"][0]["models"], json!([]));
        assert_eq!(
            empty["targets"].as_array().unwrap().len(),
            1,
            "This Mac's Home is always a target"
        );
    }

    fn request<'a>(target: &'a str, model: Option<&'a str>) -> Request<'a> {
        Request {
            request_id: "r1",
            text: "테스트 고쳐줘\n그리고 PR",
            target,
            kind: "claude",
            model,
        }
    }

    #[test]
    fn a_checkout_target_resolves_to_its_path_and_device() {
        let catalog = Catalog::of(&rest(), "local");
        let local = event(&request("c2", Some("opus")), &catalog).unwrap();
        assert_eq!(local["kind"], "agent_start_in_checkout");
        assert_eq!(
            local["payload"],
            json!({
                "provider": "claude", "prompt": "테스트 고쳐줘\n그리고 PR", "request_id": "r1",
                "model": "opus", "checkout_path": "/Users/example/projects/hide.worktrees/a",
                "stay_behind": true,
            })
        );
        let remote = event(&request("rc1", None), &catalog).unwrap();
        assert_eq!(remote["payload"]["device_id"], "mini");
        assert_eq!(remote["payload"]["checkout_path"], "/srv/contong");
        assert!(remote["payload"].get("model").is_none());
    }

    #[test]
    fn home_targets_carry_home_and_no_folder() {
        let catalog = Catalog::of(&rest(), "local");
        let local = event(&request("home:local", None), &catalog).unwrap();
        assert_eq!(local["payload"]["home"], true);
        assert!(local["payload"].get("device_id").is_none());
        assert!(local["payload"].get("checkout_path").is_none());
        let remote = event(&request("home:mini", None), &catalog).unwrap();
        assert_eq!(remote["payload"]["device_id"], "mini");
    }

    #[test]
    fn a_request_the_core_should_never_see_is_refused_here() {
        let catalog = Catalog::of(&rest(), "local");
        let refuse = |request: Request<'_>| event(&request, &catalog).unwrap_err();
        assert_eq!(refuse(request("/etc", None)), "unknown_target");
        assert_eq!(refuse(request("home:gone", None)), "unknown_target");
        assert_eq!(
            refuse(Request {
                kind: "terminal",
                ..request("c1", None)
            }),
            "unknown_kind"
        );
        assert_eq!(
            refuse(Request {
                text: "  ",
                ..request("c1", None)
            }),
            "empty"
        );
        assert_eq!(
            refuse(Request {
                text: "a\u{1b}b",
                ..request("c1", None)
            }),
            "control_characters"
        );
        let long = "가".repeat(MAX_PROMPT_CHARS + 1);
        assert_eq!(
            refuse(Request {
                text: &long,
                ..request("c1", None)
            }),
            "too_long"
        );
        assert_eq!(refuse(request("c1", Some(""))), "unknown_model");
    }

    #[test]
    fn request_ids_are_one_to_sixty_four_safe_characters() {
        assert!(request_id_valid("phone-1_A"));
        assert!(!request_id_valid(""));
        assert!(!request_id_valid(&"a".repeat(65)));
        assert!(request_id_valid(&"a".repeat(64)));
        assert!(!request_id_valid("a b"));
        assert!(!request_id_valid("가"));
    }

    #[test]
    fn answers_come_from_the_task_slot_and_the_last_error() {
        let started = answers_of(
            &json!({"task_operation": {
                "kind": "agent_start", "phase": "ready", "request_id": "r1", "pane_id": "remote:mini:pane:1", "device_id": "mini",
            }}),
            "local",
        );
        assert_eq!(
            started,
            [(
                "r1".to_owned(),
                Answer::Started {
                    device_id: "mini".into(),
                    pane_id: "remote:mini:pane:1".into()
                }
            )]
        );
        let local = answers_of(
            &json!({"task_operation": {
                "kind": "agent_start", "phase": "ready", "request_id": "r2", "pane_id": "w1:p1", "device_id": null,
            }}),
            "local",
        );
        assert_eq!(
            local[0].1,
            Answer::Started {
                device_id: "local".into(),
                pane_id: "w1:p1".into()
            }
        );
        let working = answers_of(
            &json!({"task_operation": {"kind": "agent_start", "phase": "working", "request_id": "r3"}}),
            "local",
        );
        assert!(working.is_empty());
        let other = answers_of(
            &json!({"task_operation": {"kind": "worktree_create", "phase": "ready", "request_id": "r4"}}),
            "local",
        );
        assert!(other.is_empty());
        let refused = answers_of(
            &json!({"status": {"last_error": {"kind": "task_operation.busy", "request_id": "r5"}}}),
            "local",
        );
        assert_eq!(
            refused,
            [(
                "r5".to_owned(),
                Answer::Failed {
                    reason: "task_operation.busy".into()
                }
            )]
        );
        let unlabeled = answers_of(
            &json!({"status": {"last_error": {"kind": "x", "request_id": null}}}),
            "local",
        );
        assert!(unlabeled.is_empty());
    }

    #[test]
    fn recorded_answers_are_capped_and_replaced_by_id() {
        let mut kept = Vec::new();
        let failed = |reason: &str| Answer::Failed {
            reason: reason.into(),
        };
        assert!(record(&mut kept, vec![("a".into(), failed("x"))]));
        assert!(!record(&mut kept, vec![("a".into(), failed("x"))]));
        assert!(record(&mut kept, vec![("a".into(), failed("y"))]));
        for index in 0..KEPT_ANSWERS + 3 {
            record(&mut kept, vec![(format!("id{index}"), failed("x"))]);
        }
        assert_eq!(kept.len(), KEPT_ANSWERS);
        assert!(kept.iter().all(|(id, _)| id != "a"));
    }

    type AnswerChannel = (
        tokio::sync::watch::Sender<Answers>,
        tokio::sync::watch::Receiver<Answers>,
    );
    type Dispatch = Box<dyn FnOnce(Vec<u8>) -> Result<(), String> + Send>;

    fn channel() -> AnswerChannel {
        tokio::sync::watch::channel(Arc::new(Vec::new()))
    }

    fn counter() -> (Arc<AtomicUsize>, impl Fn() -> Dispatch) {
        let count = Arc::new(AtomicUsize::new(0));
        let make = {
            let count = Arc::clone(&count);
            move || {
                let count = Arc::clone(&count);
                Box::new(move |_: Vec<u8>| {
                    count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }) as Dispatch
            }
        };
        (count, make)
    }

    #[tokio::test]
    async fn a_repeated_request_id_starts_once_and_returns_the_first_answer() {
        let desk = Desk::default();
        let catalog = Catalog::of(&rest(), "local");
        let (sender, receiver) = channel();
        let (count, dispatch) = counter();
        sender.send_replace(Arc::new(vec![(
            "r1".to_owned(),
            Answer::Started {
                device_id: "local".into(),
                pane_id: "w1:p9".into(),
            },
        )]));
        let first = desk
            .start(
                "p",
                request("c1", None),
                &catalog,
                receiver.clone(),
                ANSWER_LIMIT,
                dispatch(),
            )
            .await;
        assert_eq!(first["ok"], true);
        assert_eq!(first["pane_id"], "w1:p9");
        let second = desk
            .start(
                "p",
                request("c1", None),
                &catalog,
                receiver.clone(),
                ANSWER_LIMIT,
                dispatch(),
            )
            .await;
        assert_eq!(second, first);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        // Another phone's identical id is its own request.
        desk.start(
            "q",
            request("c1", None),
            &catalog,
            receiver,
            ANSWER_LIMIT,
            dispatch(),
        )
        .await;
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_repeat_while_the_first_waits_joins_its_answer() {
        let desk = Arc::new(Desk::default());
        let catalog = Arc::new(Catalog::of(&rest(), "local"));
        let (sender, receiver) = channel();
        let (count, dispatch) = counter();
        let waiting = {
            let (desk, catalog, receiver, dispatch) = (
                Arc::clone(&desk),
                Arc::clone(&catalog),
                receiver.clone(),
                dispatch(),
            );
            tokio::spawn(async move {
                desk.start(
                    "p",
                    request("c1", None),
                    &catalog,
                    receiver,
                    ANSWER_LIMIT,
                    dispatch,
                )
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let repeat = {
            let (desk, catalog, receiver, dispatch) = (
                Arc::clone(&desk),
                Arc::clone(&catalog),
                receiver.clone(),
                dispatch(),
            );
            tokio::spawn(async move {
                desk.start(
                    "p",
                    request("c1", None),
                    &catalog,
                    receiver,
                    ANSWER_LIMIT,
                    dispatch,
                )
                .await
            })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        sender.send_replace(Arc::new(vec![(
            "r1".to_owned(),
            Answer::Failed {
                reason: "task_operation.busy".into(),
            },
        )]));
        let (first, repeat) = (waiting.await.unwrap(), repeat.await.unwrap());
        assert_eq!(first["reason"], "task_operation.busy");
        assert_eq!(repeat, first);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_silent_core_times_out_and_a_repeat_waits_for_the_late_answer() {
        let desk = Desk::default();
        let catalog = Catalog::of(&rest(), "local");
        let (sender, receiver) = channel();
        let (count, dispatch) = counter();
        let first = desk
            .start(
                "p",
                request("c1", None),
                &catalog,
                receiver.clone(),
                Duration::from_millis(30),
                dispatch(),
            )
            .await;
        assert_eq!(
            (first["ok"].clone(), first["reason"].clone()),
            (json!(false), json!("timeout"))
        );
        let second = desk
            .start(
                "p",
                request("c1", None),
                &catalog,
                receiver.clone(),
                Duration::from_millis(30),
                dispatch(),
            )
            .await;
        assert_eq!(second, first);
        // The first start lands late: a repeat now gets its answer, still
        // without a second dispatch.
        sender.send_replace(Arc::new(vec![(
            "r1".to_owned(),
            Answer::Started {
                device_id: "local".into(),
                pane_id: "w1:p9".into(),
            },
        )]));
        let third = desk
            .start(
                "p",
                request("c1", None),
                &catalog,
                receiver,
                Duration::from_millis(30),
                dispatch(),
            )
            .await;
        assert_eq!(third["ok"], true, "{third}");
        assert_eq!(third["pane_id"], "w1:p9");
        assert_eq!(
            count.load(Ordering::SeqCst),
            1,
            "the first may still land, so the id is never dispatched again"
        );
    }

    #[tokio::test]
    async fn a_dispatch_that_failed_frees_the_id_for_a_retry() {
        let desk = Desk::default();
        let catalog = Catalog::of(&rest(), "local");
        let (_sender, receiver) = channel();
        let failed = desk
            .start(
                "p",
                request("c1", None),
                &catalog,
                receiver.clone(),
                ANSWER_LIMIT,
                |_| Err("core owner thread is gone".into()),
            )
            .await;
        assert_eq!(failed["reason"], "unavailable");
        let (count, dispatch) = counter();
        let _ = desk
            .start(
                "p",
                request("c1", None),
                &catalog,
                receiver,
                Duration::from_millis(10),
                dispatch(),
            )
            .await;
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_invalid_request_dispatches_nothing() {
        let desk = Desk::default();
        let catalog = Catalog::of(&rest(), "local");
        let (_sender, receiver) = channel();
        let (count, dispatch) = counter();
        let answer = desk
            .start(
                "p",
                request("nowhere", None),
                &catalog,
                receiver,
                ANSWER_LIMIT,
                dispatch(),
            )
            .await;
        assert_eq!(answer["reason"], "unknown_target");
        assert_eq!(count.load(Ordering::SeqCst), 0);
    }
}
