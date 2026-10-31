//! Desk's worker: the one thread that owns the Desk process (engineering
//! practice: resident processes). Every ACP call, install and file check
//! runs here, off the runtime lock; what it learns reaches the runtime as a
//! [`Report`] under a short lock. The process ends when the worker's
//! channel closes (the runtime is gone), on `End`, when it crosses its cap,
//! and with hided itself, since it is started tied.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, Weak};

use hide_desk::AdapterId;
use hide_desk::adapter::{self, Paths};
use hide_desk::host::{Host, MAX_DESCENDANTS, MAX_RSS_BYTES};
use hide_desk::install::{self, InstallError};
use hide_desk::permission;
use hide_desk::rpc::{CallError, Incoming, Sink};
use hide_desk::session::{self, Choices, OpenError, Update};
use hide_platform::programs;
use serde_json::{Value, json};

use super::super::Runtime;
use crate::desk::{DeskCredential, DeskPlatform};
use crate::handle::ChangeNotifier;
use crate::model::DeskAgentState;

/// Messages from the agent waiting for the worker; past this, reply text is
/// dropped (and logged) rather than held, while requests and answers still
/// queue, since the agent waits on them.
const MAX_QUEUED_INCOMING: usize = 4096;
/// Tool calls remembered per session for the permission request that may
/// follow one; the oldest is forgotten first.
const MAX_ANNOUNCED: usize = 256;

pub(super) enum Order {
    /// Look at every adapter: installed, Node, its own CLI.
    Check,
    Install(AdapterId),
    Send(Send),
    /// Ends the turn under way.
    Cancel,
    /// Answers a permission request with `outcome`.
    Answer {
        rpc_id: Value,
        outcome: Value,
    },
    SetChoice {
        config_id: String,
        value: String,
    },
    /// Ends the Desk process; the next message starts a new one.
    End,
    Incoming(Incoming),
    Installed(AdapterId, Result<(), InstallError>),
    /// The runtime is gone: end the process and the worker.
    Shutdown,
}

pub(super) struct Send {
    pub tag: u64,
    pub adapter: AdapterId,
    pub text: String,
    /// The conversation so far, for a session that starts new.
    pub carried: Option<String>,
    pub resume: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Where Settings › Desk puts each task's work (B19).
    pub placement: hide_desk::Placement,
}

/// What the worker tells the runtime.
pub(super) enum Report {
    Availability(Vec<(AdapterId, DeskAgentState, Option<String>)>),
    Installing(AdapterId),
    Starting,
    Opened {
        adapter: AdapterId,
        session_id: String,
        resume_failed: bool,
        choices: Choices,
    },
    OpenFailed {
        adapter: AdapterId,
        error: OpenError,
    },
    Text {
        tag: u64,
        text: String,
    },
    /// What the turn's tool calls did, sent just before it ends.
    Steps {
        tag: u64,
        steps: Vec<hide_desk::permission::Step>,
    },
    TurnEnded {
        tag: u64,
        outcome: TurnOutcome,
    },
    /// A request Hide can read, for the runtime to answer from the table
    /// or put on a card.
    Permission {
        rpc_id: Value,
        allow: String,
        reject: Option<String>,
        reading: permission::Reading,
    },
    Blocked(String),
    Choices {
        adapter: AdapterId,
        choices: Choices,
    },
    /// The process ended: `unexpected` when nobody asked it to.
    Ended {
        unexpected: bool,
        detail: Option<String>,
    },
}

pub(super) enum TurnOutcome {
    Done,
    Cancelled,
    Failed(String),
}

pub(super) struct Handle {
    pub orders: Sender<Order>,
}

impl Drop for Handle {
    /// The worker holds a sender of its own for the agent's messages, so
    /// the channel never closes by itself: the runtime's end is said.
    fn drop(&mut self) {
        let _ = self.orders.send(Order::Shutdown);
    }
}

