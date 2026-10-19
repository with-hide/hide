//! Mobile: this daemon on the operator's phone (PRD mobile-companion).
//!
//! The daemon keeps listening on loopback only; the transport module
//! (`tailscale.rs`) publishes that port inside the operator's tailnet while
//! Settings > Mobile is on. A phone pairs with a five-minute code from the QR,
//! holds a credential of its own, and talks only over `/ws` (`phone.rs`): it
//! sees the agent list (`projection.rs`), reads and answers one pane
//! (`pane.rs`), reads its agent's conversation (`conversation.rs`), and
//! receives Web Push (`push.rs`). The core knows nothing of
//! phones; this module reads its snapshot like any other client.
//!
//! What the renderer sees is one `mobile` frame, republished on every
//! change; what it asks is one of the `mobile_*` events.

pub mod conversation;
pub mod pane;
pub mod phone;
pub mod phones;
pub mod projection;
pub mod push;
pub mod start;
pub mod store;
pub mod tailscale;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use tokio::sync::{Notify, watch};

use crate::core::CoreHandle;
use herdr_core::delivery::worker::{HumanNotice, HumanNoticeKind};
use phones::{PairRefusal, Phones};
use projection::{AgentKey, Projection};
use store::{MobileSettings, Notifications, PhoneRecord, PushMode, PushSubscription, ServeRecord};
use tailscale::{Checklist, CliSource, CommandFailure, Ownership, StepState};

fn projection_failed(stage: &str, message: &str) {
    eprintln!(
        "{}",
        json!({"component": "hided", "kind": "mobile.projection_failed", "stage": stage, "message": message})
    );
}

/// How often the checklist is read again while Settings > Mobile is open.
const OBSERVE_INTERVAL: Duration = Duration::from_secs(3);
/// How often the seven-day sweep runs.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// How often Mobile is reconciled with no one watching Settings > Mobile:
/// while on, to expose once Tailscale is ready and to notice what changed
/// under an exposed entry (a Funnel); while off, to finish a failed removal.
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(60);
/// How many input request ids per phone are remembered to refuse a repeat.
const REMEMBERED_INPUTS: usize = 64;
/// Live connections one phone may hold; a newer one closes the oldest.
const CONNECTIONS_PER_PHONE: usize = 2;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// Where the Mac stands with the phone.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Exposure {
    Off,
    Checking,
    /// A checklist step fails; the checklist says which.
    Blocked,
    /// Someone else serves port 443; hide leaves it alone.
    Foreign {
        target: String,
    },
    /// A serve command failed.
    Failed {
        step: &'static str,
        message: String,
    },
    Exposed {
        dns_name: String,
    },
}

struct Inner {
    settings: MobileSettings,
    phones: Phones,
    exposure: Exposure,
    checklist: Option<Checklist>,
    /// Renderer connections with Settings > Mobile open.
    observers: HashSet<u64>,
    /// Tags of agents made Seen since each phone's last push (PRD D-21).
    pending_clear: HashMap<String, BTreeSet<AgentKey>>,
    /// Input request ids each phone sent, and how each ended (PRD B27).
    inputs: HashMap<String, VecDeque<(String, InputState)>>,
    /// An exposed Mobile's last pass could not read Tailscale; one such miss
    /// keeps the exposure, the next one in a row drops it.
    missed_check: bool,
}

/// Where one input request id stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputState {
    /// Being written now; a repeat must not write it again.
    InFlight,
    /// Herdr took it.
    Written,
    /// Herdr did not answer in time: it may or may not have landed, so a
    /// repeat of this id is never written again.
    Uncertain,
}

/// What a phone's input request id allows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reservation {
    /// A new id: write it now.
    Write,
    /// Seen before, in this state: answer from it without writing.
    Seen(InputState),
}

/// A phone on a live connection.
struct LivePhone {
    phone_id: String,
    viewing: Option<AgentKey>,
    close: Arc<Notify>,
    /// When it registered, so a phone's newest connection wins.
    since: u64,
}

/// What every phone loop is told besides the agent list.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PhoneMeta {
    pub push_mode: PushMode,
    pub live_phones: usize,
}

pub struct Config {
    pub state_dir: PathBuf,
    /// Where this Mac's agents write their transcripts.
    pub home: PathBuf,
    pub port: u16,
    pub cli: CliSource,
    pub host_name: Option<String>,
    pub core: Arc<CoreHandle>,
    /// Asked at every write to a pane, so a reply never lands in the shell
    /// under an agent that is ending, asleep or waking.
    pub input_gate: Arc<herdr_core::input_gate::PaneInputGate>,
    pub herdr_socket: Option<PathBuf>,
    /// Shell windows (desktop and web) on this daemon: "the app is open".
    pub renderers: Arc<AtomicUsize>,
    /// Which connections show a start surface; a phone's open start sheet is one.
    pub start_demand: Arc<crate::demand::ObservationDemand>,
    /// Whether the core may act outside this machine: a core on a move's
    /// copy serves no phone and pushes nothing until the move's link
    /// commits it (PRD core-host-node-move amendment 3).
    pub open: watch::Receiver<bool>,
    /// The desktop windows' notifications, judged on the projection this
    /// follower reads (`desktop_notices.rs`).
    pub desktop: Arc<crate::desktop_notices::DesktopNotices>,
}

pub struct Mobile {
    config: Config,
    inner: Mutex<Inner>,
    /// One reconcile at a time, so a double toggle or two startup passes
    /// converge on one serve entry.
    reconcile_lock: tokio::sync::Mutex<()>,
    origin: RwLock<Option<String>>,
    frame: watch::Sender<Arc<Value>>,
    projection: watch::Sender<Arc<Projection>>,
    /// What the start sheet lists, republished when it changes.
    catalog: watch::Sender<Arc<start::Catalog>>,
    /// The core's answers to start requests, for the requests waiting on them.
    answers: watch::Sender<start::Answers>,
    starts: start::Desk,
    meta: watch::Sender<PhoneMeta>,
    live: Mutex<HashMap<u64, LivePhone>>,
    vapid: Option<push::Vapid>,
    wake: Notify,
    stopping: watch::Sender<bool>,
}

fn record_transport_failure(failure: &CommandFailure, step: &str) {
    herdr_core::diagnostic!(json!({
        "component": "mobile_transport",
        "kind": "serve.failed",
        "step": step,
        "command": failure.command,
        "exit_code": failure.exit_code,
        "stderr": failure.stderr.chars().take(2000).collect::<String>(),
    }));
}

impl Mobile {
    /// The core's own machine, whose agents the navigator lists.
    pub fn node(&self) -> &herdr_core::node::NodeId {
        self.config.core.node()
    }

