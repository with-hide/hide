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
    /// Look at every agent (its adapter, Node, its own CLI) and install
    /// every adapter not in place (D-80).
    Check,
    /// Install `adapter`'s adapter; `asked` when the operator pressed
    /// `다시 받기`.
    Install {
        adapter: AdapterId,
        asked: bool,
    },
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
    /// Desk was turned off: ends the process and every install under way,
    /// whose staging folders go with them (engineering principle 14).
    Off,
    /// A message from the agent, numbered as it arrived
    /// ([`session::Arrivals`]).
    Incoming(u64, Incoming),
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
    /// Where Settings › Labs › Desk puts each task's work (B19).
    pub placement: hide_desk::Placement,
    /// The files `text` names (PRD desk-2-ui B41), each looked for on
    /// `node`, the core's own, before the agent is asked.
    pub files: Vec<String>,
    pub node: Option<Arc<dyn crate::node_access::NodeLink>>,
}

/// What the worker tells the runtime.
pub(super) enum Report {
    Availability(Vec<(AdapterId, DeskAgentState, Option<String>)>),
    Installing {
        adapter: AdapterId,
        asked: bool,
    },
    Starting,
    Opened {
        adapter: AdapterId,
        session_id: String,
        resume_failed: bool,
        choices: Choices,
    },
    /// The session did not open, or the agent refused it for its sign-in.
    /// `turn` is the turn whose message the agent never took, which ends
    /// with this report: no other report of the worker tells that turn's end.
    OpenFailed {
        adapter: AdapterId,
        error: OpenError,
        turn: Option<u64>,
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
    /// A file the message names is gone (Desk's folder dropped it past its
    /// cap), so the agent was not asked; the turn ends next.
    AttachmentsGone {
        tag: u64,
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
    /// The process ended: `unexpected` when nobody asked it to. `turn` is
    /// the turn that ends with it and how, told here and nowhere else, so
    /// no order between two reports decides whether the next queued message
    /// goes; a turn started since, whose message the worker has yet to send,
    /// is not this process's.
    Ended {
        unexpected: bool,
        detail: Option<String>,
        turn: Option<TurnEnd>,
    },
}

/// A turn's end that a process's end tells.
pub(super) struct TurnEnd {
    pub tag: u64,
    pub outcome: TurnOutcome,
}

pub(super) enum TurnOutcome {
    Done,
    Cancelled,
    /// The turn failed, with the agent's own one-line reason when it gave
    /// one (a provider's refusal, a model it cannot reach), which the
    /// operator can act on; the rest is in the diagnostic log.
    Failed(Option<String>),
    /// The process ended under it.
    Ended,
}

/// The longest reason a failed turn shows: one line of the agent's error.
const MAX_REASON_CHARS: usize = 200;

/// What a failed turn shows: the agent's own error message, first line,
/// when the agent answered with one. A call that got no answer, or a stream
/// that ended, names nothing the operator can act on (design rule 13).
fn reason(error: &CallError) -> Option<String> {
    let CallError::Rejected(rejected) = error else {
        return None;
    };
    first_line(&rejected.message)
}

/// The first line of the agent's own words with any text in it, cut to
/// [`MAX_REASON_CHARS`].
fn first_line(words: &str) -> Option<String> {
    let line = words.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(line.chars().take(MAX_REASON_CHARS).collect())
}

/// Whether the agent refused the call until the operator signs in: ACP's
/// `auth_required`, or any refusal from an agent that said it is signed out
/// ([`session::sign_in_status`]), whatever code it carries.
fn signs_in(error: &CallError, signed_out: bool) -> bool {
    matches!(error, CallError::Rejected(rejected)
        if rejected.code == session::AUTH_REQUIRED || signed_out)
}

/// An open the agent refused, as Desk reads it: one from an agent that
/// said it is signed out is a sign-in refusal (B6), whatever it answered.
fn open_refusal(error: OpenError, signed_out: bool) -> OpenError {
    match error {
        OpenError::Failed(_) if signed_out => OpenError::Login,
        other => other,
    }
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
        arrivals: session::Arrivals::default(),
        signed_out: Arc::new(AtomicBool::new(false)),
        dropped_reported: Arc::new(AtomicBool::new(false)),
        stop: Arc::new(AtomicBool::new(false)),
        on: false,
        running: None,
        installing: Vec::new(),
    };
    std::thread::Builder::new()
        .name("hide-core-desk".into())
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
    /// Numbers every message of every process this worker starts, so a
    /// turn reads only what arrived after its prompt went out.
    arrivals: session::Arrivals,
    /// The running agent said it is signed out ([`session::sign_in_status`]),
    /// so a call it refuses then is refused for the sign-in, whatever code
    /// the refusal carries.
    signed_out: Arc<AtomicBool>,
    /// Reply text was dropped past [`MAX_QUEUED_INCOMING`] and logged once.
    dropped_reported: Arc<AtomicBool>,
    /// Raised when Desk is turned off or the worker ends, so every install
    /// under way stops; turning Desk on again starts a new one.
    stop: Arc<AtomicBool>,
    /// Whether Desk is on, as the last `Check` or `Off` said: an install a
    /// quick off and on stopped is started again only while it is.
    on: bool,
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
    /// The running turn, its reply and its tool calls; it owns only what
    /// arrived after its prompt went out, so a `session/load`'s replay still
    /// waiting in the channel is no turn's.
    turn: session::TurnReader,
    /// The work placement this session was last told; none for a session
    /// resumed from an earlier run.
    placement: Option<hide_desk::Placement>,
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
            Order::Install { adapter, asked } => self.install(adapter, asked),
            Order::Send(send) => self.send(send),
            Order::Cancel => {
                if let Some(running) = &self.running
                    && running.turn.running().is_some()
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
            Order::Off => {
                self.on = false;
                self.stop.store(true, Ordering::SeqCst);
                self.stop = Arc::new(AtomicBool::new(false));
                self.end_process(false);
            }
            Order::Shutdown => {}
            Order::Incoming(number, incoming) => {
                self.queued.fetch_sub(1, Ordering::SeqCst);
                self.incoming(number, incoming);
            }
            Order::Installed(adapter, result) => {
                self.installing.retain(|installing| *installing != adapter);
                if let Err(InstallError::Stopped) = result {
                    // Desk was turned off and maybe on again before this
                    // install ended: the agent is looked at again while on.
                    if self.on {
                        self.install(adapter, false);
                    }
                    return;
                }
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
                self.report(Report::Availability(vec![(
                    adapter,
                    DeskAgentState::Ready,
                    None,
                )]));
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

    /// Looks at every agent and installs each adapter that is not in
    /// place, all at once (D-80): an agent is never left waiting for its
    /// adapter until it is chosen.
    fn check(&mut self) {
        self.on = true;
        let places = self.platform.places();
        let node = programs::find_cli("node");
        let node_state = match &node {
            None => Err(InstallError::NodeMissing),
            Some(node) => install::check_node(node),
        };
        let mut missing = Vec::new();
        let states = AdapterId::all()
            .filter(|adapter| !self.installing.contains(adapter))
            .filter_map(|adapter| {
                match look(
                    adapter,
                    &node_state,
                    &|cli| programs::find_cli(cli).is_some(),
                    &|adapter| install::installed(&places.adapters, adapter).is_some(),
                ) {
                    Look::State(state, detail) => Some((adapter, state, detail)),
                    Look::Install => {
                        missing.push(adapter);
                        None
                    }
                }
            })
            .collect();
        self.report(Report::Availability(states));
        for adapter in missing {
            self.install(adapter, false);
        }
    }

    fn install(&mut self, adapter: AdapterId, asked: bool) {
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
            .name("hide-core-desk-install".into())
            .spawn(move || {
                let result = install::install(&root, adapter, &npm, &stop).map(|_| ());
                let _ = orders.send(Order::Installed(adapter, result));
            });
        match spawned {
            Ok(_) => {
                self.installing.push(adapter);
                self.report(Report::Installing { adapter, asked });
            }
            Err(error) => {
                let error = InstallError::Failed(error.to_string());
                self.report(Report::Availability(vec![install_state(adapter, &error)]));
            }
        }
    }

    fn send(&mut self, send: Send) {
        if let Some(node) = &send.node
            && files_gone(node.as_ref(), &send.files)
        {
            self.report(Report::AttachmentsGone { tag: send.tag });
            self.report(Report::TurnEnded {
                tag: send.tag,
                outcome: TurnOutcome::Cancelled,
            });
            return;
        }
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
                        turn: Some(send.tag),
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
        let from = self.arrivals.next();
        match session::prompt(
            running.host.connection(),
            &running.session_id,
            &text,
            send.tag,
        ) {
            Ok(()) => running.turn.start(send.tag, from),
            Err(error) => {
                self.turn_failed(send.adapter, &error);
                let ended = TurnEnd {
                    tag: send.tag,
                    outcome: TurnOutcome::Failed(reason(&error)),
                };
                if matches!(error, CallError::Closed(_)) {
                    self.end_process_with(true, None, Some(ended));
                } else {
                    self.report(Report::TurnEnded {
                        tag: ended.tag,
                        outcome: ended.outcome,
                    });
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
        let package = match adapter.pin() {
            Some(_) => Some((
                install::installed(&places.adapters, adapter)
                    .ok_or_else(|| failed("the agent's adapter is not installed"))?,
                programs::find_cli("node")
                    .ok_or_else(|| failed("node is missing"))?
                    .into_path(),
            )),
            None => None,
        };
        let own_cli = match adapter.own_cli() {
            Some(cli) => Some(
                programs::find_cli(cli)
                    .ok_or_else(|| failed(&format!("{cli} is missing")))?
                    .into_path(),
            ),
            None => None,
        };
        let search = programs::account_path();
        // From the same login shell read as the search, kept until a
        // startup file changes, so a start asks no shell of its own.
        let account = programs::account_environment();
        let credential = self.platform.issue().map_err(OpenError::Failed)?;
        let launch = adapter::launch(
            adapter,
            &Paths {
                package: package
                    .as_ref()
                    .map(|(package, node)| (package.as_path(), node.as_path())),
                own_cli: own_cli.as_deref(),
                hide_dir: &places.hide_dir,
                search: search.as_ref(),
                account: account.as_deref(),
                desk: &places.desk,
                capability: &credential.reference,
                codex_home: &places.codex_home,
                codex_sign_in: places.codex_sign_in.as_deref().map_err(String::as_str),
            },
            &credential.nonce,
        );
        let launch = match launch {
            Ok(launch) => launch,
            Err(error) => {
                self.platform.revoke(&credential);
                return Err(OpenError::Failed(error));
            }
        };
        self.signed_out.store(false, Ordering::SeqCst);
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
            Err(error) => {
                let error = open_refusal(error, self.signed_out.load(Ordering::SeqCst));
                return Err(close(&self.platform, host, error));
            }
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
                turn: session::TurnReader::new(opened.prelude, adapter.failed_turn_mark()),
                placement: None,
            },
            opened.resume_failed,
            choices,
        ))
    }

    fn sink(&self) -> Sink {
        let orders = Mutex::new(self.orders.clone());
        let queued = Arc::clone(&self.queued);
        let reported = Arc::clone(&self.dropped_reported);
        let signed_out = Arc::clone(&self.signed_out);
        self.arrivals.sink(move |number, incoming: Incoming| {
            // Read here, on the connection's reader, so it is known before
            // the answer to the call it came ahead of.
            if let Incoming::Notification { method, params } = &incoming
                && let Some(signed_in) = session::sign_in_status(method, params)
            {
                signed_out.store(!signed_in, Ordering::SeqCst);
            }
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
                .send(Order::Incoming(number, incoming));
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

    fn incoming(&mut self, number: u64, incoming: Incoming) {
        let Some(running) = &mut self.running else {
            return;
        };
        match incoming {
            Incoming::Notification { method, params } if method == "session/update" => {
                let update = session::read_update(&params);
                let text = running.turn.read(number, &update);
                let adapter = running.adapter;
                match update {
                    Update::Tool { id, call } => running.remember(id, call),
                    Update::Choices(choices) => self.report(Report::Choices { adapter, choices }),
                    Update::Mode(mode) => self.mode_moved(&mode),
                    Update::Text { .. } | Update::Break | Update::Info(_) | Update::Other => {}
                }
                if let Some((tag, text)) = text {
                    self.report(Report::Text { tag, text });
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
                let adapter = running.adapter;
                let answered = running.turn.end(tag);
                let current = answered.is_some();
                let failed = answered
                    .as_ref()
                    .and_then(|answered| answered.failed.clone());
                if let Some(answered) = answered {
                    let steps = answered
                        .calls
                        .iter()
                        .filter_map(|id| running.announced.get(id))
                        .map(hide_desk::permission::step)
                        .collect();
                    self.report(Report::Steps { tag, steps });
                }
                let outcome = match result {
                    Ok(answer) => match (session::stop_reason(&answer), failed) {
                        (Some("cancelled"), _) => TurnOutcome::Cancelled,
                        // The adapter marked the turn failed and answered it
                        // as ended: what it wrote after the mark is its error.
                        (_, Some(words)) => {
                            let reason = first_line(&words);
                            self.turn_failed(
                                adapter,
                                &format!(
                                    "failed turn mark: {}",
                                    reason.as_deref().unwrap_or_default()
                                ),
                            );
                            TurnOutcome::Failed(reason)
                        }
                        (Some("refusal"), None) => {
                            crate::diagnostic!(json!({
                                "component": "desk",
                                "kind": "desk.turn_refused",
                                "adapter": adapter.as_str(),
                            }));
                            TurnOutcome::Failed(None)
                        }
                        _ => TurnOutcome::Done,
                    },
                    Err(error)
                        if current && signs_in(&error, self.signed_out.load(Ordering::SeqCst)) =>
                    {
                        self.turn_failed(adapter, &error);
                        self.sign_in_needed(adapter, tag);
                        return;
                    }
                    Err(error) => {
                        self.turn_failed(adapter, &error);
                        TurnOutcome::Failed(reason(&error))
                    }
                };
                let past_cap = self.past_cap();
                self.turn_ended(tag, outcome, past_cap);
            }
            Incoming::Closed { why } => {
                let tail = running.host.stderr_tail();
                let detail = if tail.is_empty() { why } else { tail };
                self.end_process_with(true, Some(detail), None);
            }
        }
    }

    /// Records a failed turn with what the adapter last wrote to its error
    /// output, which the screen never shows.
    fn turn_failed(&self, adapter: AdapterId, error: &dyn std::fmt::Display) {
        let tail = self
            .running
            .as_ref()
            .map(|running| running.host.stderr_tail())
            .unwrap_or_default();
        crate::diagnostic!(json!({
            "component": "desk",
            "kind": "desk.turn_failed",
            "adapter": adapter.as_str(),
            "error": error.to_string(),
            "stderr_tail": tail,
        }));
    }

    /// The agent refused a turn until the operator signs in, as a provider
    /// with no key does (B6). The turn ends as one the session's open
    /// refused: its message goes back to the operator's input and the
    /// sign-in line shows. The process ends with it, since an agent reads its
    /// sign-in and its environment as it starts; the next message starts
    /// one that reads them again.
    fn sign_in_needed(&mut self, adapter: AdapterId, tag: u64) {
        self.report(Report::OpenFailed {
            adapter,
            error: OpenError::Login,
            turn: Some(tag),
        });
        self.end_process(false);
    }

    /// The session left its ask mode: it is ended rather than used (D-53).
    fn mode_moved(&mut self, mode: &str) {
        let Some(running) = &self.running else {
            return;
        };
        if let Some(adapter::AskMode::SetMode(ask)) = running.adapter.ask_mode()
            && mode != ask
        {
            let adapter = running.adapter;
            self.end_process_with(true, Some(format!("the agent moved to mode {mode}")), None);
            // The agent took the turn's message, so it is not given back:
            // the process's end ended that turn.
            self.report(Report::OpenFailed {
                adapter,
                error: OpenError::AskModeUnconfirmed(format!("moved to {mode}")),
                turn: None,
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

    /// Measures the process tree after a turn: past its cap, why, for the
    /// stopped line.
    fn past_cap(&self) -> Option<String> {
        let measure = self.running.as_ref()?.host.measure().ok()?;
        if measure.descendants <= MAX_DESCENDANTS && measure.rss_bytes <= MAX_RSS_BYTES {
            return None;
        }
        crate::diagnostic!(json!({
            "component": "desk",
            "kind": "desk.cap_crossed",
            "descendants": measure.descendants,
            "rss_bytes": measure.rss_bytes,
        }));
        Some(format!(
            "the agent left {} processes and {} MiB",
            measure.descendants,
            measure.rss_bytes / (1024 * 1024)
        ))
    }

    /// A turn the agent answered ends. Past its cap the process ends with
    /// it, in one report, so the room shows it stopped with `다시 시작`
    /// and the next queued message waits for that rather than start
    /// another process.
    fn turn_ended(&mut self, tag: u64, outcome: TurnOutcome, past_cap: Option<String>) {
        match past_cap {
            Some(detail) => {
                self.end_process_with(true, Some(detail), Some(TurnEnd { tag, outcome }))
            }
            None => self.report(Report::TurnEnded { tag, outcome }),
        }
    }

    fn end_process(&mut self, unexpected: bool) {
        self.end_process_with(unexpected, None, None);
    }

    /// Ends the process. `last` is the turn that ends with it when the
    /// caller knows how it ended; otherwise the turn it ran ends as the
    /// process's.
    fn end_process_with(
        &mut self,
        unexpected: bool,
        detail: Option<String>,
        last: Option<TurnEnd>,
    ) {
        let Some(running) = self.running.take() else {
            return;
        };
        let Running {
            host,
            credential,
            turn,
            ..
        } = running;
        let turn = last.or_else(|| {
            turn.running().map(|tag| TurnEnd {
                tag,
                outcome: TurnOutcome::Ended,
            })
        });
        self.platform.revoke(&credential);
        let ended = host.end();
        if let Err(error) = &ended {
            crate::diagnostic!(json!({
                "component": "desk",
                "kind": "desk.end_unconfirmed",
                "error": error.to_string(),
            }));
        }
        self.report(Report::Ended {
            unexpected,
            detail,
            turn,
        });
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

/// What looking at one agent found: its state, or an adapter to install.
#[derive(Debug, PartialEq)]
enum Look {
    State(DeskAgentState, Option<String>),
    Install,
}

/// Whether the node that holds Desk's files says one of `files` is gone,
/// which Desk's folder does past its cap (`hided/src/attachments.rs`). A
/// node that does not answer proves nothing, and the message goes.
fn files_gone(node: &dyn crate::node_access::NodeLink, files: &[String]) -> bool {
    let paths: Vec<String> = files
        .iter()
        .map(|file| hide_platform::path::to_wire_lossy(std::path::Path::new(file)))
        .collect();
    match crate::node_access::call_as::<hide_node_link::catalog::PathFacts>(
        node,
        hide_node_link::protocol::Call::PathFacts {
            paths: paths.clone(),
        },
        std::time::Duration::from_secs(5),
    ) {
        Ok(facts) => paths
            .iter()
            .any(|path| facts.paths.get(path).is_none_or(|fact| !fact.exists)),
        Err(error) => {
            crate::diagnostic!(json!({
                "component": "desk",
                "kind": "desk.attachments_unchecked",
                "error": error.to_string(),
            }));
            false
        }
    }
}

/// Looks at `adapter` with the account's Node (`node`), whether a CLI is on
/// the account's `PATH` and whether an adapter is installed.
fn look(
    adapter: AdapterId,
    node: &Result<(), InstallError>,
    cli_found: &dyn Fn(&str) -> bool,
    installed: &dyn Fn(AdapterId) -> bool,
) -> Look {
    if adapter.ask_mode().is_none() {
        return Look::State(DeskAgentState::NoAskMode, None);
    }
    if let Some(cli) = adapter.own_cli()
        && !cli_found(cli)
    {
        return Look::State(DeskAgentState::CliMissing, Some(cli.to_owned()));
    }
    if adapter.pin().is_none() || installed(adapter) {
        return Look::State(DeskAgentState::Ready, None);
    }
    match node {
        Err(error) => {
            let (_, state, detail) = install_state(adapter, error);
            Look::State(state, detail)
        }
        Ok(()) => Look::Install,
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
        // Reported only while Desk is on, whose next look starts it again.
        InstallError::Stopped => (adapter, DeskAgentState::Preparing, None),
    }
}

fn hide_program() -> &'static str {
    if cfg!(windows) { "hide.exe" } else { "hide" }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::*;
    use crate::desk::DeskPlaces;
    use crate::model::DeskHostState;
    use crate::model::{DeskItem, DeskNotice};
    use crate::runtime::desk::tests::{event, items, opened, runtime, sends, sent};

    /// The variable that puts this test binary in a waiting process's role.
    const IDLE: &str = "HIDE_CORE_DESK_IDLE_PROCESS";

    /// A process that only reads its input until it closes: this binary run
    /// again with [`IDLE`], so a worker test ends a real process. Run as a
    /// test without it, it returns at once.
    #[test]
    fn idle_process() {
        if std::env::var_os(IDLE).is_some() {
            let _ = std::io::copy(&mut std::io::stdin(), &mut std::io::sink());
        }
    }

    struct NoCapability;

    impl DeskPlatform for NoCapability {
        fn issue(&self) -> Result<DeskCredential, String> {
            Err("not in this test".into())
        }
        fn bind(&self, _: &DeskCredential, _: u32, _: u64) -> Result<(), String> {
            Ok(())
        }
        fn revoke(&self, _: &DeskCredential) {}
        fn places(&self) -> DeskPlaces {
            DeskPlaces {
                adapters: PathBuf::new(),
                desk: PathBuf::new(),
                codex_home: PathBuf::new(),
                codex_sign_in: Err("not in this test".into()),
                hide_dir: PathBuf::new(),
            }
        }
    }

    /// A worker whose process is the idle one, speaking for `adapter`,
    /// running the turn `tag`, and reporting to `runtime`.
    fn worker_running(
        runtime: &Arc<Mutex<Runtime>>,
        cwd: &std::path::Path,
        adapter: AdapterId,
        tag: u64,
    ) -> Worker {
        let launch = adapter::Launch {
            program: std::env::current_exe().unwrap(),
            args: [
                "--exact",
                "runtime::desk::worker::tests::idle_process",
                "-q",
                "--test-threads=1",
            ]
            .map(OsString::from)
            .to_vec(),
            env: vec![(IDLE.into(), "1".into())],
            cwd: cwd.to_owned(),
            files: Vec::new(),
            links: Vec::new(),
            pi_marker: None,
        };
        let arrivals = session::Arrivals::default();
        let host = Host::start(&launch, arrivals.sink(|_, _| {})).unwrap();
        let mut turn = session::TurnReader::new(None, adapter.failed_turn_mark());
        turn.start(tag, arrivals.next());
        let (orders, _) = mpsc::channel();
        Worker {
            platform: Arc::new(NoCapability),
            runtime: Arc::downgrade(runtime),
            notifier: ChangeNotifier::new(),
            orders,
            queued: Arc::new(AtomicUsize::new(0)),
            arrivals,
            signed_out: Arc::new(AtomicBool::new(false)),
            dropped_reported: Arc::new(AtomicBool::new(false)),
            stop: Arc::new(AtomicBool::new(false)),
            on: true,
            running: Some(Running {
                host,
                adapter,
                session_id: "s1".into(),
                credential: DeskCredential {
                    reference: PathBuf::new(),
                    nonce: String::new(),
                },
                announced: HashMap::new(),
                announced_order: Vec::new(),
                turn,
                placement: None,
            }),
            installing: Vec::new(),
        }
    }

    /// A runtime with a turn under way on `adapter` and one message behind
    /// it.
    fn a_turn_and_one_waiting(
        root: &std::path::Path,
        adapter: AdapterId,
    ) -> (Arc<Mutex<Runtime>>, Receiver<Order>, u64) {
        let (mut runtime, received) = runtime(root);
        event(
            &mut runtime,
            "desk_select",
            json!({"agent": adapter.as_str()}),
        );
        event(&mut runtime, "desk_send", json!({"text": "one"}));
        let one = sent(&received);
        opened(&mut runtime, adapter);
        event(&mut runtime, "desk_send", json!({"text": "two"}));
        (Arc::new(Mutex::new(runtime)), received, one.tag)
    }

    fn after(runtime: &Arc<Mutex<Runtime>>) -> (bool, usize, Option<DeskHostState>) {
        let runtime = runtime.lock().unwrap();
        (
            runtime.desk.turn.is_some(),
            runtime.desk.stored.queued.len(),
            runtime.desk.host.state,
        )
    }

    /// S2, from the worker: a process that dies under a turn tells that
    /// turn's end with its own, so the message behind it waits for
    /// `다시 시작` rather than go to a process that is ending.
    #[test]
    fn a_process_that_dies_under_a_turn_sends_no_queued_message() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, received, tag) = a_turn_and_one_waiting(root.path(), AdapterId::CLAUDE);
        let mut worker = worker_running(&runtime, root.path(), AdapterId::CLAUDE, tag);
        worker.incoming(
            u64::MAX,
            Incoming::Closed {
                why: "the agent exited".into(),
            },
        );
        assert!(worker.running.is_none());
        assert_eq!(sends(&received), Vec::<String>::new());
        assert_eq!(after(&runtime), (false, 1, Some(DeskHostState::Stopped)));
    }

    /// S2, from the worker: a turn the agent answered past its cap ends
    /// with the process, in one report, so the message behind it waits for
    /// `다시 시작`; under the cap it goes at once.
    #[test]
    fn a_turn_past_the_cap_ends_with_its_process_and_sends_no_queued_message() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, received, tag) = a_turn_and_one_waiting(root.path(), AdapterId::CLAUDE);
        let mut worker = worker_running(&runtime, root.path(), AdapterId::CLAUDE, tag);
        worker.running.as_mut().unwrap().turn.end(tag);
        worker.turn_ended(
            tag,
            TurnOutcome::Done,
            Some("the agent left 60 processes".into()),
        );
        assert!(worker.running.is_none());
        assert_eq!(sends(&received), Vec::<String>::new());
        assert_eq!(after(&runtime), (false, 1, Some(DeskHostState::Stopped)));

        let root = tempfile::tempdir().unwrap();
        let (runtime, received, tag) = a_turn_and_one_waiting(root.path(), AdapterId::CLAUDE);
        let mut worker = worker_running(&runtime, root.path(), AdapterId::CLAUDE, tag);
        worker.running.as_mut().unwrap().turn.end(tag);
        worker.turn_ended(tag, TurnOutcome::Done, None);
        assert_eq!(sends(&received), ["two"]);
        worker.end_process(false);
    }

    /// A node that answers `Call::PathFacts` with `exists` for each path, or
    /// does not answer.
    struct Facts(Option<Vec<(&'static str, bool)>>);

    impl crate::node_access::NodeLink for Facts {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            _: std::time::Duration,
        ) -> Result<crate::node_access::LinkAnswer, crate::node_access::LinkError> {
            assert!(matches!(
                call,
                hide_node_link::protocol::Call::PathFacts { .. }
            ));
            let Some(facts) = &self.0 else {
                return Err(crate::node_access::LinkError::Unknown("timed out".into()));
            };
            let paths: serde_json::Map<String, Value> = facts
                .iter()
                .map(|(path, exists)| {
                    (
                        (*path).to_owned(),
                        json!({"comparison": path, "exists": exists}),
                    )
                })
                .collect();
            Ok(json!({"paths": paths, "repositories": {}}).into())
        }
    }

    #[test]
    fn a_desk_file_the_node_says_is_gone_stops_the_message_and_a_silent_node_does_not() {
        let files = [
            "/state/desk-attachments/a.png".to_owned(),
            "/state/desk-attachments/b.log".to_owned(),
        ];
        let both = Facts(Some(vec![
            ("/state/desk-attachments/a.png", true),
            ("/state/desk-attachments/b.log", true),
        ]));
        assert!(!files_gone(&both, &files));
        let one_gone = Facts(Some(vec![
            ("/state/desk-attachments/a.png", true),
            ("/state/desk-attachments/b.log", false),
        ]));
        assert!(files_gone(&one_gone, &files));
        assert!(!files_gone(&Facts(None), &files));
    }

    /// letter-47671, from the worker: a Codex turn whose model failed
    /// (429 here, as codex-acp 2.5.0 sent it) is answered `end_turn`, and
    /// still ends as a failed turn with Codex's words as its reason and no
    /// reply text; the message behind it goes next, as after any turn.
    #[test]
    fn a_turn_its_adapter_marked_failed_ends_failed_and_the_next_message_goes() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, received, tag) = a_turn_and_one_waiting(root.path(), AdapterId::CODEX);
        let mut worker = worker_running(&runtime, root.path(), AdapterId::CODEX, tag);
        let update = |update: Value| Incoming::Notification {
            method: "session/update".into(),
            params: json!({"sessionId": "s1", "update": update}),
        };
        worker.incoming(
            100,
            update(json!({"sessionUpdate": "session_info_update",
                "_meta": {"codex": {"threadStatus": {"type": "systemError"}}}})),
        );
        worker.incoming(
            101,
            update(
                json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text",
                "text": "exceeded retry limit, last status: 429 Too Many Requests\n\n"}}),
            ),
        );
        worker.incoming(
            102,
            Incoming::Reply {
                tag,
                result: Ok(json!({"stopReason": "end_turn", "usage": null,
                    "_meta": {"quota": {"token_count": null, "model_usage": []}}})),
            },
        );
        assert_eq!(sends(&received), ["two"]);
        let items = items(&runtime.lock().unwrap());
        assert!(
            !items
                .iter()
                .any(|item| matches!(item, DeskItem::Reply { .. })),
            "the error became a reply: {items:?}"
        );
        assert!(items.iter().any(|item| matches!(item,
            DeskItem::Notice { notice: DeskNotice::TurnFailed, detail: Some(detail), .. }
                if detail == "exceeded retry limit, last status: 429 Too Many Requests")));
        worker.end_process(false);
    }

    /// A turn the operator stopped reads as stopped even when its adapter
    /// marked it failed on the way out: `cancelled` comes before the mark,
    /// so no failed line shows for a turn the operator ended.
    #[test]
    fn a_cancelled_turn_its_adapter_marked_failed_reads_as_stopped() {
        let root = tempfile::tempdir().unwrap();
        let (runtime, received, tag) = a_turn_and_one_waiting(root.path(), AdapterId::CODEX);
        let mut worker = worker_running(&runtime, root.path(), AdapterId::CODEX, tag);
        worker.incoming(
            100,
            Incoming::Notification {
                method: "session/update".into(),
                params: json!({"sessionId": "s1", "update": {"sessionUpdate": "session_info_update",
                    "_meta": {"codex": {"threadStatus": {"type": "systemError"}}}}}),
            },
        );
        worker.incoming(
            101,
            Incoming::Reply {
                tag,
                result: Ok(json!({"stopReason": "cancelled"})),
            },
        );
        assert_eq!(sends(&received), ["two"]);
        let items = items(&runtime.lock().unwrap());
        assert!(
            !items.iter().any(|item| matches!(
                item,
                DeskItem::Notice {
                    notice: DeskNotice::TurnFailed,
                    ..
                }
            )),
            "a stopped turn read as failed: {items:?}"
        );
        worker.end_process(false);
    }

    fn looked(
        node: Result<(), InstallError>,
        missing_cli: &[&str],
        installed: &[AdapterId],
    ) -> Vec<(&'static str, Look)> {
        AdapterId::all()
            .map(|adapter| {
                (
                    adapter.as_str(),
                    look(
                        adapter,
                        &node,
                        &|cli| !missing_cli.contains(&cli),
                        &|adapter| installed.contains(&adapter),
                    ),
                )
            })
            .collect()
    }

    #[test]
    fn a_failed_turn_names_the_agent_s_own_first_line_and_nothing_hide_s() {
        let rejected = |message: &str| {
            CallError::Rejected(hide_desk::rpc::RpcError {
                code: -32603,
                message: message.into(),
            })
        };
        assert_eq!(
            reason(&rejected("Rate limit reached\n  at provider.ts:12")),
            Some("Rate limit reached".into())
        );
        assert_eq!(
            reason(&rejected(&"x".repeat(500))).map(|line| line.chars().count()),
            Some(MAX_REASON_CHARS)
        );
        assert_eq!(reason(&rejected("  ")), None);
        for nothing_to_act_on in [
            CallError::Unanswered,
            CallError::Busy,
            CallError::Closed("eof".into()),
        ] {
            assert_eq!(reason(&nothing_to_act_on), None);
        }
    }

    /// letter-47232: a refusal is for the sign-in when it is ACP's
    /// `auth_required`, or any refusal of an agent that said it is signed
    /// out; an open it refused so reads as the sign-in line, not a stop.
    #[test]
    fn a_refusal_after_the_agent_said_it_is_signed_out_is_a_sign_in_refusal() {
        let rejected = |code: i64| {
            CallError::Rejected(hide_desk::rpc::RpcError {
                code,
                message: "Internal error".into(),
            })
        };
        assert!(signs_in(&rejected(session::AUTH_REQUIRED), false));
        assert!(!signs_in(&rejected(-32603), false));
        assert!(signs_in(&rejected(-32603), true));
        assert!(!signs_in(&CallError::Closed("eof".into()), true));
        assert_eq!(
            open_refusal(
                OpenError::Failed("session/new: rejected -32603".into()),
                true
            ),
            OpenError::Login
        );
        assert_eq!(
            open_refusal(OpenError::Failed("x".into()), false),
            OpenError::Failed("x".into())
        );
        assert_eq!(
            open_refusal(OpenError::AskModeUnconfirmed("m".into()), true),
            OpenError::AskModeUnconfirmed("m".into())
        );
    }

    #[test]
    fn desk_turned_on_installs_every_adapter_not_in_place_at_once() {
        let ready = Look::State(DeskAgentState::Ready, None);
        assert_eq!(
            looked(Ok(()), &[], &[]),
            [
                ("claude", Look::Install),
                ("codex", Look::Install),
                ("grok", Look::State(DeskAgentState::NoAskMode, None)),
                ("opencode", ready),
                ("pi", Look::Install),
                ("omp", Look::State(DeskAgentState::Ready, None)),
                ("agy", Look::State(DeskAgentState::Ready, None)),
            ]
        );
        // An installed adapter is ready without the network.
        assert_eq!(
            looked(Err(InstallError::NodeMissing), &[], &[AdapterId::CLAUDE])[0],
            ("claude", Look::State(DeskAgentState::Ready, None))
        );
    }

    #[test]
    fn an_agent_whose_own_cli_or_node_is_missing_says_which() {
        let looked = looked(Err(InstallError::NodeMissing), &["omp", "pi"], &[]);
        assert_eq!(
            looked[1],
            ("codex", Look::State(DeskAgentState::NodeMissing, None))
        );
        assert_eq!(
            looked[4],
            (
                "pi",
                Look::State(DeskAgentState::CliMissing, Some("pi".into()))
            )
        );
        assert_eq!(
            looked[5],
            (
                "omp",
                Look::State(DeskAgentState::CliMissing, Some("omp".into()))
            )
        );
    }
}