/// Starts the worker. It ends when every [`Handle`] is dropped.
pub(super) fn start(
    platform: Arc<dyn DeskPlatform>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
) -> std::io::Result<Handle> {
    let (orders, received) = mpsc::channel();
    let worker = Worker {
        platform,
        runtime,
        notifier,
        orders: orders.clone(),
        queued: Arc::new(AtomicUsize::new(0)),
        dropped_reported: Arc::new(AtomicBool::new(false)),
        stop: Arc::new(AtomicBool::new(false)),
        running: None,
        installing: Vec::new(),
    };
    std::thread::Builder::new()
        .name("herdr-core-desk".into())
        .spawn(move || worker.run(received))?;
    Ok(Handle { orders })
}

struct Worker {
    platform: Arc<dyn DeskPlatform>,
    runtime: Weak<Mutex<Runtime>>,
    notifier: ChangeNotifier,
    /// The worker's own sender, for the agent's messages and installs.
    orders: Sender<Order>,
    queued: Arc<AtomicUsize>,
    /// Reply text was dropped past [`MAX_QUEUED_INCOMING`] and logged once.
    dropped_reported: Arc<AtomicBool>,
    /// Set when the worker ends, so an install under way stops.
    stop: Arc<AtomicBool>,
    running: Option<Running>,
    installing: Vec<AdapterId>,
}

/// The Desk process and its session.
struct Running {
    host: Host,
    adapter: AdapterId,
    session_id: String,
    credential: DeskCredential,
    /// The tool calls the session announced, by id.
    announced: HashMap<String, Value>,
    announced_order: Vec<String>,
    turn: Option<u64>,
    /// The tool calls announced in the turn running now, in order.
    turn_calls: Vec<String>,
    /// The work placement this session was last told; none for a session
    /// resumed from an earlier run.
    placement: Option<hide_desk::Placement>,
    /// The startup text the adapter said it writes on its own, until it
    /// arrives; it may land inside the first turn and is not its reply.
    prelude: Option<String>,
}

impl Worker {
    fn run(mut self, received: Receiver<Order>) {
        while let Ok(order) = received.recv() {
            if matches!(order, Order::Shutdown) || self.runtime.strong_count() == 0 {
                break;
            }
            self.handle(order);
        }
        self.stop.store(true, Ordering::SeqCst);
        self.end_process(false);
    }

    fn handle(&mut self, order: Order) {
        match order {
            Order::Check => self.check(),
            Order::Install(adapter) => self.install(adapter),
            Order::Send(send) => self.send(send),
            Order::Cancel => {
                if let Some(running) = &self.running
                    && running.turn.is_some()
                {
                    let _ = session::cancel(running.host.connection(), &running.session_id);
                }
            }
            Order::Answer { rpc_id, outcome } => {
                if let Some(running) = &self.running {
                    let _ = running.host.connection().respond(&rpc_id, outcome);
                }
            }
            Order::SetChoice { config_id, value } => self.set_choice(&config_id, &value),
            Order::End => self.end_process(false),
            Order::Shutdown => {}
            Order::Incoming(incoming) => {
                self.queued.fetch_sub(1, Ordering::SeqCst);
                self.incoming(incoming);
            }
            Order::Installed(adapter, result) => {
                self.installing.retain(|installing| *installing != adapter);
                if let Err(error) = result {
                    crate::diagnostic!(json!({
                        "component": "desk",
                        "kind": "desk.install_failed",
                        "adapter": adapter.as_str(),
                        "error": format!("{error:?}"),
                    }));
                    self.report(Report::Availability(vec![install_state(adapter, &error)]));
                    return;
                }
                self.check();
            }
        }
    }

    fn report(&self, report: Report) {
        let Some(runtime) = self.runtime.upgrade() else {
            return;
        };
        let changed = match runtime.lock() {
            Ok(mut guard) => guard.desk_report(report),
            Err(poisoned) => poisoned.into_inner().desk_report(report),
        };
        drop(runtime);
        if changed {
            self.notifier.notify();
        }
    }