    pub fn start(config: Config) -> Arc<Self> {
        let mut settings: MobileSettings = store::read(&store::settings_path(&config.state_dir));
        let vapid = match settings
            .vapid_pkcs8
            .as_deref()
            .and_then(|stored| URL_SAFE_NO_PAD.decode(stored).ok())
            .map(|bytes| push::Vapid::from_pkcs8(&bytes))
        {
            Some(Ok(vapid)) => Some(vapid),
            _ => match push::Vapid::generate() {
                Ok((vapid, pkcs8)) => {
                    settings.vapid_pkcs8 = Some(URL_SAFE_NO_PAD.encode(pkcs8));
                    store::write_logged(&store::settings_path(&config.state_dir), &settings);
                    Some(vapid)
                }
                Err(message) => {
                    herdr_core::diagnostic!(json!({
                        "component": "mobile_push", "kind": "vapid.unavailable", "message": message,
                    }));
                    None
                }
            },
        };
        let phones = Phones::load(
            store::phones_path(&config.state_dir),
            settings.serve.as_ref().map(|serve| serve.dns_name.as_str()),
        );
        let exposure = if settings.enabled {
            Exposure::Checking
        } else {
            Exposure::Off
        };
        let push_mode = settings.push_mode;
        let mobile = Arc::new(Self {
            inner: Mutex::new(Inner {
                settings,
                phones,
                exposure,
                checklist: None,
                observers: HashSet::new(),
                pending_clear: HashMap::new(),
                inputs: HashMap::new(),
                missed_check: false,
            }),
            config,
            reconcile_lock: tokio::sync::Mutex::new(()),
            origin: RwLock::new(None),
            frame: watch::channel(Arc::new(Value::Null)).0,
            projection: watch::channel(Arc::new(Projection::default())).0,
            catalog: watch::channel(Arc::new(start::Catalog::default())).0,
            answers: watch::channel(Arc::new(Vec::new())).0,
            starts: start::Desk::default(),
            meta: watch::channel(PhoneMeta {
                push_mode,
                live_phones: 0,
            })
            .0,
            live: Mutex::new(HashMap::new()),
            vapid,
            wake: Notify::new(),
            stopping: watch::channel(false).0,
        });
        mobile.sweep();
        mobile.publish();
        let startup = Arc::clone(&mobile);
        tokio::spawn(async move {
            let mut open = startup.config.open.clone();
            let mut stopping = startup.stopping.subscribe();
            tokio::select! {
                opened = open.wait_for(|open| *open) => if opened.is_err() {
                    return;
                },
                _ = stopping.wait_for(|stopping| *stopping) => return,
            }
            tokio::spawn(Arc::clone(&startup).observe_loop());
            tokio::spawn(Arc::clone(&startup).follow());
            startup.reconcile().await;
        });
        tokio::spawn(Arc::clone(&mobile).sweep_loop());
        mobile
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn live(&self) -> std::sync::MutexGuard<'_, HashMap<u64, LivePhone>> {
        self.live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn save_settings(&self, inner: &Inner) {
        store::write_logged(
            &store::settings_path(&self.config.state_dir),
            &inner.settings,
        );
    }

    /// The mac's name on the phone: its tailnet name, else the system's.
    pub fn mac_name(&self) -> Option<String> {
        let inner = self.lock();
        inner
            .checklist
            .as_ref()
            .and_then(|checklist| checklist.host_name.clone())
            .or_else(|| self.config.host_name.clone())
    }

    /// Whether a WebSocket Origin is this Mac's tailnet address while it is
    /// exposed. Only phone handshakes are accepted from it.
    pub fn origin_allowed(&self, origin: Option<&str>) -> bool {
        let Some(origin) = origin else {
            return false;
        };
        self.origin
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_deref()
            == Some(origin)
    }

    /// Mobile on with a phone paired keeps the daemon from its idle exit (D-10).
    pub fn keep_alive(&self) -> bool {
        let inner = self.lock();
        inner.settings.enabled && !inner.phones.is_empty()
    }

    /// The follower reads the core for the phones while Mobile is on with a
    /// phone paired, and for the desktop windows while one has notifications
    /// on, so the two share one snapshot read per burst.
    fn follower_active(&self) -> bool {
        self.keep_alive() || self.config.desktop.wanted()
    }

    /// A desktop window turned its notifications on: the follower starts if
    /// it was resting, or seeds the windows' transitions on its next pass.
    pub fn wake_follower(&self) {
        self.wake.notify_one();
    }

    pub fn subscribe_frame(&self) -> watch::Receiver<Arc<Value>> {
        self.frame.subscribe()
    }

    pub fn subscribe_projection(&self) -> watch::Receiver<Arc<Projection>> {
        self.projection.subscribe()
    }

    pub fn subscribe_catalog(&self) -> watch::Receiver<Arc<start::Catalog>> {
        self.catalog.subscribe()
    }

    pub fn subscribe_meta(&self) -> watch::Receiver<PhoneMeta> {
        self.meta.subscribe()
    }

    pub fn vapid_public_key(&self) -> Option<String> {
        self.vapid.as_ref().map(push::Vapid::public_key)
    }

    /// Rebuilds the renderer frame and the phones' meta from current state.
    /// Both are sent under the lock, so two publishes racing each other
    /// never leave the older state as the last one sent.
    fn publish(&self) {
        let inner = self.lock();
        self.frame.send_replace(Arc::new(self.frame_value(&inner)));
        let push_mode = inner.settings.push_mode;
        let live_phones = {
            let live = self.live();
            live.values()
                .map(|phone| phone.phone_id.clone())
                .collect::<HashSet<_>>()
                .len()
        };
        self.meta.send_if_modified(|meta| {
            let next = PhoneMeta {
                push_mode,
                live_phones,
            };
            if *meta == next {
                false
            } else {
                *meta = next;
                true
            }
        });
    }

    fn frame_value(&self, inner: &Inner) -> Value {
        let step = |state: StepState| match state {
            StepState::Ok => "ok",
            StepState::Failed => "failed",
            StepState::Waiting => "waiting",
        };
        let checklist = inner.checklist.as_ref().map(|checklist| {
            json!({
                "installed": step(checklist.installed),
                "logged_in": step(checklist.logged_in),
                "https": step(checklist.https),
                "host_name": checklist.host_name,
            })
        });
        let (exposure, foreign, failure, dns) = match &inner.exposure {
            Exposure::Off => ("off", None, None, None),
            Exposure::Checking => ("checking", None, None, None),
            Exposure::Blocked => ("blocked", None, None, None),
            Exposure::Foreign { target } => ("foreign", Some(target.clone()), None, None),
            Exposure::Failed { step, message } => (
                "failed",
                None,
                Some(json!({"step": step, "message": message})),
                None,
            ),
            Exposure::Exposed { dns_name } => ("exposed", None, None, Some(dns_name.clone())),
        };
        let code = dns.as_ref().and(inner.phones.code());
        let qr = match (&dns, code) {
            (Some(dns), Some((code, _))) => {
                let endpoint = format!("https://{dns}");
                let pair = URL_SAFE_NO_PAD
                    .encode(json!({"v": 1, "endpoint": endpoint, "code": code}).to_string());
                Some(format!("{endpoint}/m/#pair={pair}"))
            }
            _ => None,
        };
        let live: HashSet<String> = self
            .live()
            .values()
            .map(|phone| phone.phone_id.clone())
            .collect();
        let phones: Vec<Value> = inner
            .phones
            .current()
            .map(|phone| {
                json!({
                    "id": phone.id,
                    "name": phone.name,
                    "last_seen_ms": phone.last_seen_ms,
                    "connected": live.contains(&phone.id),
                    "notifications": match (phone.notifications, phone.push.is_some()) {
                        (_, true) => "on",
                        (Notifications::Off, false) => "off",
                        _ => "unasked",
                    },
                    "revoke_at_ms": phone.last_seen_ms + phones::INACTIVE_REVOKE_MS,
                })
            })
            .collect();
        json!({
            "enabled": inner.settings.enabled,
            "exposure": exposure,
            "checklist": checklist,
            "download_url": tailscale::DOWNLOAD_URL,
            "admin_url": tailscale::ADMIN_DNS_URL,
            "foreign_target": foreign,
            "failure": failure,
            "url": dns.as_ref().map(|dns| format!("https://{dns}")),
            "qr": qr,
            "code_expires_at_ms": code.map(|(_, expires)| expires),
            "phones": phones,
            "max_phones": phones::MAX_PHONES,
            "push_mode": inner.settings.push_mode.as_str(),
            "now_ms": now_ms(),
        })
    }

    fn set_origin(&self, dns_name: Option<&str>) {
        if let Some(dns_name) = dns_name {
            self.lock().phones.set_origin(dns_name);
        }
        *self
            .origin
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            dns_name.map(|dns| format!("https://{dns}"));
    }

    /// Brings the serve entry in line with the switch (PRD D-06, D-07):
    /// on, the Mac's three checks pass and port 443 holds hide's own entry
    /// for this port; off, hide's recorded entry is gone. Runs one at a time.
    pub async fn reconcile(&self) {
        let _running = self.reconcile_lock.lock().await;
        // Once the daemon is stopping, shutdown owns the serve entry; a
        // pending core touches none.
        if *self.stopping.borrow() || !*self.config.open.borrow() {
            return;
        }
        let (enabled, record) = {
            let inner = self.lock();
            (inner.settings.enabled, inner.settings.serve.clone())
        };
        let program = self.config.cli.resolve();
        if !enabled {
            self.set_origin(None);
            self.close_phones("mobile_off");
            let removal = match (record, program.as_deref()) {
                (Some(record), Some(program)) => self.remove_recorded(program, &record).await,
                (Some(_), None) => Err((
                    "remove",
                    "the tailscale CLI is gone; hide's serve entry may remain".to_owned(),
                )),
                (None, _) => Ok(()),
            };
            let mut inner = self.lock();
            if !inner.settings.enabled {
                // A removal that failed stays on screen while the switch is
                // off, and the background pass tries it again (PRD B6).
                inner.exposure = match removal {
                    Ok(()) => Exposure::Off,
                    Err((step, message)) => Exposure::Failed { step, message },
                };
                inner.phones.clear_code();
            }
            drop(inner);
            self.publish();
            return;
        }
        let Some(program) = program else {
            self.settle(Exposure::Blocked, Some(Checklist::not_installed()));
            return;
        };
        let status = match tailscale::status(&program).await {
            Ok(status) => status,
            Err(failure) => {
                record_transport_failure(&failure, "status");
                if self.tolerate_missed_check() {
                    return;
                }
                self.set_origin(None);
                // A CLI that cannot answer reads as not logged in: the
                // Tailscale app is not running or not signed in.
                self.settle(
                    Exposure::Blocked,
                    Some(Checklist::from_status(&tailscale::Status::default())),
                );
                return;
            }
        };
        let checklist = Checklist::from_status(&status);
        let Some(dns_name) = checklist.dns_name.clone().filter(|_| checklist.passed()) else {
            self.set_origin(None);
            self.settle(Exposure::Blocked, Some(checklist));
            return;
        };
        let port = self.config.port;
        let exposure = match self
            .expose(&program, &dns_name, record.as_ref(), port)
            .await
        {
            Ok(()) => Exposure::Exposed {
                dns_name: dns_name.clone(),
            },
            Err(exposure) => exposure,
        };
        if matches!(exposure, Exposure::Failed { step: "check", .. })
            && self.tolerate_missed_check()
        {
            return;
        }
        let exposed = matches!(exposure, Exposure::Exposed { .. });
        self.set_origin(exposed.then_some(dns_name.as_str()));
        if matches!(exposure, Exposure::Failed { step: "funnel", .. }) {
            // Funnel publishes whatever serves this name to the internet:
            // hide's own entry comes down and every phone is closed until
            // the operator turns Funnel off (PRD D-16).
            self.close_phones("mobile_off");
            let record = self.lock().settings.serve.clone();
            if let Some(record) = record
                && let Err((step, message)) = self.remove_recorded(&program, &record).await
            {
                herdr_core::diagnostic!(json!({
                    "component": "mobile_transport", "kind": "serve.funnel_withdraw_failed",
                    "step": step, "message": message,
                }));
            }
        }
        {
            let mut inner = self.lock();
            // A code is minted only by `mobile_show_code`; a pass here only
            // lets an expired one go, so the frame never offers a QR no
            // phone can pair with.
            if inner
                .phones
                .code()
                .is_some_and(|(_, expires)| expires <= now_ms())
            {
                inner.phones.clear_code();
            }
        }
        self.settle(exposure, Some(checklist));
    }

    /// A read of Tailscale failed: an exposed Mobile stays exposed through
    /// one such miss, so a single slow `tailscale` call does not refuse
    /// phones until the next pass. True when this miss is tolerated.
    fn tolerate_missed_check(&self) -> bool {
        let mut inner = self.lock();
        let exposed = matches!(inner.exposure, Exposure::Exposed { .. });
        let tolerated = exposed && !inner.missed_check;
        inner.missed_check = tolerated;
        tolerated
    }

    fn settle(&self, exposure: Exposure, checklist: Option<Checklist>) {
        {
            let mut inner = self.lock();
            if !inner.settings.enabled {
                return;
            }
            if !matches!(exposure, Exposure::Exposed { .. }) {
                inner.phones.clear_code();
            }
            // A pass that reached an answer ends any run of missed checks.
            inner.missed_check = false;
            inner.exposure = exposure;
            inner.checklist = checklist;
        }
        self.publish();
        self.wake.notify_one();
    }

    fn set_record(&self, record: Option<ServeRecord>) {
        let mut inner = self.lock();
        inner.settings.serve = record;
        self.save_settings(&inner);
    }

    /// Removes hide's recorded entry; the error names the step and why.
    async fn remove_recorded(
        &self,
        program: &std::path::Path,
        record: &ServeRecord,
    ) -> Result<(), (&'static str, String)> {
        let serve = match tailscale::serve_status(program).await {
            Ok(serve) => serve,
            Err(failure) => {
                record_transport_failure(&failure, "check");
                return Err(("check", failure.message()));
            }
        };
        match tailscale::ownership(&serve, &record.dns_name, Some(record)) {
            Ok(Ownership::Ours { .. }) | Ok(Ownership::Foreign { ours_too: true, .. }) => {
                match tailscale::serve_remove(program).await {
                    Ok(()) => {
                        herdr_core::diagnostic!(json!({
                            "component": "mobile_transport", "kind": "serve.removed",
                            "dns_name": record.dns_name, "port": record.port,
                        }));
                        self.set_record(None);
                        Ok(())
                    }
                    Err(failure) => {
                        record_transport_failure(&failure, "remove");
                        Err(("remove", failure.message()))
                    }
                }
            }
            // Gone already, or someone else's now: nothing of hide's is left.
            Ok(_) => {
                self.set_record(None);
                Ok(())
            }
            Err(message) => {
                herdr_core::diagnostic!(json!({
                    "component": "mobile_transport", "kind": "serve.unreadable", "message": message,
                }));
                Err(("check", message))
            }
        }
    }

    /// Adds, keeps or replaces hide's entry, then confirms it with a fresh
    /// `serve status` read before the QR may show.
    async fn expose(
        &self,
        program: &std::path::Path,
        dns_name: &str,
        record: Option<&ServeRecord>,
        port: u16,
    ) -> Result<(), Exposure> {
        let failed = |step: &'static str, failure: &CommandFailure| {
            record_transport_failure(failure, step);
            Exposure::Failed {
                step,
                message: failure.message(),
            }
        };
        let funnel = |serve: &Value| {
            let on = tailscale::funnel_on(serve, dns_name);
            if on {
                herdr_core::diagnostic!(json!({
                    "component": "mobile_transport", "kind": "serve.funnel_on", "dns_name": dns_name,
                }));
            }
            on
        };
        let serve = tailscale::serve_status(program)
            .await
            .map_err(|failure| failed("check", &failure))?;
        if funnel(&serve) {
            return Err(Exposure::Failed {
                step: "funnel",
                message: tailscale::FUNNEL_MESSAGE.to_owned(),
            });
        }
        let owned =
            tailscale::ownership(&serve, dns_name, record).map_err(|message| Exposure::Failed {
                step: "check",
                message,
            })?;
        match owned {
            Ownership::Foreign { target, ours_too } => {
                if ours_too {
                    match tailscale::serve_remove(program).await {
                        Ok(()) => self.set_record(None),
                        Err(failure) => record_transport_failure(&failure, "remove"),
                    }
                }
                herdr_core::diagnostic!(json!({
                    "component": "mobile_transport", "kind": "serve.foreign", "target": target,
                }));
                return Err(Exposure::Foreign { target });
            }
            Ownership::Ours { port: current } if current == port => {}
            Ownership::Ours { .. } => {
                tailscale::serve_remove(program)
                    .await
                    .map_err(|failure| failed("remove", &failure))?;
                self.set_record(None);
                self.add(program, dns_name, port)
                    .await
                    .map_err(|failure| failed("add", &failure))?;
            }
            Ownership::Free => {
                self.add(program, dns_name, port)
                    .await
                    .map_err(|failure| failed("add", &failure))?;
            }
        }
        let record = ServeRecord {
            dns_name: dns_name.to_owned(),
            port,
            added_at: now_ms() / 1000,
        };
        let confirmed = tailscale::serve_status(program)
            .await
            .map_err(|failure| failed("check", &failure))?;
        if funnel(&confirmed) {
            return Err(Exposure::Failed {
                step: "funnel",
                message: tailscale::FUNNEL_MESSAGE.to_owned(),
            });
        }
        match tailscale::ownership(&confirmed, dns_name, Some(&record)) {
            Ok(Ownership::Ours { port: current }) if current == port => Ok(()),
            Ok(other) => {
                let failure = CommandFailure {
                    command: "tailscale serve status --json".to_owned(),
                    stderr: format!("hide's entry is not there after adding it ({other:?})"),
                    exit_code: Some(0),
                };
                Err(failed("check", &failure))
            }
            Err(message) => Err(Exposure::Failed {
                step: "check",
                message,
            }),
        }
    }

