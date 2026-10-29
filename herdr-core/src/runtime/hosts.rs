//! Each registered device's file host: the operator's consent, the helper
//! connection it allows, and the one state every file and Git action on that
//! device reads (PRD S5.5 D-20, D-23, B50-B52).
//!
//! A device without consent never gets a helper connection. Consent given at
//! registration or later is bound to the SSH identity the helper first runs
//! on; a device that later answers with another account, address or host key
//! is refused until the operator allows it again. Revoking stops the
//! connection admitting work at once, whoever still holds it, and refuses a
//! request still waiting for a slot; work already admitted answers on it and
//! settles to its real result before it closes, a save held for the helper is
//! dropped unsent, and nothing on the device is removed.

use std::sync::Arc;

use super::devices::{DeviceReach, Retiring};
use super::*;
use crate::agent_state::header::LinkWait;
use crate::model::{DeviceHostSnapshot, HostConsent};
use crate::node_access::NodeLink;
use crate::remote::{
    Attempt, EstablishError, Established, HOST_CONSENT_CARRIED_FROM, HOST_CONSENT_CONTRACT,
    NodeReady, OnClose, SshFailure, SshProblem, Wake,
};

/// A node and the link that reads its files.
pub(crate) type LinkedNode = (crate::node::NodeId, Arc<dyn NodeLink>);

/// A connected device's node link, which the daemon's Browser View routes
/// ride to the device's loopback and files, over whichever link the device
/// has. All fields are owned handles, so no network work is done under the
/// runtime lock.
#[derive(Clone)]
pub struct WorkspaceRemoteRoute {
    pub device_id: String,
    pub generation: u64,
    pub channel: Arc<dyn NodeLink>,
}

pub(super) enum HostPhase {
    NotAllowed,
    Connecting,
    Ready {
        host: Arc<dyn NodeLink>,
        platform: String,
        /// The core's helper on a device it dialed; a node that dials in
        /// runs its own Hide, so it has none.
        helper_path: Option<String>,
        /// Where the node shows its Windows PC's drives when it is a WSL
        /// distribution (`Hello::windows_drives`).
        windows_drives: Option<String>,
    },
    IdentityChanged(String),
    Unsupported(String),
    Unavailable(String),
}

/// How long a device's link waits before it is tried again after it was
/// lost or could not start: two seconds, doubling to a minute. A link that
/// starts begins the schedule over, and the operator's Retry tries at once.
const HOST_RETRY_FIRST_MS: u64 = 2_000;
const HOST_RETRY_MAX_MS: u64 = 60_000;

/// Whether the device's Herdr session answering again has already moved this
/// outage's helper retry forward. It does so once, so a session that flaps
/// cannot turn the schedule's cap into a dial every few seconds; the link
/// reaching ready ends the outage and its record.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum SessionBack {
    #[default]
    Unseen,
    /// The session came back while an attempt ran: should it fail, the next
    /// one is due at once.
    DuringAttempt,
    Spent,
}

/// When a lost or failed link may be tried again, and the wait after that.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct HostRetry {
    pub(super) at_unix_ms: Option<u64>,
    pub(super) delay_ms: u64,
    pub(super) session_back: SessionBack,
}

impl Default for HostRetry {
    fn default() -> Self {
        Self {
            at_unix_ms: None,
            delay_ms: HOST_RETRY_FIRST_MS,
            session_back: SessionBack::Unseen,
        }
    }
}

/// One connection attempt's work: a dial of the core's own, or the node a
/// node that dialed in brought.
enum HostStart {
    Dial {
        client: Arc<dyn crate::remote::DialedTransport>,
        consent: HostConsent,
        retirement_projects: Vec<String>,
        wake: Wake,
    },
    Arrived {
        node: NodeReady,
        hear_close: Box<dyn FnOnce(OnClose) + Send + 'static>,
    },
}

/// Why a device's link is tried. This is the one place that decides whether
/// the attempt may start a machine that is off, a WSL distribution of this
/// PC (PRD wsl-device D-11, B3, B23, B24): only the app opening and the
/// operator's own asking may; everything Hide does by itself may not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum HostAttempt {
    /// The app opened a window on this core.
    AppOpened,
    /// The operator asked: `다시 연결`, consent given, a device added.
    Operator,
    /// Hide's own work: the core starting, a scheduled retry, a read or kit
    /// job that needs the link, the device's session answering again.
    Background,
}

impl HostAttempt {
    fn wake(self) -> Wake {
        match self {
            Self::AppOpened | Self::Operator => Wake::Allowed,
            Self::Background => Wake::NotAllowed,
        }
    }
}

/// A connection attempt that reached the device's node.
pub(super) enum HostReady {
    Dialed(Established),
    Arrived(NodeReady),
}

impl HostReady {
    fn node(&self) -> &NodeReady {
        match self {
            Self::Dialed(established) => &established.node,
            Self::Arrived(node) => node,
        }
    }
}

pub(super) struct DeviceHost {
    pub(super) phase: HostPhase,
    /// Taken from the runtime-wide counter on every connection attempt and
    /// close, so a late answer from an older attempt, including one made
    /// before the device was removed and added again, is recognised and
    /// dropped.
    pub(super) generation: u64,
    /// Whether the running or last attempt might ask the operator.
    pub(super) wake: Wake,
    /// Why the last attempt's `ssh` did not connect, by class (B18); none
    /// once an attempt starts or the link is up.
    pub(super) problem: Option<SshProblem>,
    /// What the running attempt waits on from the operator (B4-B12, B27).
    pub(super) sign_in: Option<super::device_sign_in::SignInState>,
}

impl DeviceHost {
    pub(super) fn new(phase: HostPhase, generation: u64) -> Self {
        Self {
            phase,
            generation,
            wake: Wake::NotAllowed,
            problem: None,
            sign_in: None,
        }
    }
}

impl Runtime {
    pub fn workspace_remote_routes(&self) -> Vec<WorkspaceRemoteRoute> {
        self.device_hosts
            .iter()
            .filter_map(|(device_id, host)| {
                let HostPhase::Ready { host: channel, .. } = &host.phase else {
                    return None;
                };
                if channel.closed_reason().is_some()
                    || !self
                        .snapshot
                        .status
                        .remote
                        .iter()
                        .any(|status| status.target_id == *device_id && status.state == "connected")
                {
                    return None;
                }
                Some(WorkspaceRemoteRoute {
                    device_id: device_id.clone(),
                    generation: host.generation,
                    channel: Arc::clone(channel),
                })
            })
            .collect()
    }
    pub(super) fn host_helper_root(&self) -> String {
        self.host_helper_root.clone()
    }

    pub(super) fn device_registration_exists(&self, device_id: &str) -> bool {
        self.device_registration(device_id).is_some()
    }