    fn check(&self) {
        let places = self.platform.places();
        let node = programs::find_cli("node");
        let node_state = match &node {
            None => Err(InstallError::NodeMissing),
            Some(node) => install::check_node(node),
        };
        let states = AdapterId::ALL
            .into_iter()
            .filter(|adapter| !self.installing.contains(adapter))
            .map(|adapter| {
                if let Err(error) = &node_state {
                    return install_state(adapter, error);
                }
                if let Some(cli) = adapter.own_cli()
                    && programs::find_cli(cli).is_none()
                {
                    return (adapter, DeskAgentState::CliMissing, Some(cli.to_owned()));
                }
                match install::installed(&places.adapters, adapter) {
                    Some(_) => (adapter, DeskAgentState::Ready, None),
                    None => (adapter, DeskAgentState::NotInstalled, None),
                }
            })
            .collect();
        self.report(Report::Availability(states));
    }

    fn install(&mut self, adapter: AdapterId) {
        if self.installing.contains(&adapter) {
            return;
        }
        let (Some(node), Some(npm)) = (programs::find_cli("node"), programs::find_cli("npm"))
        else {
            let error = if programs::find_cli("node").is_none() {
                InstallError::NodeMissing
            } else {
                InstallError::NpmMissing
            };
            self.report(Report::Availability(vec![install_state(adapter, &error)]));
            return;
        };
        if let Err(error) = install::check_node(&node) {
            self.report(Report::Availability(vec![install_state(adapter, &error)]));
            return;
        }
        let root = self.platform.places().adapters;
        let orders = self.orders.clone();
        let stop = Arc::clone(&self.stop);
        let spawned = std::thread::Builder::new()
            .name("herdr-core-desk-install".into())
            .spawn(move || {
                let result = install::install(&root, adapter, &npm, &stop).map(|_| ());
                let _ = orders.send(Order::Installed(adapter, result));
            });
        match spawned {
            Ok(_) => {
                self.installing.push(adapter);
                self.report(Report::Installing(adapter));
            }
            Err(error) => {
                let error = InstallError::Failed(error.to_string());
                self.report(Report::Availability(vec![install_state(adapter, &error)]));
            }
        }
    }