    async fn add(
        &self,
        program: &std::path::Path,
        dns_name: &str,
        port: u16,
    ) -> Result<(), CommandFailure> {
        // Recorded before the command runs: a `serve --bg` that applies the
        // entry and then times out or fails must still leave hide owning it,
        // or the next pass would read hide's own entry as someone else's.
        let record = ServeRecord {
            dns_name: dns_name.to_owned(),
            port,
            added_at: now_ms() / 1000,
        };
        self.set_record(Some(record.clone()));
        if let Err(failure) = tailscale::serve_add(program, port).await {
            // Cleared only when a fresh read shows the entry absent; a record
            // with no entry behind it reads as Free on the next pass.
            let absent = tailscale::serve_status(program).await.is_ok_and(|serve| {
                !matches!(
                    tailscale::ownership(&serve, dns_name, Some(&record)),
                    Ok(Ownership::Ours { port: current }) if current == port
                )
            });
            if absent {
                self.set_record(None);
            }
            return Err(failure);
        }
        herdr_core::diagnostic!(json!({
            "component": "mobile_transport", "kind": "serve.added", "dns_name": dns_name, "port": port,
        }));
        Ok(())
    }

    /// Removes hide's serve entry as the daemon stops (a graceful exit);
    /// a crash leaves it to the next start's reconcile.
    pub async fn shutdown(&self) {
        self.stopping.send_replace(true);
        self.touch_live();
        // After any reconcile in flight: an add it finishes is recorded, so
        // the record read here is the entry actually there.
        let _running = self.reconcile_lock.lock().await;
        // A core that never left pending added no entry; the one recorded
        // is the previous core's.
        if !*self.config.open.borrow() {
            return;
        }
        let record = self.lock().settings.serve.clone();
        let (Some(record), Some(program)) = (record, self.config.cli.resolve()) else {
            return;
        };
        let _ = self.remove_recorded(&program, &record).await;
    }