    pub(super) fn device_registration(
        &self,
        device_id: &str,
    ) -> Option<&crate::model::DeviceRegistration> {
        self.snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.id == device_id)
    }

    /// Whether a consent still covers what this build would do: the same
    /// contract, the same install root and the same command folder. A
    /// contract-2 or -3 consent for the same folders counts: the operator
    /// chose to carry it to the whole kit on its next connection (PRD
    /// device-parity D-13), and contract 4 changed only what the consent is
    /// bound to (PRD system-ssh-device-link D-09, B16).
    fn consent_current(&self, consent: &HostConsent) -> bool {
        (consent.contract == HOST_CONSENT_CONTRACT
            || HOST_CONSENT_CARRIED_FROM.contains(&consent.contract))
            && (consent.helper_root == self.host_helper_root || self.carries_root(consent))
            && consent.cli_dir.as_deref() == Some(self.host_cli_dir.as_str())
    }

    /// Whether a consent names the legacy default helper root while this
    /// build installs at the default one: the operator allowed Hide's helper
    /// in Hide's own folder, which moved, so the consent moves with it
    /// without asking (PRD hide-home-layout D-12). A root the daemon was told
    /// (HIDE_HOST_HELPER_ROOT) is another scope, and a consent for any other
    /// root stays as it was.
    fn carries_root(&self, consent: &HostConsent) -> bool {
        self.host_helper_root == hide_kit::layout::HELPER_ROOT
            && consent.helper_root == hide_kit::layout::LEGACY_HELPER_ROOT
    }

    /// The root a current consent installs at.
    fn consent_root(&self, consent: &HostConsent) -> String {
        if self.carries_root(consent) {
            self.host_helper_root.clone()
        } else {
            consent.helper_root.clone()
        }
    }

    /// Rewrites a carried consent to this build's scope (a contract-2 or -3
    /// consent to contract 4, a legacy default root to the default root),
    /// keeping the identity it was bound to, before the connection it covers
    /// starts.
    fn carry_consent_forward(&mut self, device_id: &str) {
        let root = self.host_helper_root.clone();
        let carries_root = self
            .device_registration(device_id)
            .and_then(|registration| registration.host_consent.as_ref())
            .is_some_and(|consent| self.carries_root(consent));
        let Some(consent) = self
            .snapshot
            .ui_state
            .device_registrations
            .iter_mut()
            .find(|registration| registration.id == device_id)
            .and_then(|registration| registration.host_consent.as_mut())
            .filter(|consent| {
                HOST_CONSENT_CARRIED_FROM.contains(&consent.contract) || carries_root
            })
        else {
            return;
        };
        let from_contract = consent.contract;
        consent.contract = HOST_CONSENT_CONTRACT;
        let from_root = carries_root.then(|| std::mem::replace(&mut consent.helper_root, root));
        self.persist_current_ui_state();
        if HOST_CONSENT_CARRIED_FROM.contains(&from_contract) {
            crate::diagnostic!(serde_json::json!({
                "component": "remote_host",
                "kind": "host.consent_upgraded",
                "target": device_id,
                "from": from_contract,
                "contract": HOST_CONSENT_CONTRACT,
            }));
        }
        if let Some(from_root) = from_root {
            crate::diagnostic!(serde_json::json!({
                "component": "remote_host",
                "kind": "host.consent_root_carried",
                "target": device_id,
                "from": from_root,
                "helper_root": self.host_helper_root,
            }));
        }
    }

    /// The consent a connection starts with: carried forward first, then read
    /// again, so the connection installs at the root the carry wrote rather
    /// than at the one the registration was cloned with.
    pub(super) fn carried_consent(&mut self, device_id: &str, consent: HostConsent) -> HostConsent {
        self.carry_consent_forward(device_id);
        self.device_registration(device_id)
            .and_then(|registration| registration.host_consent.clone())
            .unwrap_or(consent)
    }

    /// A fresh consent in this build's scope, unbound until the first
    /// connection.
    pub(super) fn new_host_consent(&self) -> HostConsent {
        HostConsent {
            contract: HOST_CONSENT_CONTRACT,
            helper_root: self.host_helper_root(),
            cli_dir: Some(self.host_cli_dir.clone()),
            granted_at_unix_ms: now_unix_ms(),
            identity: None,
        }
    }

    /// Gives or withdraws consent for one device.
    pub(super) fn set_host_consent(&mut self, device_id: &str, allow: bool) -> bool {
        if self
            .link_origin(device_id)
            .is_some_and(|origin| !origin.takes_consent())
        {
            self.set_error(
                "device.host.inbound",
                "This machine connects to the core itself with your SSH login; it takes no helper consent",
                false,
            );
            return true;
        }
        let fresh = self.new_host_consent();
        let Some(registration) = self
            .snapshot
            .ui_state
            .device_registrations
            .iter_mut()
            .find(|registration| registration.id == device_id)
        else {
            self.set_error(
                "device.host.unknown_device",
                format!("Device {device_id} is not registered"),
                false,
            );
            return true;
        };
        if allow {
            registration.host_consent = Some(fresh);
            self.persist_current_ui_state();
            crate::diagnostic!(serde_json::json!({
                "component": "remote_host",
                "kind": "host.consent_granted",
                "target": device_id,
                "contract": HOST_CONSENT_CONTRACT,
            }));
            self.close_device_host(device_id, "consent renewed");
            self.retry_device_host_now(device_id, HostAttempt::Operator);
            self.relist_remote_files(device_id);
        } else {
            registration.host_consent = None;
            self.persist_current_ui_state();
            crate::diagnostic!(serde_json::json!({
                "component": "remote_host",
                "kind": "host.consent_revoked",
                "target": device_id,
            }));
            // New work stops here: the channel is no longer handed out and
            // admits nothing more from a worker still holding it. Work
            // already admitted answers on the old connection, which closes
            // once it is idle, so a save in flight settles to its real
            // result rather than becoming unknown (B52).
            if self.device_hosts.contains_key(device_id) {
                self.advance_host_generation(device_id);
                let entry = self.device_host_entry(device_id);
                if let HostPhase::Ready { host, .. } =
                    std::mem::replace(&mut entry.phase, HostPhase::NotAllowed)
                {
                    host.close_when_idle("consent revoked");
                }
            }
            self.set_host_phase(device_id, HostPhase::NotAllowed);
            // The terminals ride the link that is closing: the device's panes
            // stop taking keys, so they read unavailable now and not only
            // when the link's end is heard (it is ignored, its attempt is
            // over).
            self.end_device_terminals(device_id, "Hide's helper is not allowed on this device");
            self.refresh_pane_headers();
            self.forget_device_kit_work(device_id);
            // A save held for the helper to become ready would otherwise go
            // out on its own if consent is given again later (B52).
            self.release_held_saves(
                device_id,
                "Hide's helper is no longer allowed on this device",
            );
        }
        self.refresh_device_snapshots();
        true
    }

    /// Starts a helper connection for a consented device unless one is
    /// running or starting, for `attempt`. Returns whether the device row
    /// changed. A node that dials this core is started only by the link it
    /// brings ([`Self::start_arrived_host`]).
    pub(super) fn start_device_host(&mut self, device_id: &str, attempt: HostAttempt) -> bool {
        let Some(registration) = self.device_registration(device_id).cloned() else {
            return false;
        };
        if !registration.origin.core_redials() || self.effects_held() {
            return false;
        }
        match self.device_hosts.get(device_id) {
            Some(host) if matches!(host.phase, HostPhase::Connecting) => {
                // Leave for the attempt after this one, should this one
                // need what it could not ask for; one that may ask has it.
                if attempt.wake() == Wake::Allowed && host.wake == Wake::NotAllowed {
                    self.device_wake_pending.insert(device_id.to_owned());
                }
                return false;
            }
            Some(host) if matches!(host.phase, HostPhase::Ready { .. }) => return false,
            _ => {}
        }
        let generation = self.advance_host_generation(device_id);
        // This is the attempt a waiting retry was for; a failed one
        // schedules the next.
        if let Some(retry) = self.device_host_retries.get_mut(device_id)
            && retry.at_unix_ms.take().is_some()
        {
            // No retry waits any longer; the panes read what the attempt
            // makes of the link (connecting, or unavailable if it starts
            // nothing).
            self.refresh_pane_headers();
        }
        if self.device_kit_removing(device_id) {
            self.set_host_phase(
                device_id,
                HostPhase::Unavailable(
                    "Hide is still taking its kit off this device; it connects when that is done"
                        .to_owned(),
                ),
            );
            return self.refresh_device_snapshots();
        }
        let Some(consent) = registration.host_consent.clone() else {
            self.set_host_phase(device_id, HostPhase::NotAllowed);
            return self.refresh_device_snapshots();
        };
        if !self.consent_current(&consent) {
            self.set_host_phase(device_id, HostPhase::NotAllowed);
            return self.refresh_device_snapshots();
        }
        let mut consent = self.carried_consent(device_id, consent);
        if consent.cli_dir.is_none() {
            consent.cli_dir = Some(self.host_cli_dir.clone());
        }
        let Some(client) = self
            .remote_connections
            .get(device_id)
            .and_then(|connection| match &connection.transport {
                DeviceReach::Dialed(transport) => Some(Arc::clone(transport)),
                DeviceReach::Inbound(_) => None,
            })
        else {
            self.set_host_phase(
                device_id,
                HostPhase::Unavailable("The device has no SSH connection yet".to_owned()),
            );
            return self.refresh_device_snapshots();
        };
        let retirement_projects = self.retirement_projects(device_id);
        // Leave the app gave while another attempt ran is this attempt's,
        // whatever started it: it is taken only by an attempt that runs.
        let wake = if self.device_wake_pending.remove(device_id) {
            Wake::Allowed
        } else {
            attempt.wake()
        };
        self.spawn_host_start(
            device_id,
            generation,
            HostStart::Dial {
                client,
                consent,
                retirement_projects,
                wake,
            },
        )
    }

    /// The app opened a window on this core: each WSL distribution that is
    /// off or lost is tried with leave to start it (PRD wsl-device B3), and
    /// each SSH device that stopped because signing in needed the operator
    /// is tried with leave to ask (PRD system-ssh-device-link D-06, B10);
    /// one whose attempt is running gets that leave for the attempt after
    /// it, should that one need it.
    pub(super) fn wake_devices_for_app(&mut self) -> bool {
        let woken: Vec<String> = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .filter(|registration| {
                registration.origin.wsl_distro().is_some()
                    || registration.origin.ssh_alias().is_some()
                        && self.device_hosts.get(&registration.id).is_some_and(|host| {
                            matches!(host.phase, HostPhase::Connecting)
                                || host.problem == Some(SshProblem::SignInNeeded)
                        })
            })
            .map(|registration| registration.id.clone())
            .collect();
        let mut changed = false;
        for device_id in woken {
            let ended = self.device_host_ended(&device_id);
            if ended {
                self.device_host_retries.remove(&device_id);
            }
            changed |= self.start_device_host(&device_id, HostAttempt::AppOpened);
        }
        changed
    }

    /// Takes the node a node that dialed this core brought: its link is up
    /// already, so it is ready as soon as the worker takes it.
    pub(super) fn start_arrived_host(
        &mut self,
        device_id: &str,
        node: NodeReady,
        hear_close: Box<dyn FnOnce(OnClose) + Send + 'static>,
    ) -> bool {
        let generation = self.advance_host_generation(device_id);
        self.device_host_retries.remove(device_id);
        self.spawn_host_start(
            device_id,
            generation,
            HostStart::Arrived { node, hear_close },
        )
    }

    /// Runs one connection attempt on a worker, off the runtime lock: a dial
    /// blocks, and a link that already ended tells its end at once, which
    /// takes the lock.
    fn spawn_host_start(&mut self, device_id: &str, generation: u64, start: HostStart) -> bool {
        let host = self.device_host_entry(device_id);
        host.problem = None;
        host.wake = match &start {
            HostStart::Dial { wake, .. } => *wake,
            HostStart::Arrived { .. } => Wake::NotAllowed,
        };
        if matches!(start, HostStart::Dial { .. }) {
            self.begin_device_connect(device_id);
        }
        let Some(context) = self.worker_context.clone() else {
            self.fail_device_connect_install(device_id);
            self.set_host_phase(
                device_id,
                HostPhase::Unavailable("The helper worker is unavailable".to_owned()),
            );
            return self.refresh_device_snapshots();
        };
        self.set_host_phase(device_id, HostPhase::Connecting);
        let device = device_id.to_owned();
        let spawned = thread::Builder::new()
            .name("herdr-core-device-host".to_owned())
            .spawn(move || {
                let close_context = context.clone();
                let close_device = device.clone();
                let on_close: OnClose = Box::new(move |reason: String| {
                    let Some(runtime) = close_context.runtime.upgrade() else {
                        return;
                    };
                    let changed = match runtime.lock() {
                        Ok(mut guard) => {
                            guard.ingest_host_closed(&close_device, generation, reason)
                        }
                        Err(_) => return,
                    };
                    drop(runtime);
                    if changed {
                        close_context.notifier.notify();
                    }
                });
                // An arrived link hears its end only once its node is
                // ready, so a link that ended already is told to a phase
                // that takes it.
                let (result, end) = match start {
                    HostStart::Dial {
                        client,
                        consent,
                        retirement_projects,
                        wake,
                    } => {
                        let attempt = Attempt {
                            wake,
                            sign_in: Arc::new(super::device_sign_in::RowSignIn {
                                context: context.clone(),
                                device_id: device.clone(),
                                generation,
                                wake,
                            }),
                            on_close,
                        };
                        (
                            client
                                .establish(&consent, &retirement_projects, attempt)
                                .map(HostReady::Dialed),
                            None,
                        )
                    }
                    HostStart::Arrived { node, hear_close } => {
                        (Ok(HostReady::Arrived(node)), Some((hear_close, on_close)))
                    }
                };
                if let Ok(HostReady::Dialed(established)) = &result
                    && let Some(Ok(report)) = &established.kit
                {
                    crate::kit::completed(&device, &hide_kit::Scope::automatic(), report);
                }
                let Some(runtime) = context.runtime.upgrade() else {
                    return;
                };
                let changed = match runtime.lock() {
                    Ok(mut guard) => guard.ingest_host_established(&device, generation, result),
                    Err(_) => return,
                };
                drop(runtime);
                if changed {
                    context.notifier.notify();
                }
                if let Some((hear_close, on_close)) = end {
                    hear_close(on_close);
                }
            });
        if let Err(error) = spawned {
            self.fail_device_connect_install(device_id);
            self.set_host_phase(
                device_id,
                HostPhase::Unavailable(format!("The helper worker could not start: {error}")),
            );
        }
        self.refresh_device_snapshots()
    }

    /// Starts the device's link for a read that needs it, unless a lost or
    /// failed one is still waiting out its retry.
    fn reconnect_device_host(&mut self, device_id: &str, now_unix_ms: u64) -> bool {
        // Only the node can bring a node that dials this core back.
        if self
            .link_origin(device_id)
            .is_some_and(|origin| !origin.core_redials())
        {
            return false;
        }
        let waiting = self
            .device_host_retries
            .get(device_id)
            .and_then(|retry| retry.at_unix_ms)
            .is_some_and(|at| now_unix_ms < at);
        !waiting && self.start_device_host(device_id, HostAttempt::Background)
    }

    /// The device's link is asked for now: it is tried at once and its retry
    /// schedule begins over.
    pub(super) fn retry_device_host_now(&mut self, device_id: &str, attempt: HostAttempt) -> bool {
        self.device_host_retries.remove(device_id);
        self.start_device_host(device_id, attempt)
    }

    /// The link was lost or could not start: it is tried again after the
    /// device's current wait, and the wait after that doubles.
    fn schedule_host_retry(&mut self, device_id: &str, now_unix_ms: u64) {
        let retry = self
            .device_host_retries
            .entry(device_id.to_owned())
            .or_default();
        // The session answered while the attempt ran, so the machine can be
        // reached: this failure's retry is due now, the once.
        let wait = if retry.session_back == SessionBack::DuringAttempt {
            retry.session_back = SessionBack::Spent;
            0
        } else {
            retry.delay_ms
        };
        retry.at_unix_ms = Some(now_unix_ms.saturating_add(wait));
        crate::diagnostic!(serde_json::json!({
            "component": "remote_host",
            "kind": "host.retry_scheduled",
            "target": device_id,
            "delay_ms": wait,
        }));
        retry.delay_ms = retry.delay_ms.saturating_mul(2).min(HOST_RETRY_MAX_MS);
        // A pane reads reconnecting once a retry is scheduled.
        self.refresh_pane_headers();
    }

    /// Tries again each device whose lost or failed link has waited out its
    /// retry, so its panes' `hide` comes back without a read asking first.
    pub(crate) fn tick_device_hosts(&mut self, now_unix_ms: u64) -> bool {
        let due: Vec<String> = self
            .device_host_retries
            .iter()
            .filter(|(_, retry)| retry.at_unix_ms.is_some_and(|at| at <= now_unix_ms))
            .map(|(device_id, _)| device_id.clone())
            .collect();
        let mut changed = false;
        for device_id in due {
            if matches!(
                self.device_hosts.get(&device_id).map(|host| &host.phase),
                Some(HostPhase::Unavailable(_))
            ) {
                changed |= self.start_device_host(&device_id, HostAttempt::Background);
            } else if let Some(retry) = self.device_host_retries.get_mut(&device_id) {
                retry.at_unix_ms = None;
            }
        }
        changed
    }

    fn device_host_entry(&mut self, device_id: &str) -> &mut DeviceHost {
        self.device_hosts
            .entry(device_id.to_owned())
            .or_insert_with(|| DeviceHost::new(HostPhase::NotAllowed, 0))
    }

    fn set_host_phase(&mut self, device_id: &str, phase: HostPhase) {
        let waiting = self.helper_link_wait(device_id);
        let linked = self.linked_node(device_id).is_ok();
        self.device_host_entry(device_id).phase = phase;
        // A pane of the device says what its link waits for, so its notice
        // follows the phase.
        if waiting != self.helper_link_wait(device_id) {
            self.refresh_pane_headers();
        }
        // Hide AI's rows of the device's agents follow its link (B13).
        if linked != self.linked_node(device_id).is_ok() {
            self.refresh_background_ai();
        }
    }

    /// What the device's helper link waits for while it is away: an attempt
    /// that runs or a retry that is scheduled, or the operator has to allow
    /// the helper. A changed identity or an unsupported device is neither, and
    /// its panes stay unavailable.
    pub(super) fn helper_link_wait(&self, device_id: &str) -> Option<LinkWait> {
        if !self
            .link_origin(device_id)
            .is_some_and(LinkOrigin::core_redials)
        {
            return None;
        }
        match self.device_hosts.get(device_id).map(|host| &host.phase) {
            Some(HostPhase::Connecting) => Some(LinkWait::Returns),
            // Only a retry that is scheduled brings an unavailable link back;
            // the paths that schedule none (no SSH connection yet, the kit
            // still coming off, no worker) wait for something else.
            Some(HostPhase::Unavailable(_))
                if self
                    .device_host_retries
                    .get(device_id)
                    .is_some_and(|retry| retry.at_unix_ms.is_some()) =>
            {
                Some(LinkWait::Returns)
            }
            Some(HostPhase::NotAllowed) => Some(LinkWait::NeedsConsent),
            _ => None,
        }
    }

    /// The device's Herdr session answers again, so the machine is reachable:
    /// the helper link's retry is due now, without waiting out a wait that
    /// grew while the machine was away. The delay after it keeps doubling,
    /// and this happens once per outage: a session that flaps while the
    /// helper keeps failing is still dialed no oftener than the schedule's
    /// cap allows. A link that is ready, or waits for the operator, is left
    /// as it is.
    pub(super) fn device_reachable_again(&mut self, device_id: &str) -> bool {
        if !self
            .link_origin(device_id)
            .is_some_and(LinkOrigin::core_redials)
        {
            return false;
        }
        let retry = self
            .device_host_retries
            .entry(device_id.to_owned())
            .or_default();
        if retry.session_back == SessionBack::Spent {
            return false;
        }
        match self.device_hosts.get(device_id).map(|host| &host.phase) {
            Some(HostPhase::Unavailable(_)) => {
                retry.session_back = SessionBack::Spent;
                crate::diagnostic!(serde_json::json!({
                    "component": "remote_host",
                    "kind": "host.retry_due_session_back",
                    "target": device_id,
                }));
                self.start_device_host(device_id, HostAttempt::Background)
            }
            Some(HostPhase::Connecting) => {
                retry.session_back = SessionBack::DuringAttempt;
                false
            }
            _ => {
                // Nothing waits for the session; the entry only stands for it.
                if *retry == HostRetry::default() {
                    self.device_host_retries.remove(device_id);
                }
                false
            }
        }
    }

    pub(super) fn advance_host_generation(&mut self, device_id: &str) -> u64 {
        self.last_host_generation += 1;
        let generation = self.last_host_generation;
        let host = self.device_host_entry(device_id);
        host.generation = generation;
        // A question of the attempt this one replaces answers nothing now:
        // its channel goes with it, which ends the wait.
        host.sign_in = None;
        generation
    }

    pub(super) fn ingest_host_established(
        &mut self,
        device_id: &str,
        generation: u64,
        result: Result<HostReady, EstablishError>,
    ) -> bool {
        let current = self
            .device_hosts
            .get(device_id)
            .is_some_and(|host| host.generation == generation);
        if !current {
            if let Ok(ready) = result {
                ready.node().host.close("superseded by a newer attempt");
            }
            return false;
        }
        // A link that ended while the attempt still ran told its end to a
        // phase that takes none (`ingest_host_closed`), and storing it ready
        // would leave a dead host nobody retries. It is the same lost link,
        // read here: a dialed link says so through its host, and an arrived
        // one is told its end after this (`spawn_host_start`).
        let result = match result {
            Ok(HostReady::Dialed(established)) => match established.node.host.closed_reason() {
                Some(reason) => Err(EstablishError::Helper(format!(
                    "The device helper disconnected: {reason}"
                ))),
                None => Ok(HostReady::Dialed(established)),
            },
            other => other,
        };
        match result {
            Ok(ready) => {
                let (node, dialed, kit) = match ready {
                    HostReady::Dialed(established) => {
                        let Established {
                            node,
                            identity,
                            installed,
                            helper_path,
                            upload,
                            kit,
                        } = established;
                        (node, Some((identity, installed, helper_path, upload)), kit)
                    }
                    HostReady::Arrived(node) => (node, None, None),
                };
                self.ingest_device_machine_id(
                    device_id,
                    node.hello.machine_identity.clone().into_result(),
                );
                let platform = format!("{} {}", node.hello.os, node.hello.arch);
                self.device_connect_platform(device_id, &node.hello.os, &node.hello.arch);
                let mut bound_now = false;
                let mut helper_path = None;
                if let Some((identity, installed, path, upload)) = dialed {
                    let mut machine_carried = false;
                    if let Some(consent) = self
                        .snapshot
                        .ui_state
                        .device_registrations
                        .iter_mut()
                        .find(|registration| registration.id == device_id)
                        .and_then(|registration| registration.host_consent.as_mut())
                    {
                        match &mut consent.identity {
                            None => {
                                consent.identity = Some(identity);
                                bound_now = true;
                            }
                            // A consent bound before contract 4 names no
                            // machine; the one that answered as its account,
                            // address and port is the one it was given for,
                            // and from now on the only one (D-09, B16).
                            Some(bound)
                                if bound.machine_id.is_none() && identity.machine_id.is_some() =>
                            {
                                bound.machine_id = identity.machine_id;
                                machine_carried = true;
                            }
                            Some(_) => {}
                        }
                    }
                    if bound_now || machine_carried {
                        self.persist_current_ui_state();
                    }
                    if machine_carried {
                        crate::diagnostic!(serde_json::json!({
                            "component": "remote_host",
                            "kind": "host.consent_machine_bound",
                            "target": device_id,
                            "generation": generation,
                        }));
                    }
                    crate::diagnostic!(serde_json::json!({
                        "component": "remote_host",
                        "kind": "host.ready",
                        "target": device_id,
                        "generation": generation,
                        "installed": installed,
                        "platform": platform,
                        "consent_bound": bound_now,
                        "upload": upload,
                    }));
                    helper_path = Some(path);
                }
                self.device_host_retries.remove(device_id);
                // Connected: leave the app gave meanwhile has nothing left to
                // start, and must not start the distribution after a later end.
                self.device_wake_pending.remove(device_id);
                let host = self.device_host_entry(device_id);
                host.problem = None;
                host.sign_in = None;
                self.set_host_phase(
                    device_id,
                    HostPhase::Ready {
                        host: node.host,
                        platform,
                        helper_path,
                        windows_drives: node.hello.windows_drives.clone(),
                    },
                );
                // A WSL distribution's Herdr rides this link: its session
                // reads again now rather than at its next reconnect.
                if self
                    .link_origin(device_id)
                    .is_some_and(|origin| origin.wsl_distro().is_some())
                    && let Some(sync) = self
                        .remote_connections
                        .get(device_id)
                        .and_then(|connection| connection.sync.as_ref())
                {
                    sync.republish_waker().refresh_topology();
                }
                match node.terminals {
                    Ok(terminals) => {
                        self.terminals.install_device(device_id, terminals);
                        // The device's panes on screen attach inside the new
                        // link (D-19, B17).
                        let prefix = remote_pane_id_prefix(device_id);
                        self.terminal_states
                            .retain(|pane_id, _| !pane_id.starts_with(&prefix));
                        self.terminal_attach_requested
                            .retain(|pane_id| !pane_id.starts_with(&prefix));
                        self.reconcile_remote_terminal_selection();
                    }
                    Err(reason) => self.terminals.terminals_unstarted(device_id, &reason),
                }
                self.settle_device_saves(device_id);
                // The device's hooks read the protections this core holds
                // for its projects, whatever its file said before (D-28).
                self.publish_guardrail_policy(device_id);
                self.reread_device_facts(device_id);
                self.relist_remote_files(device_id);
                // A device Workspace in front waited for this helper to
                // bring its View tabs back.
                self.restore_front_when_ready();
                self.home_helper_ready(device_id);
                // Every connection brings the machine's kit up to this build
                // without asking (B10, B13, B19): a device's from the helper
                // this core installed, a node's that dials this core from
                // the app or build it runs, through its link.
                if let Some(report) = kit {
                    let report = report.map(|report| *report);
                    let token = self.claim_device_connect_kit(device_id);
                    self.complete_device_connect_kit(device_id, token, &report);
                    // A read or automatic pass that waited for this link
                    // is already answered. Keep an explicit repair or
                    // agent choice, and start it after the bootstrap.
                    if report.is_ok()
                        && self.device_kit_pending.get(device_id).is_some_and(|job| {
                            *job == super::KitJob::Status
                                || *job == super::KitJob::Apply(hide_kit::Scope::automatic())
                        })
                    {
                        self.device_kit_pending.remove(device_id);
                    }
                    self.ingest_device_kit_answer(
                        device_id,
                        super::DeviceKitAnswer::Report(report),
                        None,
                    );
                    self.start_device_kit(device_id);
                } else {
                    self.queue_device_kit(
                        device_id,
                        super::KitJob::Apply(hide_kit::Scope::automatic()),
                    );
                }
            }
            Err(error) => {
                self.fail_device_connect(device_id, &error);
                let message = error.to_string();
                crate::diagnostic!(serde_json::json!({
                    "component": "remote_host",
                    "kind": "host.failed",
                    "target": device_id,
                    "generation": generation,
                    "message": message,
                }));
                self.release_held_saves(device_id, &message);
                // Another machine answers the device's address: what the
                // last one said about its directories is not this one's.
                if matches!(error, EstablishError::IdentityChanged { .. }) {
                    self.forget_device_directories(device_id);
                }
                self.device_facts
                    .entry(device_id.to_owned())
                    .or_default()
                    .unavailable = Some(message.clone());
                // A changed identity, an unsupported device or an ssh
                // failure only the operator can change wait for the operator
                // and drop leave the app gave meanwhile, except a sign-in
                // that needed someone to ask, which the app's leave tries
                // again at once, as it does a WSL distribution that is off;
                // anything else may pass, so it is tried again, and the
                // retry takes the app's leave.
                let needs_operator = matches!(
                    &error,
                    EstablishError::Ssh(SshFailure {
                        problem: SshProblem::SignInNeeded,
                        ..
                    })
                );
                let wake_next = (matches!(error, EstablishError::Stopped(_)) || needs_operator)
                    && self.device_wake_pending.contains(device_id);
                let ssh_device = self
                    .link_origin(device_id)
                    .is_some_and(|origin| origin.ssh_alias().is_some());
                let problem = match &error {
                    EstablishError::Ssh(failure) => Some(failure.problem),
                    EstablishError::Unsupported(_) if ssh_device => Some(SshProblem::Unsupported),
                    _ => None,
                };
                if matches!(
                    &error,
                    EstablishError::IdentityChanged { .. } | EstablishError::Unsupported(_)
                ) || problem.is_some_and(|problem| !problem.retried()) && !needs_operator
                {
                    self.device_wake_pending.remove(device_id);
                }
                self.device_host_entry(device_id).problem = problem;
                let phase = match error {
                    EstablishError::IdentityChanged { .. } => HostPhase::IdentityChanged(message),
                    EstablishError::Unsupported(_) => HostPhase::Unsupported(message),
                    EstablishError::Stopped(_) => HostPhase::Unavailable(message),
                    // A refused sign-in, a changed server key or a missing
                    // ssh would only be refused again: the operator's retry
                    // tries it (B7, B13, B14, B18).
                    EstablishError::Ssh(failure) if !failure.problem.retried() => {
                        HostPhase::Unavailable(message)
                    }
                    _ => {
                        if self
                            .link_origin(device_id)
                            .is_some_and(LinkOrigin::core_redials)
                        {
                            self.schedule_host_retry(device_id, now_unix_ms());
                        }
                        HostPhase::Unavailable(message)
                    }
                };
                self.set_host_phase(device_id, phase);
                self.refresh_device_catalog(device_id);
                self.clear_kit_busy(device_id);
                if wake_next {
                    self.start_device_host(device_id, HostAttempt::AppOpened);
                }
            }
        }
        self.refresh_device_snapshots();
        true
    }

    pub(super) fn device_host_generation(&self, device_id: &str) -> u64 {
        self.device_hosts
            .get(device_id)
            .map_or(0, |host| host.generation)
    }

    /// Whether the device's link was up and has ended: its helper reports
    /// unavailable, or the channel it was ready on has closed.
    pub(super) fn device_host_ended(&self, device_id: &str) -> bool {
        match self.device_hosts.get(device_id).map(|host| &host.phase) {
            Some(HostPhase::Unavailable(_)) => true,
            Some(HostPhase::Ready { host, .. }) => host.closed_reason().is_some(),
            _ => false,
        }
    }

    pub(super) fn device_host_connecting(&self, device_id: &str) -> bool {
        matches!(
            self.device_hosts.get(device_id).map(|host| &host.phase),
            Some(HostPhase::Connecting)
        )
    }

    pub(super) fn ingest_host_closed(
        &mut self,
        device_id: &str,
        generation: u64,
        reason: String,
    ) -> bool {
        let Some(host) = self.device_hosts.get(device_id) else {
            return false;
        };
        if host.generation != generation || !matches!(host.phase, HostPhase::Ready { .. }) {
            return false;
        }
        self.fail_device_connect_install(device_id);
        self.set_host_phase(
            device_id,
            HostPhase::Unavailable(format!("The device helper disconnected: {reason}")),
        );
        // A node that dials this core comes back by dialing again.
        if self
            .link_origin(device_id)
            .is_some_and(LinkOrigin::core_redials)
        {
            self.schedule_host_retry(device_id, now_unix_ms());
        }
        self.end_device_terminals(
            device_id,
            &format!("The device helper disconnected: {reason}"),
        );
        self.drop_queued_codex_daemon_off(device_id);
        if self.device_machine_ids.remove(device_id).is_some() {
            self.refresh_agent_lineage();
        }
        self.refresh_device_snapshots();
        true
    }

    /// Ends the helper connection, if any. Work still waiting settles as
    /// unknown in its own caller.
    pub(super) fn close_device_host(&mut self, device_id: &str, reason: &str) {
        self.forget_device_connect(device_id);
        if self.device_hosts.contains_key(device_id) {
            self.advance_host_generation(device_id);
            let host = self.device_host_entry(device_id);
            let closing =
                std::mem::replace(&mut host.phase, HostPhase::Unavailable(reason.to_owned()));
            if let HostPhase::Ready { host: remote, .. } = closing {
                remote.close(reason);
            }
        }
        self.end_device_terminals(device_id, reason);
        self.refresh_pane_headers();
        self.drop_queued_codex_daemon_off(device_id);
        if self.device_machine_ids.remove(device_id).is_some() {
            self.refresh_agent_lineage();
        }
    }

    /// The device's link ended: every terminal flow inside it ended too, so
    /// its panes read unavailable and refuse keys until a link is back
    /// (D-19, B17).
    fn end_device_terminals(&mut self, device_id: &str, reason: &str) {
        self.terminals.remove_device(device_id);
        let prefix = remote_pane_id_prefix(device_id);
        let panes = self
            .terminal_states
            .iter_mut()
            .filter(|(pane_id, _)| pane_id.starts_with(&prefix));
        let mut ended = Vec::new();
        for (pane_id, state) in panes {
            state.state = "unavailable".to_owned();
            state.mode = None;
            state.message = Some(reason.to_owned());
            state.exit_category = Some("device_unavailable".to_owned());
            state.retry_decision = "manual".to_owned();
            ended.push(pane_id.clone());
        }
        for pane_id in ended {
            self.sync_transport_projection(&pane_id);
        }
    }

    pub(super) fn forget_device_host(&mut self, device_id: &str, why: Retiring) {
        self.close_device_host(
            device_id,
            match why {
                Retiring::Removed => "device removed",
                Retiring::Retried => "device reconnecting",
            },
        );
        self.device_hosts.remove(device_id);
        self.device_host_retries.remove(device_id);
        self.device_wake_pending.remove(device_id);
        // A row on this device no longer waits for its helper's answer.
        self.row_registrations
            .retain(|_, pending| pending.device_id != device_id);
        self.codex_daemon_off_running.remove(device_id);
        self.refresh_background_ai();
        self.forget_guardrails_device(device_id);
        match why {
            Retiring::Removed => {
                self.forget_device_catalog(device_id);
                self.device_kit_pending.remove(device_id);
                self.device_first_run_choice.remove(device_id);
                self.kit_states.remove(device_id);
            }
            // Work queued for the device runs on the next link; the row stops
            // saying it is working until a worker can run it.
            Retiring::Retried => self.clear_kit_busy(device_id),
        }
    }

    /// The link to the machine this core runs on.
    pub(crate) fn own_node(&self) -> Arc<dyn NodeLink> {
        Arc::clone(&self.own_node)
    }

    /// Where Hide AI's agents run: this node, and each connected device,
    /// looked up when its agent is asked and never woken (PRD wsl-device
    /// B13). Asked off the runtime's lock, as every provider call is.
    pub(crate) fn ai_machines(&self) -> crate::ai::AiMachines {
        crate::ai::AiMachines {
            own: self.own_node(),
            devices: device_links(
                self.worker_context
                    .as_ref()
                    .map_or_else(Weak::new, |context| context.runtime.clone()),
            ),
        }
    }

    /// The link to the node `device_id` names, or the sentence that says why
    /// it cannot take work now. The core's own node answers in process; an SSH
    /// device answers through its helper, and an unavailable helper on a
    /// consented device is asked for again here, so the next action finds it
    /// ready.
    pub(crate) fn node_link(&mut self, device_id: &str) -> Result<Arc<dyn NodeLink>, String> {
        if device_id == self.node.as_str() {
            return Ok(self.own_node());
        }
        match self.device_hosts.get(device_id).map(|host| &host.phase) {
            Some(HostPhase::Ready { host, .. }) if host.closed_reason().is_none() => {
                return Ok(Arc::clone(host));
            }
            Some(HostPhase::Ready { .. }) | Some(HostPhase::Unavailable(_)) | None => {
                self.reconnect_device_host(device_id, now_unix_ms());
            }
            _ => {}
        }
        Err(self
            .host_snapshot(device_id)
            .message
            .unwrap_or_else(|| "The device helper is not ready".to_owned()))
    }

    /// The node that does a Project's work on `device` for what the core
    /// keeps, with its link: the core's own node or any registered device
    /// while its link is up. Link direction grants no feature authority;
    /// Factory admission and reader support are checked by their owners.
    /// Asking starts no helper.
    pub(crate) fn linked_node(&self, device: &str) -> Result<LinkedNode, &'static str> {
        if device == self.node.as_str() {
            return Ok((self.node.clone(), self.own_node()));
        }
        self.snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.id == device)
            .ok_or("unregistered")?;
        let node = crate::node::NodeId::parse(device).map_err(|_| "unregistered")?;
        match self.device_hosts.get(device).map(|host| &host.phase) {
            Some(HostPhase::Ready { host, .. }) if host.closed_reason().is_none() => {
                Ok((node, Arc::clone(host)))
            }
            _ => Err("link_down"),
        }
    }

    /// A Memory-capable node, independent of who opened the connection.
    /// An older helper without the conversation-based Memory reader is an
    /// explicit refusal, never a successfully read empty Project.
    pub(crate) fn memory_node(&self, device: &str) -> Result<LinkedNode, &'static str> {
        let (node, link) = self.linked_node(device)?;
        let readers = link
            .reader_features()
            .ok_or("session_reader_support_unavailable")?;
        if !hide_session::Agent::supported().any(|agent| {
            readers.supports(
                agent.as_str(),
                hide_node_link::sessions::ReaderFeature::Memory,
            ) && readers.supports(
                agent.as_str(),
                hide_node_link::sessions::ReaderFeature::Conversation,
            )
        }) {
            return Err("session_reader_memory_unsupported");
        }
        Ok((node, link))
    }

    /// The machines whose Projects have Memory now, each with its own file
    /// reader, and registered nodes waiting for their link or reader.
    pub(crate) fn memory_nodes(&mut self) -> (Vec<LinkedNode>, Vec<crate::node::NodeId>) {
        let mut nodes = vec![(self.node.clone(), self.own_node())];
        let mut unreached = Vec::new();
        let previous = std::mem::take(&mut self.memory_work.node_refusals);
        let mut refusals = HashMap::new();
        for registration in &self.snapshot.ui_state.device_registrations {
            match self.memory_node(&registration.id) {
                Ok(linked) => nodes.push(linked),
                Err(reason) => {
                    if reason != "link_down" && previous.get(&registration.id) != Some(&reason) {
                        crate::diagnostic!(serde_json::json!({
                            "component": "memory", "kind": "analysis.node_unavailable",
                            "device": registration.id, "reason": reason,
                        }));
                    }
                    refusals.insert(registration.id.clone(), reason);
                    unreached.extend(crate::node::NodeId::parse(&registration.id).ok());
                }
            }
        }
        self.memory_work.node_refusals = refusals;
        (nodes, unreached)
    }

    /// What a hook's Memory read for a pane in `context` is answered from
    /// (`crate::memory_hook`), on the node of the pane's checkout.
    pub(crate) fn memory_scope(
        &self,
        context: &crate::workspace_control::Context,
    ) -> Result<crate::memory_hook::Scope, &'static str> {
        let (node, _) = self.memory_node(&context.device_id)?;
        // These facts already belong to the catalog's background owner. A
        // disconnected or unconfirmed catalog never triggers hook discovery.
        let project = if node == self.node {
            let paths = &self.catalog_paths;
            if !paths.knows(&context.checkout_path) || !paths.exists(&context.checkout_path) {
                return Err("project_unresolved");
            }
            let facts = paths
                .project(&context.checkout_path)
                .ok_or("project_unresolved")?;
            hide_project::ProjectIdentity {
                id: hide_project::project_id(node.as_str(), &facts.root),
                root: facts.root.clone(),
                checkout_root: facts.checkout_root.clone(),
                device_id: node.as_str().into(),
                kind: facts.kind,
            }
        } else {
            let facts = self
                .device_facts
                .get(&context.device_id)
                .filter(|facts| !facts.reread && facts.unavailable.is_none())
                .ok_or("project_unresolved")?;
            let Some(crate::device_catalog::Fact::Known(facts)) =
                facts.facts.get(&context.checkout_path)
            else {
                return Err("project_unresolved");
            };
            hide_project::ProjectIdentity {
                id: hide_project::project_id(node.as_str(), &facts.root),
                root: facts.root.clone(),
                checkout_root: facts.checkout_root.clone(),
                device_id: node.as_str().into(),
                kind: facts.kind,
            }
        };
        let task_context = self
            .snapshot
            .navigator
            .workspaces
            .iter()
            .filter(|workspace| workspace.device_id == context.device_id)
            .flat_map(|workspace| &workspace.checkouts)
            .find(|checkout| checkout.path == context.checkout_path)
            .and_then(|checkout| checkout.purpose.as_ref())
            .filter(|purpose| {
                matches!(
                    purpose.origin,
                    crate::model::CheckoutPurposeOrigin::Token
                        | crate::model::CheckoutPurposeOrigin::BranchDescription
                )
            })
            .map(|purpose| hide_memory::hook::prefix(&purpose.text, 1024).to_owned());
        Ok(crate::memory_hook::Scope {
            store: self.memory_database_path(),
            project,
            task_context,
        })
    }

    /// The devices whose helper is connected now, each with its channel;
    /// asking starts no helper (PRD link-graph D-21).
    pub(crate) fn ready_device_channels(&self) -> Vec<(String, Arc<dyn NodeLink>)> {
        let mut ready = self
            .device_hosts
            .iter()
            .filter_map(|(device, host)| match &host.phase {
                HostPhase::Ready { host, .. } if host.closed_reason().is_none() => {
                    Some((device.clone(), Arc::clone(host)))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        ready.sort_by(|left, right| left.0.cmp(&right.0));
        ready
    }

    /// The host row for one device, read by Settings and by every file or
    /// Git surface that needs to say why it cannot act.
    pub(super) fn host_snapshot(&self, device_id: &str) -> DeviceHostSnapshot {
        if self.link_origin(device_id) == Some(&LinkOrigin::Inbound) {
            return self.inbound_host_snapshot(device_id);
        }
        let consent = self
            .device_registration(device_id)
            .and_then(|registration| registration.host_consent.as_ref());
        let current = consent.is_some_and(|consent| self.consent_current(consent));
        let mut snapshot = DeviceHostSnapshot {
            consent: match consent {
                None => "none",
                Some(_) if current => "granted",
                Some(_) => "outdated",
            }
            .to_owned(),
            helper_root: Some(
                consent
                    .filter(|_| current)
                    .map(|consent| self.consent_root(consent))
                    .unwrap_or_else(|| self.host_helper_root()),
            ),
            cli_dir: Some(
                consent
                    .filter(|_| current)
                    .and_then(|consent| consent.cli_dir.clone())
                    .unwrap_or_else(|| self.host_cli_dir.clone()),
            ),
            contract: HOST_CONSENT_CONTRACT,
            bound_identity: consent
                .and_then(|consent| consent.identity.as_ref())
                .map(|identity| identity.describe()),
            granted_at_unix_ms: consent.map(|consent| consent.granted_at_unix_ms),
            state: "not_allowed".to_owned(),
            message: None,
            platform: None,
            helper_path: None,
            windows_drives: None,
        };
        let phase = self.device_hosts.get(device_id).map(|host| &host.phase);
        let (state, message) = match (consent, current, phase) {
            (None, _, _) => (
                "not_allowed",
                Some("Allow Hide's helper on this device in Settings to use its files and Git".to_owned()),
            ),
            (Some(_), false, _) => (
                "not_allowed",
                Some("This version of Hide needs a wider permission on this device; review and allow it again in Settings".to_owned()),
            ),
            (Some(_), true, Some(HostPhase::Ready { host, platform, helper_path, windows_drives })) => {
                snapshot.platform = Some(platform.clone());
                snapshot.helper_path = helper_path.clone();
                snapshot.windows_drives = windows_drives.clone();
                match host.closed_reason() {
                    None => ("ready", None),
                    Some(reason) => ("unavailable", Some(format!("The device helper disconnected: {reason}"))),
                }
            }
            (Some(_), true, Some(HostPhase::Connecting)) => ("connecting", Some("Connecting to the device helper…".to_owned())),
            (Some(_), true, Some(HostPhase::IdentityChanged(message))) => ("identity_changed", Some(message.clone())),
            (Some(_), true, Some(HostPhase::Unsupported(message))) => ("unsupported", Some(message.clone())),
            (Some(_), true, Some(HostPhase::Unavailable(message))) => ("unavailable", Some(message.clone())),
            (Some(_), true, Some(HostPhase::NotAllowed) | None) => (
                "unavailable",
                Some("The device helper has not connected yet".to_owned()),
            ),
        };
        snapshot.state = state.to_owned();
        snapshot.message = message;
        snapshot
    }

    /// The host row of a node that dials this core: its link's state, with
    /// the operator's SSH login as its consent and no helper of the core's.
    fn inbound_host_snapshot(&self, device_id: &str) -> DeviceHostSnapshot {
        let phase = self.device_hosts.get(device_id).map(|host| &host.phase);
        let windows_drives = match phase {
            Some(HostPhase::Ready { windows_drives, .. }) => windows_drives.clone(),
            _ => None,
        };
        let (state, message, platform) = match phase {
            Some(HostPhase::Ready { host, platform, .. }) => match host.closed_reason() {
                None => ("ready", None, Some(platform.clone())),
                Some(reason) => (
                    "unavailable",
                    Some(format!("The machine's link ended: {reason}")),
                    Some(platform.clone()),
                ),
            },
            Some(HostPhase::Connecting) => (
                "connecting",
                Some("Connecting to the machine…".to_owned()),
                None,
            ),
            Some(
                HostPhase::Unavailable(message)
                | HostPhase::IdentityChanged(message)
                | HostPhase::Unsupported(message),
            ) => ("unavailable", Some(message.clone()), None),
            Some(HostPhase::NotAllowed) | None => (
                "unavailable",
                Some("The machine has not connected to this core yet".to_owned()),
                None,
            ),
        };
        DeviceHostSnapshot {
            consent: "granted".to_owned(),
            helper_root: None,
            cli_dir: None,
            contract: HOST_CONSENT_CONTRACT,
            bound_identity: None,
            granted_at_unix_ms: None,
            state: state.to_owned(),
            message,
            platform,
            helper_path: None,
            windows_drives,
        }
    }

    /// The install root and the command folder a consent names.
    pub(super) fn helper_places_from(options: &CoreOptions) -> (String, String) {
        (
            options
                .host_helper_root
                .clone()
                .unwrap_or_else(|| crate::remote::DEFAULT_HELPER_ROOT.to_owned()),
            options
                .host_cli_dir
                .clone()
                .unwrap_or_else(|| crate::remote::DEFAULT_CLI_DIR.to_owned()),
        )
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

/// A device's link as [`Runtime::linked_node`] answers it, with the host
/// generation every new link of the device advances, looked up under a
/// brief lock of `runtime`; `None` for a device that is not connected or a
/// runtime that is gone. Never called under that lock.
pub(crate) fn device_links(runtime: Weak<Mutex<Runtime>>) -> crate::ai::DeviceLinks {
    Arc::new(move |device: &str| {
        let runtime = runtime.upgrade()?;
        let guard = runtime.lock().ok()?;
        let (_, link) = guard.linked_node(device).ok()?;
        Some(crate::ai::DeviceLink {
            generation: guard.device_host_generation(device),
            link,
        })
    })
}