    fn send(&mut self, send: Send) {
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.adapter != send.adapter)
        {
            self.end_process(false);
        }
        let mut new_session = false;
        if self.running.is_none() {
            self.report(Report::Starting);
            match self.open(&send) {
                Ok((running, resume_failed, choices)) => {
                    new_session = send.resume.is_none() || resume_failed;
                    self.report(Report::Opened {
                        adapter: send.adapter,
                        session_id: running.session_id.clone(),
                        resume_failed,
                        choices,
                    });
                    self.running = Some(running);
                }
                Err(error) => {
                    self.report(Report::OpenFailed {
                        adapter: send.adapter,
                        error,
                    });
                    self.report(Report::TurnEnded {
                        tag: send.tag,
                        outcome: TurnOutcome::Cancelled,
                    });
                    return;
                }
            }
        }
        let Some(running) = &mut self.running else {
            return;
        };
        let text = if new_session {
            hide_desk::first_message(send.placement, send.carried.as_deref(), &send.text)
        } else {
            let changed = (running.placement != Some(send.placement)).then_some(send.placement);
            hide_desk::next_message(changed, &send.text)
        };
        running.placement = Some(send.placement);
        match session::prompt(
            running.host.connection(),
            &running.session_id,
            &text,
            send.tag,
        ) {
            Ok(()) => {
                running.turn = Some(send.tag);
                running.turn_calls.clear();
            }
            Err(error) => {
                self.report(Report::TurnEnded {
                    tag: send.tag,
                    outcome: TurnOutcome::Failed(error.to_string()),
                });
                if matches!(error, CallError::Closed(_)) {
                    self.end_process(true);
                }
            }
        }
    }

    /// Starts the Desk process for `send.adapter` and opens its session in
    /// the adapter's ask mode, then applies the chosen model and effort.
    fn open(&mut self, send: &Send) -> Result<(Running, bool, Choices), OpenError> {
        let adapter = send.adapter;
        let places = self.platform.places();
        let failed = |what: &str| OpenError::Failed(what.to_owned());
        let package = install::installed(&places.adapters, adapter)
            .ok_or_else(|| failed("the agent is not installed"))?;
        let node = programs::find_cli("node").ok_or_else(|| failed("node is missing"))?;
        let own_cli = match adapter.own_cli() {
            Some(cli) => Some(
                programs::find_cli(cli)
                    .ok_or_else(|| failed(&format!("{cli} is missing")))?
                    .into_path(),
            ),
            None => None,
        };
        let search = programs::account_path();
        let credential = self.platform.issue().map_err(OpenError::Failed)?;
        let launch = adapter::launch(
            adapter,
            &Paths {
                package: &package,
                node: node.path(),
                hide_dir: &places.hide_dir,
                search: search.as_ref(),
                desk: &places.desk,
                capability: &credential.reference,
                codex_home: &places.codex_home,
                own_cli: own_cli.as_deref(),
            },
            &credential.nonce,
        );
        let host = match Host::start(&launch, self.sink()) {
            Ok(host) => host,
            Err(error) => {
                self.platform.revoke(&credential);
                return Err(OpenError::Failed(format!(
                    "the process did not start: {error}"
                )));
            }
        };
        let close = |platform: &Arc<dyn DeskPlatform>, host: Host, error: OpenError| {
            let tail = host.stderr_tail();
            let _ = host.end();
            platform.revoke(&credential);
            match error {
                OpenError::Failed(detail) if !tail.is_empty() => {
                    OpenError::Failed(format!("{detail}: {tail}"))
                }
                other => other,
            }
        };
        if let Err(error) = self.platform.bind(&credential, host.pid(), host.started()) {
            return Err(close(&self.platform, host, OpenError::Failed(error)));
        }
        let marker = launch.pi_marker.clone();
        let host_pid = host.pid();
        let pi_ready = move || hide_desk::adapter::pi_extension_loaded(marker.as_deref(), host_pid);
        let work = adapter::work_dir(&places.desk);
        let opened = match session::open(
            host.connection(),
            adapter,
            &work,
            send.resume.as_deref(),
            &pi_ready,
        ) {
            Ok(opened) => opened,
            Err(error) => return Err(close(&self.platform, host, error)),
        };
        let mut choices = opened.choices.clone();
        for (choice, wanted) in [
            (opened.choices.model.as_ref(), send.model.as_deref()),
            (opened.choices.effort.as_ref(), send.effort.as_deref()),
        ] {
            if let (Some(choice), Some(wanted)) = (choice, wanted)
                && choice.current.as_deref() != Some(wanted)
                && choice.values.iter().any(|(id, _)| id == wanted)
                && let Ok(answer) = session::set_choice(
                    host.connection(),
                    &opened.session_id,
                    &choice.config_id,
                    wanted,
                )
            {
                merge_choices(&mut choices, answer);
            }
        }
        Ok((
            Running {
                host,
                adapter,
                session_id: opened.session_id,
                credential,
                announced: HashMap::new(),
                announced_order: Vec::new(),
                turn: None,
                turn_calls: Vec::new(),
                placement: None,
                prelude: opened.prelude,
            },
            opened.resume_failed,
            choices,
        ))
    }

    fn sink(&self) -> Sink {
        let orders = Mutex::new(self.orders.clone());
        let queued = Arc::clone(&self.queued);
        let reported = Arc::clone(&self.dropped_reported);
        Arc::new(move |incoming: Incoming| {
            if matches!(incoming, Incoming::Notification { .. })
                && queued.load(Ordering::SeqCst) >= MAX_QUEUED_INCOMING
            {
                if !reported.swap(true, Ordering::SeqCst) {
                    crate::diagnostic!(json!({
                        "component": "desk",
                        "kind": "desk.incoming_dropped",
                        "cap": MAX_QUEUED_INCOMING,
                    }));
                }
                return;
            }
            queued.fetch_add(1, Ordering::SeqCst);
            let sent = orders
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .send(Order::Incoming(incoming));
            if sent.is_err() {
                queued.fetch_sub(1, Ordering::SeqCst);
            }
        })
    }

    fn set_choice(&mut self, config_id: &str, value: &str) {
        let Some(running) = &self.running else {
            return;
        };
        match session::set_choice(
            running.host.connection(),
            &running.session_id,
            config_id,
            value,
        ) {
            Ok(choices) => self.report(Report::Choices {
                adapter: running.adapter,
                choices,
            }),
            Err(error) => crate::diagnostic!(json!({
                "component": "desk",
                "kind": "desk.choice_refused",
                "config": config_id,
                "error": error.to_string(),
            })),
        }
    }

    fn incoming(&mut self, incoming: Incoming) {
        let Some(running) = &mut self.running else {
            return;
        };
        match incoming {
            Incoming::Notification { method, params } if method == "session/update" => {
                match session::read_update(&params) {
                    // Pi's adapter writes startup lines on its own after the
                    // session opens, announced in its session answer.
                    Update::Text(text) if running.prelude.as_deref() == Some(text.as_str()) => {
                        running.prelude = None;
                    }
                    Update::Text(text) => {
                        if let Some(tag) = running.turn {
                            self.report(Report::Text { tag, text });
                        }
                    }
                    Update::Tool { id, call } => {
                        if running.turn.is_some()
                            && running.turn_calls.len() < MAX_ANNOUNCED
                            && !running.turn_calls.contains(&id)
                        {
                            running.turn_calls.push(id.clone());
                        }
                        running.remember(id, call);
                    }
                    Update::Choices(choices) => {
                        let adapter = running.adapter;
                        self.report(Report::Choices { adapter, choices });
                    }
                    Update::Mode(mode) => self.mode_moved(&mode),
                    Update::Other => {}
                }
            }
            Incoming::Notification { .. } => {}
            Incoming::Request { id, method, params } => {
                if method == "session/request_permission" {
                    self.permission(id, &params);
                } else {
                    let _ = running.host.connection().respond_error(
                        &id,
                        -32601,
                        "Desk offers no client methods",
                    );
                }
            }
            Incoming::Reply { tag, result } => {
                if running.turn == Some(tag) {
                    running.turn = None;
                    let steps = std::mem::take(&mut running.turn_calls)
                        .iter()
                        .filter_map(|id| running.announced.get(id))
                        .map(hide_desk::permission::step)
                        .collect();
                    self.report(Report::Steps { tag, steps });
                }
                let outcome = match result {
                    Ok(answer) => match session::stop_reason(&answer) {
                        Some("cancelled") => TurnOutcome::Cancelled,
                        Some("refusal") => TurnOutcome::Failed("the agent refused".into()),
                        _ => TurnOutcome::Done,
                    },
                    Err(error) => TurnOutcome::Failed(error.to_string()),
                };
                self.report(Report::TurnEnded { tag, outcome });
                self.check_cap();
            }
            Incoming::Closed { why } => {
                let tail = running.host.stderr_tail();
                let detail = if tail.is_empty() { why } else { tail };
                self.end_process_with(true, Some(detail));
            }
        }
    }

    /// The session left its ask mode: it is ended rather than used (D-53).
    fn mode_moved(&mut self, mode: &str) {
        let Some(running) = &self.running else {
            return;
        };
        if let adapter::AskMode::SetMode(ask) = running.adapter.ask_mode()
            && mode != ask
        {
            let adapter = running.adapter;
            self.end_process_with(true, Some(format!("the agent moved to mode {mode}")));
            self.report(Report::OpenFailed {
                adapter,
                error: OpenError::AskModeUnconfirmed(format!("moved to {mode}")),
            });
        }
    }

    /// Reads a permission request off the lock: one Hide cannot read, or
    /// answer with a one-time allowance, is refused here (D-60); the
    /// runtime answers every other one (D-82).
    fn permission(&mut self, rpc_id: Value, params: &Value) {
        let Some(running) = &self.running else {
            return;
        };
        let call_id = params
            .pointer("/toolCall/toolCallId")
            .and_then(Value::as_str);
        let announced = call_id.and_then(|id| running.announced.get(id));
        let request = permission::read_request(running.adapter, params, announced);
        let (Some(action), Some(allow)) = (permission::readable(&request), request.allow_once())
        else {
            let selected =
                |option: &str| json!({"outcome": {"outcome": "selected", "optionId": option}});
            let answer = request
                .reject()
                .map_or_else(|| json!({"outcome": {"outcome": "cancelled"}}), selected);
            let _ = running.host.connection().respond(&rpc_id, answer);
            let title = params
                .pointer("/toolCall/title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(400)
                .collect();
            self.report(Report::Blocked(title));
            return;
        };
        let places = self.platform.places();
        let hide = places.hide_dir.join(hide_program());
        let cwd = hide_platform::path::to_wire_lossy(&adapter::work_dir(&places.desk));
        let home = hide_platform::host::home_dir()
            .ok()
            .map(|home| hide_platform::path::to_wire_lossy(&home));
        let mut reading = permission::classify(
            action,
            &permission::Place {
                hide: &hide,
                cwd: &cwd,
                home: home.as_deref(),
            },
        );
        // Where a deleted path leads is read from the filesystem here, off
        // the lock, so the core judges the place the command would reach.
        permission::follow_links(&mut reading);
        self.report(Report::Permission {
            rpc_id,
            allow: allow.to_owned(),
            reject: request.reject().map(str::to_owned),
            reading,
        });
    }

    /// Measures the process tree after a turn; past its cap the process
    /// ends, which the room shows as stopped with `다시 시작`.
    fn check_cap(&mut self) {
        let Some(running) = &self.running else {
            return;
        };
        let Ok(measure) = running.host.measure() else {
            return;
        };
        if measure.descendants > MAX_DESCENDANTS || measure.rss_bytes > MAX_RSS_BYTES {
            crate::diagnostic!(json!({
                "component": "desk",
                "kind": "desk.cap_crossed",
                "descendants": measure.descendants,
                "rss_bytes": measure.rss_bytes,
            }));
            self.end_process_with(
                true,
                Some(format!(
                    "the agent left {} processes and {} MiB",
                    measure.descendants,
                    measure.rss_bytes / (1024 * 1024)
                )),
            );
        }
    }

    fn end_process(&mut self, unexpected: bool) {
        self.end_process_with(unexpected, None);
    }

    fn end_process_with(&mut self, unexpected: bool, detail: Option<String>) {
        let Some(running) = self.running.take() else {
            return;
        };
        let Running {
            host,
            credential,
            turn,
            ..
        } = running;
        self.platform.revoke(&credential);
        let ended = host.end();
        if let Err(error) = &ended {
            crate::diagnostic!(json!({
                "component": "desk",
                "kind": "desk.end_unconfirmed",
                "error": error.to_string(),
            }));
        }
        if let Some(tag) = turn {
            self.report(Report::TurnEnded {
                tag,
                outcome: TurnOutcome::Cancelled,
            });
        }
        self.report(Report::Ended { unexpected, detail });
    }
}