    /// A phone on a live connection is being seen now: its last-seen time
    /// moves, so a phone connected for a week is not revoked at the next start.
    fn touch_live(&self) {
        let live: HashSet<String> = self
            .live()
            .values()
            .map(|phone| phone.phone_id.clone())
            .collect();
        let mut inner = self.lock();
        for phone_id in &live {
            inner.phones.touch(phone_id, now_ms());
        }
    }

    /// Rechecks every three seconds while Settings > Mobile is open, and
    /// once a minute otherwise when Mobile is on but not exposed (Tailscale
    /// may come up after hided) or a removal is still owed after switching off.
    async fn observe_loop(self: Arc<Self>) {
        let mut stopping = self.stopping.subscribe();
        let mut since_background = Duration::ZERO;
        let mut since_delivery = BACKGROUND_INTERVAL;
        let mut pending_delivery: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            tokio::select! {
                _ = tokio::time::sleep(OBSERVE_INTERVAL) => {}
                _ = stopping.changed() => break,
            }
            if pending_delivery
                .as_ref()
                .is_some_and(tokio::task::JoinHandle::is_finished)
                && let Some(pending) = pending_delivery.take()
            {
                let _ = pending.await;
            }
            since_background += OBSERVE_INTERVAL;
            since_delivery += OBSERVE_INTERVAL;
            if since_delivery >= BACKGROUND_INTERVAL {
                since_delivery = Duration::ZERO;
                if pending_delivery.is_none() {
                    let mobile = Arc::clone(&self);
                    // This resident owner retains one pass until it finishes.
                    // A slow Push never delays the existing settings checks.
                    // No timer/queue joins pane input or agent refresh.
                    pending_delivery = Some(tokio::task::spawn_blocking(move || {
                        mobile.deliver_delivery()
                    }));
                }
            }
            let (observed, owed) = {
                let inner = self.lock();
                let observed = inner.settings.enabled && !inner.observers.is_empty();
                // While on, the minute pass also catches what changed under an
                // exposed entry (a Funnel turned on, the entry removed by hand).
                let owed = inner.settings.enabled || inner.settings.serve.is_some();
                (observed, owed)
            };
            if observed || (owed && since_background >= BACKGROUND_INTERVAL) {
                since_background = Duration::ZERO;
                self.reconcile().await;
            }
        }
        if let Some(pending) = pending_delivery {
            // The stop flag prevents another channel call, and this owner
            // joins the one already started before releasing its resources.
            let _ = pending.await;
        }
    }

    fn deliver_delivery(&self) {
        if *self.stopping.borrow() {
            return;
        }
        let notices = match self
            .config
            .core
            .prepare_delivery_human()
            .and_then(|prepared| prepared.run(Duration::from_secs(5)))
        {
            Ok(notices) => notices,
            Err(code) => {
                herdr_core::diagnostic!(
                    json!({"component":"delivery","kind":"human.claim_failed","code":code})
                );
                return;
            }
        };
        for notice in notices {
            if *self.stopping.borrow() {
                return;
            }
            let push = match self.push_delivery(&notice) {
                Ok(()) => continue,
                Err(reason) => reason,
            };
            if *self.stopping.borrow() {
                return;
            }
            // The toast goes to the Herdr of the machine the operator works
            // at: a screen machine linked to this core, through its link,
            // and otherwise this machine's (PRD core-host-node-remote-core
            // D-18). Herdr is skipped only when that machine has no socket
            // for it; its toast setting is not readable, so every other case
            // is tried.
            let machine = self
                .config
                .core
                .linked_nodes()
                .ok()
                .and_then(|nodes| nodes.into_iter().next())
                .unwrap_or_else(|| self.node().as_str().to_owned());
            let herdr = match self.herdr_api(&machine) {
                Ok(connector) => match notice.notify_herdr(connector.as_ref()) {
                    Ok(()) => continue,
                    Err(reason) => reason,
                },
                Err(_) => "no_socket",
            };
            // Its ledger claim remains consumed even when both channels
            // fail. A restart never becomes an external resend. The record
            // names the notice and the reason per channel, never its text.
            herdr_core::diagnostic!(json!({
                "component": "delivery", "kind": "human.channels_failed",
                "notice": notice.kind.code(), "letter_id": notice.id,
                "push": push, "herdr": herdr, "herdr_node": machine,
            }));
        }
    }

    /// `Ok` when a phone got the notice. `Err` is why none did: a reason
    /// `push::notice_route` found before sending, `send_failed` or
    /// `subscription_gone` after trying, or `stopping`.
    fn push_delivery(&self, notice: &HumanNotice) -> Result<(), &'static str> {
        let (vapid, targets, subject) = {
            let inner = self.lock();
            let targets = inner
                .phones
                .current()
                .filter_map(|phone| {
                    phone
                        .push
                        .clone()
                        .map(|subscription| (phone.id.clone(), subscription))
                })
                .take(4)
                .collect::<Vec<_>>();
            let vapid = push::notice_route(
                self.vapid.as_ref(),
                inner.settings.push_mode,
                inner.settings.enabled,
                self.config.renderers.load(Ordering::SeqCst),
                targets.len(),
            )?;
            let subject = inner
                .settings
                .serve
                .as_ref()
                .map(|serve| format!("https://{}", serve.dns_name))
                .unwrap_or_else(|| "mailto:hide@localhost".into());
            (vapid, targets, subject)
        };
        let payload = push::payload(
            &push::Notice {
                key: AgentKey {
                    device_id: notice.actor.device_id.clone(),
                    pane_id: notice.actor.pane_id.clone(),
                },
                // A delivery notice names the agent that has not answered,
                // and the phone words the cause from its state key.
                title: notice.recipient.clone(),
                status_code: None,
                detail: None,
                state: match notice.kind {
                    HumanNoticeKind::ObserverUnconfirmed => push::NoticeState::ObserverUnconfirmed,
                    HumanNoticeKind::LetterUndelivered => push::NoticeState::LetterUndelivered,
                },
                place: match notice.kind {
                    HumanNoticeKind::ObserverUnconfirmed => notice.about.clone(),
                    HumanNoticeKind::LetterUndelivered => notice.id.clone(),
                },
            },
            &BTreeSet::new(),
        );
        let (mut delivered, mut failed) = (false, false);
        for (phone_id, subscription) in targets {
            if *self.stopping.borrow() {
                return Err("stopping");
            }
            match push::send(vapid, &subject, &subscription, &payload, now_ms() / 1000) {
                push::SendOutcome::Delivered => delivered = true,
                push::SendOutcome::Gone(_) => {
                    self.lock()
                        .phones
                        .drop_subscription(&phone_id, &subscription.endpoint);
                    self.publish();
                }
                push::SendOutcome::Failed(_) => failed = true,
            }
        }
        match (delivered, failed) {
            (true, _) => Ok(()),
            (false, true) => Err("send_failed"),
            (false, false) => Err("subscription_gone"),
        }
    }

    async fn sweep_loop(self: Arc<Self>) {
        let mut stopping = self.stopping.subscribe();
        loop {
            tokio::select! {
                _ = tokio::time::sleep(SWEEP_INTERVAL) => {}
                _ = stopping.changed() => return,
            }
            self.sweep();
        }
    }

    /// Revokes every phone unseen for seven days (PRD D-04).
    fn sweep(&self) {
        self.touch_live();
        let connected: Vec<String> = self
            .live()
            .values()
            .map(|phone| phone.phone_id.clone())
            .collect();
        let revoked = self.lock().phones.sweep(now_ms(), &connected);
        for phone in &revoked {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "phone.revoked", "phone_id": phone.id,
                "reason": "inactive_7_days", "had_subscription": phone.push.is_some(),
            }));
        }
        if !revoked.is_empty() {
            self.publish();
        }
    }

    /// Handles one `mobile_*` event from a renderer.
    pub fn handle_event(self: &Arc<Self>, connection: u64, event: &Value) -> Result<(), String> {
        let kind = event.get("kind").and_then(Value::as_str).unwrap_or("");
        let payload = event.get("payload").cloned().unwrap_or(Value::Null);
        match kind {
            "mobile_enable" => {
                let enabled = payload
                    .get("enabled")
                    .and_then(Value::as_bool)
                    .ok_or("mobile_enable.enabled must be a boolean")?;
                {
                    let mut inner = self.lock();
                    if !enabled {
                        // No phone may pair from here on, before the serve
                        // entry is even gone.
                        inner.phones.clear_code();
                    }
                    if inner.settings.enabled != enabled {
                        inner.settings.enabled = enabled;
                        inner.exposure = if enabled {
                            Exposure::Checking
                        } else {
                            Exposure::Off
                        };
                        self.save_settings(&inner);
                    }
                }
                herdr_core::diagnostic!(json!({
                    "component": "mobile_transport", "kind": "mobile.switched", "enabled": enabled,
                }));
                self.publish();
                let mobile = Arc::clone(self);
                tokio::spawn(async move { mobile.reconcile().await });
            }
            "mobile_observe" => {
                let observing = payload
                    .get("observing")
                    .and_then(Value::as_bool)
                    .ok_or("mobile_observe.observing must be a boolean")?;
                let reread = {
                    let mut inner = self.lock();
                    if observing {
                        // Opening Settings > Mobile issues no code and voids
                        // none (B58); only `mobile_show_code` does.
                        inner.observers.insert(connection);
                        inner.settings.enabled
                    } else {
                        inner.observers.remove(&connection);
                        // The last Mobile page closing takes the QR with it.
                        if inner.observers.is_empty() {
                            inner.phones.clear_code();
                        }
                        false
                    }
                };
                self.publish();
                if reread {
                    let mobile = Arc::clone(self);
                    tokio::spawn(async move { mobile.reconcile().await });
                }
            }
            "mobile_show_code" => {
                {
                    let mut inner = self.lock();
                    if !matches!(inner.exposure, Exposure::Exposed { .. }) {
                        return Err("a pairing code needs Mobile exposed".to_owned());
                    }
                    inner.phones.new_code(now_ms());
                }
                self.publish();
            }
            "mobile_hide_code" => {
                self.lock().phones.clear_code();
                self.publish();
            }
            "mobile_revoke" => {
                let id = payload
                    .get("phone_id")
                    .and_then(Value::as_str)
                    .ok_or("mobile_revoke.phone_id must be a string")?;
                self.revoke(id, "revoked_in_settings");
            }
            "mobile_push_mode" => {
                let mode = payload
                    .get("mode")
                    .and_then(Value::as_str)
                    .and_then(PushMode::parse)
                    .ok_or("mobile_push_mode.mode must be off, app_closed or always")?;
                {
                    let mut inner = self.lock();
                    inner.settings.push_mode = mode;
                    self.save_settings(&inner);
                }
                self.publish();
            }
            other => return Err(format!("unknown mobile event {other}")),
        }
        Ok(())
    }

    /// A renderer connection closed: it no longer watches Settings > Mobile.
    pub fn release(&self, connection: u64) {
        let removed = {
            let mut inner = self.lock();
            let removed = inner.observers.remove(&connection);
            if removed && inner.observers.is_empty() {
                inner.phones.clear_code();
            }
            removed
        };
        if removed {
            self.publish();
        }
    }

    /// Revokes a phone: its credential and push subscription go in one
    /// write and its live connections close. A second revoke is a no-op.
    pub fn revoke(&self, phone_id: &str, reason: &str) {
        let removed = {
            let mut inner = self.lock();
            inner.pending_clear.remove(phone_id);
            inner.inputs.remove(phone_id);
            inner.phones.revoke(phone_id)
        };
        if let Some(phone) = removed {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "phone.revoked", "phone_id": phone.id,
                "reason": reason, "had_subscription": phone.push.is_some(),
            }));
            for live in self
                .live()
                .values()
                .filter(|live| live.phone_id == phone_id)
            {
                live.close.notify_one();
            }
        }
        self.publish();
        self.wake.notify_one();
    }

    fn close_phones(&self, reason: &str) {
        let live = self.live();
        if !live.is_empty() {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "phones.closed", "reason": reason, "count": live.len(),
            }));
        }
        for phone in live.values() {
            phone.close.notify_one();
        }
    }

    /// Pairs a phone with the live code (the phone's `pair` handshake). A
    /// code exists only while Mobile is exposed.
    pub fn pair(&self, code: &str, user_agent: &str) -> Result<(PhoneRecord, String), PairRefusal> {
        let result = {
            let mut inner = self.lock();
            let admitted =
                inner.settings.enabled && matches!(inner.exposure, Exposure::Exposed { .. });
            let result = if admitted {
                inner.phones.pair(code, user_agent, now_ms())
            } else {
                Err(PairRefusal::CodeExpired)
            };
            // The code is spent; the next phone needs `mobile_show_code`.
            result
        };
        match &result {
            Ok((phone, _)) => herdr_core::diagnostic!(json!({
                "component": "mobile_pairing", "kind": "phone.paired", "phone_id": phone.id, "name": phone.name,
            })),
            Err(refusal) => herdr_core::diagnostic!(json!({
                "component": "mobile_pairing", "kind": "pairing.refused", "reason": refusal.reason(),
            })),
        }
        self.publish();
        self.wake.notify_one();
        result
    }

    /// The phone a credential belongs to. `mobile_off` while the switch is
    /// off (the phone stays paired); `revoked` for a credential no phone
    /// holds, which is what a revoked phone's credential is.
    pub fn authenticate(&self, credential: &str) -> Result<PhoneRecord, &'static str> {
        let mut inner = self.lock();
        // A phone paired where the core ran before a move stays paired and
        // reads the core as unreachable until it pairs here (B17).
        let phone =
            inner.phones.authenticate(credential).cloned().map_err(
                |unadmitted| match unadmitted {
                    phones::Unadmitted::Unknown => "revoked",
                    phones::Unadmitted::Elsewhere => "mobile_off",
                },
            )?;
        // The seven-day rule holds at the door too, not only at the hourly
        // sweep, whose timer does not run while the Mac sleeps.
        if now_ms().saturating_sub(phone.last_seen_ms) >= phones::INACTIVE_REVOKE_MS {
            drop(inner);
            self.revoke(&phone.id, "inactive_7_days");
            return Err("revoked");
        }
        if !inner.settings.enabled {
            return Err("mobile_off");
        }
        inner.phones.touch(&phone.id, now_ms());
        Ok(phone)
    }

    /// Why a registered connection must close at once: the phone was revoked
    /// or Mobile switched off between its handshake and its registration.
    /// Admits one write to a pane's terminal, or refuses it while the pane's
    /// agent is ending, asleep or waking. Hold it across the write.
    pub fn admit_input(
        &self,
        pane_id: &str,
    ) -> Result<herdr_core::input_gate::Admission, pane::PaneError> {
        self.config
            .input_gate
            .admit(pane_id)
            .map_err(|_| pane::PaneError::Asleep)
    }

    pub fn still_admitted(&self, phone_id: &str) -> Result<(), &'static str> {
        let inner = self.lock();
        if inner.phones.get(phone_id).is_none() {
            Err("revoked")
        } else if !inner.settings.enabled {
            Err("mobile_off")
        } else {
            Ok(())
        }
    }

    pub fn phone(&self, id: &str) -> Option<PhoneRecord> {
        self.lock().phones.get(id).cloned()
    }

    pub fn register(&self, connection: u64, phone_id: &str) -> Arc<Notify> {
        let close = Arc::new(Notify::new());
        {
            let mut live = self.live();
            // One phone holds at most two sockets: a phone that reconnects
            // while its old socket is still half-open closes the oldest, so
            // flapping never crowds the desktop out of MAX_CLIENTS.
            let mut own: Vec<(u64, u64)> = live
                .iter()
                .filter(|(_, phone)| phone.phone_id == phone_id)
                .map(|(connection, phone)| (phone.since, *connection))
                .collect();
            own.sort_unstable();
            while own.len() >= CONNECTIONS_PER_PHONE {
                let (_, oldest) = own.remove(0);
                if let Some(phone) = live.get(&oldest) {
                    phone.close.notify_one();
                }
            }
            live.insert(
                connection,
                LivePhone {
                    phone_id: phone_id.to_owned(),
                    viewing: None,
                    close: Arc::clone(&close),
                    since: now_ms(),
                },
            );
        }
        herdr_core::diagnostic!(json!({
            "component": "mobile_phone", "kind": "phone.connected", "phone_id": phone_id,
        }));
        self.publish();
        self.wake.notify_one();
        close
    }

    pub fn unregister(&self, connection: u64) {
        self.set_start_sheet(connection, false);
        let removed = self.live().remove(&connection);
        if let Some(phone) = removed {
            self.lock().phones.touch(&phone.phone_id, now_ms());
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "phone.disconnected", "phone_id": phone.phone_id,
            }));
        }
        self.publish();
    }

    /// This connection's start sheet opened or closed: while any surface
    /// shows one, the core reads the provider catalog.
    pub fn set_start_sheet(&self, connection: u64, open: bool) {
        self.config.start_demand.set(connection, open, |aggregate| {
            crate::server::dispatch_observation(
                &self.config.core,
                crate::server::Demand::Start,
                connection,
                aggregate,
            )
        });
    }

    /// One `start_agent` frame: refused before anything is dispatched when
    /// the phone is no longer admitted, else started through the core and
    /// answered once the core says how it went (PRD D-24, D-25).
    pub async fn start_agent(&self, phone_id: &str, message: &Value) -> Value {
        let refused = |reason: &str| json!({"type": "start_result", "request_id": message.get("request_id"), "ok": false, "reason": reason});
        let Some(request_id) = message
            .get("request_id")
            .and_then(Value::as_str)
            .filter(|id| start::request_id_valid(id))
        else {
            return json!({"type": "start_result", "ok": false, "reason": "invalid_request"});
        };
        if let Err(reason) = self.still_admitted(phone_id) {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "start.refused", "phone_id": phone_id, "reason": reason,
            }));
            return refused(reason);
        }
        let text = |field: &str| message.get(field).and_then(Value::as_str);
        let (Some(prompt), Some(target), Some(kind)) = (text("text"), text("target"), text("kind"))
        else {
            return refused("invalid_request");
        };
        let model = match message.get("model") {
            None | Some(Value::Null) => None,
            Some(Value::String(model)) => Some(model.as_str()),
            Some(_) => return refused("invalid_request"),
        };
        let catalog = Arc::clone(&self.catalog.borrow());
        let core = Arc::clone(&self.config.core);
        self.starts
            .start(
                phone_id,
                start::Request {
                    request_id,
                    text: prompt,
                    target,
                    kind,
                    model,
                },
                &catalog,
                self.answers.subscribe(),
                start::ANSWER_LIMIT,
                move |event| core.dispatch(event),
            )
            .await
    }

    pub fn set_viewing(&self, connection: u64, viewing: Option<AgentKey>) {
        if let Some(phone) = self.live().get_mut(&connection) {
            phone.viewing = viewing;
        }
    }

    /// Whether any phone has this root agent's detail open (no push then).
    fn viewed(&self, root: &AgentKey, projection: &Projection) -> bool {
        self.live()
            .values()
            .filter_map(|phone| phone.viewing.as_ref())
            .any(|viewing| {
                viewing == root
                    || projection
                        .find(viewing)
                        .is_some_and(|agent| &agent.root_key() == root)
            })
    }

    /// Claims an input request id before it is written, in one step, so a
    /// repeat on this or another socket of the phone never writes it twice.
    pub fn reserve_input(&self, phone_id: &str, request_id: &str) -> Reservation {
        let mut inner = self.lock();
        let seen = inner.inputs.entry(phone_id.to_owned()).or_default();
        if let Some((_, state)) = seen.iter().find(|(id, _)| id == request_id) {
            return Reservation::Seen(*state);
        }
        if seen.len() >= REMEMBERED_INPUTS {
            seen.pop_front();
        }
        seen.push_back((request_id.to_owned(), InputState::InFlight));
        Reservation::Write
    }

    /// Settles a reserved id; `None` forgets it (nothing was written, so the
    /// phone may send it again).
    pub fn settle_input(&self, phone_id: &str, request_id: &str, state: Option<InputState>) {
        let mut inner = self.lock();
        let Some(seen) = inner.inputs.get_mut(phone_id) else {
            return;
        };
        match state {
            Some(state) => {
                if let Some(entry) = seen.iter_mut().find(|(id, _)| id == request_id) {
                    entry.1 = state;
                }
            }
            None => seen.retain(|(id, _)| id != request_id),
        }
    }

    pub fn set_subscription(&self, phone_id: &str, push: PushSubscription) -> bool {
        if !push::subscription_valid(&push) {
            herdr_core::diagnostic!(json!({
                "component": "mobile_push", "kind": "subscription.refused", "phone_id": phone_id,
            }));
            return false;
        }
        let changed = self.lock().phones.set_subscription(phone_id, Some(push));
        if changed {
            herdr_core::diagnostic!(json!({
                "component": "mobile_push", "kind": "subscription.stored", "phone_id": phone_id,
            }));
            self.publish();
        }
        true
    }

    pub fn set_notifications(&self, phone_id: &str, notifications: Notifications) {
        if self
            .lock()
            .phones
            .set_notifications(phone_id, notifications)
        {
            self.publish();
        }
    }

    pub fn home(&self) -> &std::path::Path {
        &self.config.home
    }

    /// Tells the core the operator submitted a reply to `pane_id` from the
    /// phone, so the message it writes reads as theirs (PRD
    /// overview-request-view D-19). A refusal only loses that attribution.
    pub fn note_submit(&self, pane_id: &str) {
        let event = json!({
            "schema_version": 2,
            "kind": "pane_input_submitted",
            "payload": {"pane_id": pane_id},
        });
        if let Err(message) = self.config.core.dispatch(event.to_string().into_bytes()) {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "input.submit_unrecorded", "message": message,
            }));
        }
    }

    /// Tells the core a phone write to `pane_id` is about to be made, so the
    /// doorbell treats the pane as typed into. Sent before the write: one
    /// Herdr then refuses still holds the pane until a prompt hook or a turn
    /// clears it, which is safer than a window where the write lands unseen.
    pub fn note_input(&self, pane_id: &str) {
        let event = json!({
            "schema_version": 2,
            "kind": "pane_input_sent",
            "payload": {"pane_id": pane_id},
        });
        if let Err(message) = self.config.core.dispatch(event.to_string().into_bytes()) {
            herdr_core::diagnostic!(json!({
                "component": "mobile_phone", "kind": "input.sent_unrecorded", "message": message,
            }));
        }
    }

    pub fn terminals(&self) -> &dyn hide_node_link::terminal::TerminalNode {
        self.config.core.terminals.as_ref()
    }

    /// The Herdr connection for a pane on this Mac or a connected device.
    pub fn herdr_api(
        &self,
        device_id: &str,
    ) -> Result<Arc<dyn hide_herdr_client::ApiConnector>, pane::PaneError> {
        if self.node() == device_id {
            return self
                .config
                .herdr_socket
                .as_ref()
                .map(|socket| {
                    Arc::new(hide_herdr_client::LocalSocketConnector::new(socket.clone()))
                        as Arc<dyn hide_herdr_client::ApiConnector>
                })
                .ok_or_else(|| {
                    pane::PaneError::Unavailable("this Mac has no Herdr socket".to_owned())
                });
        }
        match self.config.core.remote_herdr_api(device_id) {
            Ok(Some(connector)) => Ok(connector),
            Ok(None) => Err(pane::PaneError::DeviceUnreachable),
            Err(message) => Err(pane::PaneError::Unavailable(message)),
        }
    }

    /// Follows the core's snapshot while Mobile is on with a phone paired or
    /// a desktop window has notifications on: one read per notification
    /// burst, the phone projection republished only when it changed, and the
    /// phones' and the desktop windows' transitions judged on it, each only
    /// while its own side wants it.
    async fn follow(self: Arc<Self>) {
        let mut changes = self.config.core.notify.subscribe();
        let mut stopping = self.stopping.subscribe();
        let mut cursor = 0_u64;
        let mut rest = Value::Null;
        let mut transitions = push::Transitions::default();
        loop {
            if *stopping.borrow() {
                return;
            }
            if !self.follower_active() {
                cursor = 0;
                rest = Value::Null;
                transitions.reset();
                self.catalog
                    .send_replace(Arc::new(start::Catalog::default()));
                self.answers.send_replace(Arc::new(Vec::new()));
                self.projection.send_if_modified(|current| {
                    if current.groups.is_empty() {
                        false
                    } else {
                        *current = Arc::new(Projection::default());
                        true
                    }
                });
                tokio::select! {
                    _ = self.wake.notified() => continue,
                    _ = stopping.changed() => return,
                }
            }
            let mut changed = false;
            let core = Arc::clone(&self.config.core);
            let have_revision = cursor;
            let read = tokio::task::spawn_blocking(move || core.snapshot(have_revision)).await;
            let payload = match read {
                Ok(Ok(payload)) => payload,
                Ok(Err(_)) => return,
                Err(error) => {
                    projection_failed("snapshot_worker", &error.to_string());
                    return;
                }
            };
            cursor = payload.revision;
            match projection::update_inputs(&mut rest, &payload.current_rest, payload.have_revision)
            {
                Err(error) => {
                    projection_failed("inputs", &error.to_string());
                    return;
                }
                Ok(true) => {
                    let catalog = start::Catalog::of(&rest, self.node().as_str());
                    self.catalog.send_if_modified(|current| {
                        if **current == catalog {
                            false
                        } else {
                            *current = Arc::new(catalog);
                            true
                        }
                    });
                    self.answers.send_if_modified(|current| {
                        let mut kept = current.as_ref().clone();
                        let changed = start::record(
                            &mut kept,
                            start::answers_of(&rest, self.node().as_str()),
                        );
                        if changed {
                            *current = Arc::new(kept);
                        }
                        changed
                    });
                    let next = projection::project(&rest, self.node().as_str());
                    changed = self.projection.send_if_modified(|current| {
                        if **current == next {
                            false
                        } else {
                            *current = Arc::new(next.clone());
                            true
                        }
                    });
                }
                Ok(false) => {}
            }
            let current = Arc::clone(&self.projection.borrow());
            if let Some((notices, seen)) = judge_pass(
                &mut transitions,
                self.keep_alive(),
                &self.config.desktop,
                &current,
                changed,
                cursor != 0,
            ) {
                self.deliver(notices, seen, &current);
            }
            tokio::select! {
                changed = changes.recv() => {
                    if matches!(changed, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                        return;
                    }
                    while changes.try_recv().is_ok() {}
                }
                _ = self.wake.notified() => {}
                _ = stopping.changed() => return,
            }
        }
    }

    /// Sends each notice to every subscribed phone the mode allows, on a
    /// blocking thread: a push service's latency never holds the follower.
    fn deliver(
        self: &Arc<Self>,
        notices: Vec<push::Notice>,
        seen: BTreeSet<AgentKey>,
        projection: &Projection,
    ) {
        let (mode, targets, subject) = {
            let mut inner = self.lock();
            let ids: Vec<String> = inner
                .phones
                .current()
                .map(|phone| phone.id.clone())
                .collect();
            if !seen.is_empty() {
                for id in &ids {
                    inner
                        .pending_clear
                        .entry(id.clone())
                        .or_default()
                        .extend(seen.iter().cloned());
                }
            }
            let targets: Vec<(String, PushSubscription)> = inner
                .phones
                .current()
                .filter_map(|phone| phone.push.clone().map(|push| (phone.id.clone(), push)))
                .collect();
            let subject = inner
                .settings
                .serve
                .as_ref()
                .map(|serve| format!("https://{}", serve.dns_name))
                .unwrap_or_else(|| "mailto:hide@localhost".to_owned());
            (inner.settings.push_mode, targets, subject)
        };
        let renderers = self.config.renderers.load(Ordering::SeqCst);
        let mut jobs = Vec::new();
        for notice in notices {
            if !push::mode_allows(mode, renderers) {
                herdr_core::diagnostic!(json!({
                    "component": "mobile_push", "kind": "push.skipped", "reason": "mode",
                    "mode": mode.as_str(), "renderers": renderers,
                }));
                continue;
            }
            if self.viewed(&notice.key, projection) {
                herdr_core::diagnostic!(json!({
                    "component": "mobile_push", "kind": "push.skipped", "reason": "viewing",
                }));
                continue;
            }
            for (phone_id, subscription) in &targets {
                let clear = self
                    .lock()
                    .pending_clear
                    .remove(phone_id)
                    .unwrap_or_default();
                jobs.push((
                    phone_id.clone(),
                    subscription.clone(),
                    push::payload(&notice, &clear),
                ));
            }
        }
        if jobs.is_empty() || self.vapid.is_none() {
            return;
        }
        let mobile = Arc::clone(self);
        tokio::task::spawn_blocking(move || {
            let Some(vapid) = mobile.vapid.as_ref() else {
                return;
            };
            for (phone_id, subscription, payload) in jobs {
                match push::send(vapid, &subject, &subscription, &payload, now_ms() / 1000) {
                    push::SendOutcome::Delivered => herdr_core::diagnostic!(json!({
                        "component": "mobile_push", "kind": "push.sent", "phone_id": phone_id,
                        "tag": payload.get("tag"),
                    })),
                    push::SendOutcome::Gone(status) => {
                        mobile
                            .lock()
                            .phones
                            .drop_subscription(&phone_id, &subscription.endpoint);
                        herdr_core::diagnostic!(json!({
                            "component": "mobile_push", "kind": "subscription.dropped", "phone_id": phone_id,
                            "status": status,
                        }));
                        mobile.publish();
                    }
                    push::SendOutcome::Failed(message) => herdr_core::diagnostic!(json!({
                        "component": "mobile_push", "kind": "push.failed", "phone_id": phone_id,
                        "message": message,
                    })),
                }
            }
        });
    }
}