impl Running {
    fn remember(&mut self, id: String, call: Value) {
        if let Some(announced) = self.announced.get_mut(&id) {
            session::merge_tool(announced, &call);
            return;
        }
        if self.announced_order.len() >= MAX_ANNOUNCED {
            let oldest = self.announced_order.remove(0);
            self.announced.remove(&oldest);
        }
        self.announced_order.push(id.clone());
        self.announced.insert(id, call);
    }
}

fn merge_choices(choices: &mut Choices, answer: Choices) {
    if answer.model.is_some() {
        choices.model = answer.model;
    }
    if answer.effort.is_some() {
        choices.effort = answer.effort;
    }
}

fn install_state(
    adapter: AdapterId,
    error: &InstallError,
) -> (AdapterId, DeskAgentState, Option<String>) {
    match error {
        InstallError::NodeMissing => (adapter, DeskAgentState::NodeMissing, None),
        InstallError::NodeTooOld(version) => {
            (adapter, DeskAgentState::NodeMissing, Some(version.clone()))
        }
        InstallError::NpmMissing => (adapter, DeskAgentState::NodeMissing, Some("npm".into())),
        InstallError::Busy => (adapter, DeskAgentState::Preparing, None),
        InstallError::Failed(detail) => {
            (adapter, DeskAgentState::InstallFailed, Some(detail.clone()))
        }
    }
}

fn hide_program() -> &'static str {
    if cfg!(windows) { "hide.exe" } else { "hide" }
}