/// One follower pass's judgement of the projection `current` for each side
/// that wants it: the phones' push transitions while Mobile keeps the
/// follower (`phone_on`), the desktop windows' while one asks (B38). A
/// projection that `changed` is observed by each side on; once `read`, a side
/// that is on but unseeded is seeded from what this follower last read, so a
/// wait that began while that side was off is never announced late (B33). The
/// phones' notices to deliver are returned; the desktop's go out as a frame.
fn judge_pass(
    phone: &mut push::Transitions,
    phone_on: bool,
    desktop: &crate::desktop_notices::DesktopNotices,
    current: &Projection,
    changed: bool,
    read: bool,
) -> Option<(Vec<push::Notice>, BTreeSet<AgentKey>)> {
    let mut delivered = None;
    if changed && phone_on {
        delivered = Some(phone.observe(current));
    }
    if changed && desktop.wanted() {
        desktop.observe(current);
    }
    if !phone_on {
        phone.reset();
    } else if read && !phone.seeded() {
        phone.observe(current);
    }
    if read && desktop.seed_pending() {
        desktop.observe(current);
    }
    delivered.filter(|(notices, seen)| !notices.is_empty() || !seen.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_notices::DesktopNotices;
    use herdr_core::agent_state::phone::project;

    fn projected(group: &str, demand: &str) -> Projection {
        project(
            &json!({
                "navigator": {
                    "agents": [{
                        "pane_id": "w1:p1", "group": group, "mark": "going",
                        "identity_label": "Add a dark mode switch", "agent_kind": "claude",
                        "changed_at_unix_ms": 1_790_000_000_000_u64, "status_code": "idle",
                        "demand": demand,
                    }],
                    "workspaces": [{"label": "my-app", "is_git": true, "checkouts": [
                        {"label": "main", "branch": "main", "tabs": [{"panes": [{"id": "w1:p1"}]}]},
                    ]}],
                    "devices": [{"id": "node", "label": null}],
                },
            }),
            "node",
        )
    }

    /// What one pass hands each side: the phones' notice count, and whether
    /// a desktop frame went out.
    fn pass(
        phone: &mut push::Transitions,
        phone_on: bool,
        desktop: &DesktopNotices,
        current: &Projection,
    ) -> (usize, bool) {
        let mut frames = desktop.subscribe();
        let phones = judge_pass(phone, phone_on, desktop, current, true, true)
            .map_or(0, |(notices, _)| notices.len());
        (phones, frames.try_recv().is_ok())
    }

    /// B38, B33: each side judges the shared projection only while it is on,
    /// a side turned on later is seeded by what was already waiting, and the
    /// other side's switch never changes what a side announces.
    #[test]
    fn each_side_announces_only_while_on_and_a_late_side_is_seeded_not_told() {
        for (phone_on, desktop_on) in [(true, true), (true, false), (false, true), (false, false)] {
            let label = format!("phone {phone_on}, desktop {desktop_on}");
            let mut phone = push::Transitions::default();
            let desktop = DesktopNotices::default();
            desktop.set(1, desktop_on);
            assert_eq!(
                pass(
                    &mut phone,
                    phone_on,
                    &desktop,
                    &projected("working", "none")
                ),
                (0, false),
                "{label}: the first read only seeds"
            );
            assert_eq!(
                pass(
                    &mut phone,
                    phone_on,
                    &desktop,
                    &projected("needs_you", "approval")
                ),
                (usize::from(phone_on), desktop_on),
                "{label}: a new wait is announced by each side that is on"
            );
        }

        // A side turned on while an agent already waits is seeded, not told,
        // and the other side keeps announcing as before.
        let mut phone = push::Transitions::default();
        let desktop = DesktopNotices::default();
        let waiting = projected("needs_you", "approval");
        assert_eq!(
            pass(&mut phone, true, &desktop, &projected("working", "none")),
            (0, false)
        );
        assert_eq!(pass(&mut phone, true, &desktop, &waiting), (1, false));
        desktop.set(1, true);
        let mut frames = desktop.subscribe();
        assert!(judge_pass(&mut phone, true, &desktop, &waiting, false, true).is_none());
        assert!(
            frames.try_recv().is_err(),
            "the desktop is seeded with the wait that was there"
        );
        assert!(
            pass(&mut phone, true, &desktop, &projected("seen", "none")).1,
            "and clears it once seen"
        );

        let mut phone = push::Transitions::default();
        let desktop = DesktopNotices::default();
        desktop.set(1, true);
        assert_eq!(
            pass(&mut phone, false, &desktop, &projected("working", "none")),
            (0, false)
        );
        assert_eq!(pass(&mut phone, false, &desktop, &waiting), (0, true));
        assert!(
            judge_pass(&mut phone, true, &desktop, &waiting, false, true).is_none(),
            "Mobile turned on later is seeded with the wait, not told"
        );
        assert_eq!(
            pass(&mut phone, true, &desktop, &projected("working", "none")),
            (0, false)
        );
        assert_eq!(pass(&mut phone, true, &desktop, &waiting), (1, true));
    }
}
