//! Bootstrap of short-lived Workspace credentials. This local socket does
//! kernel peer attestation and credential issuance, and the bounded hook
//! reads; other commands use `/ws`.
//!
//! A caller is bound one of two ways. A process descending from the shell
//! Herdr runs for the named pane gets a pane-bound capability that records
//! the pane's terminal identity and shell birth and dies with the pane. A
//! local process that is not such a descendant (a Codex tool shell run by
//! the shared `codex app-server` daemon, whose parent is launchd) is bound
//! instead to the registered, connected checkout that contains its working
//! directory, read from the kernel for the peer pid and never from the
//! request; that capability is rechecked only for registration and device
//! connection. Both hold the same Workspace commands for the same checkout.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use herdr_core::workspace_control::{Caller, Context, Query, checkout_caller_id};
use hide_node::pane_proof::{PaneIdentity, caller_directory, descends_from, process_start};
use hide_node::ssh::RemoteHost;
use hide_node_link::panes::StreamCaller;
use hide_node_link::protocol::Call;
use hide_platform::fs::private;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::core::CoreHandle;
use crate::state_file::new_token;

const MAX_CAPABILITIES: usize = 64;
const CAPABILITY_LIFETIME: Duration = Duration::from_secs(8 * 60 * 60);
pub use hide_node_link::panes::{CHECKOUT_NEXT_ACTION, PANE_NEXT_ACTION, refusal_next_action};
const UNCLAIMED_LIFETIME: Duration = Duration::from_secs(30);
const MAX_BOOTSTRAPS: usize = 8;
const BOOTSTRAP_TIMEOUT: Duration = Duration::from_secs(5);

/// How a capability was bound to its caller, and what its validation rechecks.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Binding {
    Pane {
        terminal_id: String,
        shell_pid: i32,
        shell_started: u64,
    },
    Checkout,
}

#[derive(Clone)]
pub struct Capability {
    /// The caller id the core resolves: a pane id, or the encoded checkout
    /// caller (`workspace_control::Caller`) for a checkout-bound capability.
    pub pane_id: String,
    pub context: Context,
    binding: Binding,
    pub one_shot: bool,
    holder: Option<(i32, u64)>,
    created: Instant,
    claimed: bool,
    path: PathBuf,
    remote: Option<RemoteCapability>,
}

impl Capability {
    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        (self.binding == Binding::Checkout || self.pane_id == other.pane_id)
            && self.context == other.context
            && self.binding == other.binding
    }
    pub fn question_guard_terminal(&self) -> Option<&str> {
        match &self.binding {
            Binding::Pane { terminal_id, .. } => Some(terminal_id),
            Binding::Checkout => None,
        }
    }

    /// Whether this credential was issued for a pane of `node`, vouched for
    /// over `link`; a local one never was.
    pub fn vouched_by(&self, node: &str, link: &RemoteHost) -> bool {
        self.remote
            .as_ref()
            .is_some_and(|remote| remote.node == node && remote.link.same_link(link))
    }

    pub fn question_guard_source_matches(&self, source: Option<(&str, &RemoteHost)>) -> bool {
        match (&self.remote, source) {
            (None, None) => true,
            (Some(_), Some((node, link))) => self.vouched_by(node, link),
            _ => false,
        }
    }

    /// The refusal for a command whose answer no longer matches this
    /// credential's context: a pane moved or closed, or a checkout that is
    /// no longer registered on its device.
    pub fn changed_refusal(&self) -> (&'static str, &'static str) {
        match self.binding {
            Binding::Pane { .. } => ("pane_changed", PANE_NEXT_ACTION),
            Binding::Checkout => ("checkout_not_registered", CHECKOUT_NEXT_ACTION),
        }
    }
}

/// A device pane's credential: the node that vouched for the pane and the
/// link it vouched over. It holds only while that link does (B18).
#[derive(Clone)]
struct RemoteCapability {
    node: String,
    link: RemoteHost,
    source_pane_id: String,
}

pub(crate) struct RemoteGrant {
    pub node: String,
    pub link: RemoteHost,
    pub source_pane_id: String,
    pub one_shot: bool,
}

/// How long a device's node may take to re-read a pane for a command.
const PANE_INSPECT_TIMEOUT: Duration = Duration::from_secs(10);

/// What a device's node proved about the process that opened a command's
/// stream (`NodeEvent::StreamOpen`), with the node and link it came over.
pub struct StreamAttestation {
    pub node: String,
    pub link: RemoteHost,
    pub caller: StreamCaller,
    /// Receipt on this core's clock. A delayed handshake or executor job
    /// must not reuse the stream-open proof past a normal inspection budget.
    pub received: Instant,
}

/// The identity of a device credential's pane that a command is checked
/// against: the one its node read when the command's stream opened, when
/// that came over the link that vouched for the credential and names the
/// credential's pane; otherwise the node's answer to `Call::PaneInspect`
/// now, within `timeout`.
fn remote_identity(
    cap: &Capability,
    remote: &RemoteCapability,
    attested: Option<&StreamAttestation>,
    timeout: Duration,
) -> Result<PaneIdentity, &'static str> {
    if let Some(attested) = attested.filter(|attested| {
        cap.vouched_by(&attested.node, &attested.link)
            && attested.caller.pane_id == remote.source_pane_id
            && attested.received.elapsed() < PANE_INSPECT_TIMEOUT
    }) {
        return Ok(attested.caller.identity.clone());
    }
    hide_node_link::call_as(
        &remote.link,
        Call::PaneInspect {
            pane_id: remote.source_pane_id.clone(),
        },
        timeout,
    )
    .map_err(|_| "remote_unavailable")
}

fn pane_binding(identity: PaneIdentity) -> Binding {
    Binding::Pane {
        terminal_id: identity.terminal_id,
        shell_pid: identity.shell_pid,
        shell_started: identity.shell_started,
    }
}

fn entry_alive(entry: &Capability) -> bool {
    if entry.created.elapsed() >= CAPABILITY_LIFETIME
        || (!entry.claimed && entry.created.elapsed() >= UNCLAIMED_LIFETIME)
    {
        return false;
    }
    if let Some(remote) = &entry.remote {
        return remote.link.closed_reason().is_none();
    }
    let shell_alive = match &entry.binding {
        Binding::Pane {
            shell_pid,
            shell_started,
            ..
        } => process_start(*shell_pid) == Some(*shell_started),
        Binding::Checkout => true,
    };
    entry.path.is_file()
        && shell_alive
        && entry
            .holder
            .is_none_or(|(pid, born)| process_start(pid) == Some(born))
}

/// A persistent bootstrap from the same caller reuses its live capability. A
/// pane caller is the same pane with the same shell; a checkout caller is any
/// caller bound to the same checkout, whatever nonce its bootstrap carried.
fn same_caller(entry: &Capability, attestation: &Attestation) -> bool {
    entry.context == attestation.context
        && entry.binding == attestation.binding
        && match attestation.binding {
            Binding::Pane { .. } => entry.pane_id == attestation.pane_id,
            Binding::Checkout => true,
        }
}

fn remove_local_reference(entry: &Capability) {
    if entry.remote.is_none() {
        let _ = fs::remove_file(&entry.path);
        let _ = fs::remove_file(entry.path.with_extension("claimed"));
    }
}

/// What a reference file holds: the credential, and the way to the daemon
/// that issued it.
#[derive(Serialize, Deserialize)]
pub struct Reference {
    pub token: String,
    #[serde(flatten)]
    pub route: Route,
}

/// How a command reaches the daemon that issued its credential: this
/// machine's loopback port, or, on a device, the node socket whose link to
/// that daemon carries the command (`hide_host::panes`).
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Route {
    Daemon { port: u16, origin_port: u16 },
    Node { socket: String },
}

pub struct Registry {
    directory: PathBuf,
    entries: Mutex<HashMap<String, Capability>>,
    closed: AtomicBool,
}

impl Registry {
    pub fn new(state_dir: &Path) -> Result<Self, String> {
        let directory = state_dir.join("pane-capabilities");
        if let Ok(metadata) = fs::symlink_metadata(&directory)
            && (!metadata.is_dir() || metadata.file_type().is_symlink())
        {
            return Err("pane capability directory is not a directory".to_owned());
        }
        private::create_dir_all(&directory).map_err(|error| error.to_string())?;
        private::restrict_to_owner(&directory).map_err(|error| error.to_string())?;
        // A daemon restart invalidates every earlier token. Remove the old
        // references before the new daemon accepts a caller.
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            if entry.file_name().to_string_lossy().ends_with(".json") {
                fs::remove_file(entry.path()).map_err(|error| error.to_string())?;
            }
        }
        Ok(Self {
            directory,
            entries: Mutex::new(HashMap::new()),
            closed: AtomicBool::new(false),
        })
    }

    pub fn reference_path(&self, nonce: &str) -> Result<PathBuf, &'static str> {
        if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("invalid_nonce");
        }
        Ok(self.directory.join(format!("{nonce}.json")))
    }

    pub fn issue(
        &self,
        attestation: &Attestation,
        nonce: &str,
        port: u16,
        holder: Option<(i32, u64)>,
    ) -> Result<(PathBuf, bool), &'static str> {
        self.issue_with_token(attestation, nonce, port, holder)
            .map(|(path, new, _)| (path, new))
    }

    fn issue_with_token(
        &self,
        attestation: &Attestation,
        nonce: &str,
        port: u16,
        holder: Option<(i32, u64)>,
    ) -> Result<(PathBuf, bool, String), &'static str> {
        let path = self.reference_path(nonce)?;
        let mut entries = self.entries.lock().map_err(|_| "capability_unavailable")?;
        if self.closed.load(Ordering::SeqCst) {
            return Err("hide_unavailable");
        }
        entries.retain(|_, entry| {
            let alive = entry_alive(entry);
            if !alive {
                remove_local_reference(entry);
            }
            alive
        });
        if holder.is_none()
            && let Some((token, existing)) = entries.iter().find(|(_, entry)| {
                !entry.one_shot && entry.remote.is_none() && same_caller(entry, attestation)
            })
        {
            return Ok((existing.path.clone(), false, token.clone()));
        }
        if entries.len() >= MAX_CAPABILITIES {
            return Err("capability_limit");
        }
        let token = new_token();
        let mut file = private::create_new_file(&path).map_err(|_| "reference_unavailable")?;
        let bytes = serde_json::to_vec(&Reference {
            token: token.clone(),
            route: Route::Daemon {
                port,
                origin_port: port,
            },
        })
        .map_err(|_| "reference_unavailable")?;
        // Not flushed to disk: the reference lives no longer than the token
        // it names, which no restart keeps, and only this machine reads it.
        if file.write_all(&bytes).is_err() {
            let _ = fs::remove_file(&path);
            return Err("reference_unavailable");
        }
        entries.insert(
            token.clone(),
            Capability {
                pane_id: attestation.pane_id.clone(),
                context: attestation.context.clone(),
                binding: attestation.binding.clone(),
                one_shot: holder.is_some(),
                holder,
                created: Instant::now(),
                claimed: false,
                path: path.clone(),
                remote: None,
            },
        );
        Ok((path, true, token))
    }

    pub(crate) fn issue_remote(
        &self,
        attestation: &Attestation,
        grant: RemoteGrant,
    ) -> Result<(String, bool), &'static str> {
        let mut entries = self.entries.lock().map_err(|_| "capability_unavailable")?;
        if self.closed.load(Ordering::SeqCst) || grant.link.closed_reason().is_some() {
            return Err("hide_unavailable");
        }
        entries.retain(|_, entry| {
            let alive = entry_alive(entry);
            if !alive {
                remove_local_reference(entry);
            }
            alive
        });
        if !grant.one_shot
            && let Some((token, _)) = entries.iter().find(|(_, entry)| {
                !entry.one_shot
                    && same_caller(entry, attestation)
                    && entry.vouched_by(&grant.node, &grant.link)
            })
        {
            return Ok((token.clone(), false));
        }
        if entries.len() >= MAX_CAPABILITIES {
            return Err("capability_limit");
        }
        let token = new_token();
        entries.insert(
            token.clone(),
            Capability {
                pane_id: attestation.pane_id.clone(),
                context: attestation.context.clone(),
                binding: attestation.binding.clone(),
                one_shot: grant.one_shot,
                holder: None,
                created: Instant::now(),
                claimed: false,
                path: PathBuf::new(),
                remote: Some(RemoteCapability {
                    node: grant.node,
                    link: grant.link,
                    source_pane_id: grant.source_pane_id,
                }),
            },
        );
        Ok((token, true))
    }

    pub fn get(&self, token: &str) -> Option<Capability> {
        let mut entries = self.entries.lock().ok()?;
        if self.closed.load(Ordering::SeqCst) {
            return None;
        }
        let cap = entries.get(token)?.clone();
        if !entry_alive(&cap) {
            entries.remove(token);
            remove_local_reference(&cap);
            return None;
        }
        Some(cap)
    }

    pub fn revoke(&self, token: &str) {
        if let Ok(mut entries) = self.entries.lock()
            && let Some(entry) = entries.remove(token)
        {
            remove_local_reference(&entry);
        }
    }

    /// Revokes `token` when `node` vouched for it over `link`; a node can
    /// withdraw only what it vouched for.
    /// The credential `token` names, when `node` vouched for it over `link`:
    /// a command a device's node relays runs only on a credential that same
    /// node vouched for over that same connection (PRD core-host-node B18).
    pub fn vouched(&self, token: &str, node: &str, link: &RemoteHost) -> Option<Capability> {
        self.get(token)
            .filter(|capability| capability.vouched_by(node, link))
    }

    /// A hook's named token constrains a fresh proof only while it exists.
    /// An extant token must belong to this node and this very connection.
    pub(crate) fn hook_reference(
        &self,
        token: &str,
        node: &str,
        link: &RemoteHost,
        request_id: &str,
    ) -> Result<Option<Capability>, &'static str> {
        let Some(cap) = self.get(token) else {
            herdr_core::diagnostic!(json!({
                "component": "node_panes",
                "kind": "hook.reference_missing",
                "node": node,
                "request_id": request_id,
            }));
            return Ok(None);
        };
        if !cap.vouched_by(node, link) {
            return Err("credential_expired");
        }
        Ok(Some(cap))
    }

    pub fn revoke_vouched(&self, node: &str, link: &RemoteHost, token: &str) {
        if let Ok(mut entries) = self.entries.lock()
            && entries
                .get(token)
                .is_some_and(|entry| entry.vouched_by(node, link))
        {
            entries.remove(token);
        }
    }

    /// Whether a command claimed `token`, while the registry holds it.
    #[cfg(test)]
    pub(crate) fn claimed(&self, token: &str) -> Option<bool> {
        self.entries
            .lock()
            .ok()?
            .get(token)
            .map(|entry| entry.claimed)
    }

    pub fn claim(&self, token: &str) -> Result<(), &'static str> {
        let mut entries = self.entries.lock().map_err(|_| "capability_unavailable")?;
        let entry = entries.get_mut(token).ok_or("credential_expired")?;
        if !entry_alive(entry) {
            let stale = entries.remove(token).expect("entry exists");
            remove_local_reference(&stale);
            return Err("credential_expired");
        }
        entry.claimed = true;
        Ok(())
    }

    /// Checks `token` for one command. A device credential's pane is read
    /// as `attested` says when that holds for it ([`remote_identity`]).
    pub fn validate(
        &self,
        token: &str,
        herdr_socket: Option<&Path>,
        core: &CoreHandle,
        attested: Option<&StreamAttestation>,
    ) -> Result<Capability, &'static str> {
        let cap = self.get(token).ok_or("credential_expired")?;
        if cap.binding == Binding::Checkout {
            // Only registration and connection are rechecked: the checkout
            // must still resolve, on the same device, to the same context.
            let outcome =
                match core.workspace_query(&cap.context.device_id, &cap.pane_id, Query::Info) {
                    Ok(actual) if actual.context == cap.context => return Ok(cap),
                    Ok(_) => "checkout_not_registered",
                    Err(refusal) => refusal.reason,
                };
            self.revoke(token);
            note_checkout_capability(
                "checkout_capability.refused",
                "validate",
                None,
                Some(!cap.one_shot),
                Some(&cap.context),
                Some(outcome),
                None,
            );
            return Err(outcome);
        }
        let actual = if let Some(remote) = &cap.remote {
            let binding = pane_binding(remote_identity(
                &cap,
                remote,
                attested,
                PANE_INSPECT_TIMEOUT,
            )?);
            // A pane that changed is refused before the core is asked, as
            // the question guard does.
            if binding != cap.binding {
                self.revoke(token);
                return Err("pane_changed");
            }
            let context = core
                .workspace_query(&cap.context.device_id, &cap.pane_id, Query::Info)
                .map_err(|_| "pane_not_connected")?
                .context;
            Attestation {
                pane_id: cap.pane_id.clone(),
                context,
                binding,
            }
        } else {
            let socket = herdr_socket.ok_or("pane_unavailable")?;
            inspect_pane(&cap.pane_id, socket, core).inspect_err(|_| self.revoke(token))?
        };
        if actual.context != cap.context || actual.binding != cap.binding {
            self.revoke(token);
            return Err("pane_changed");
        }
        Ok(cap)
    }

    /// A question guard never borrows checkout authority or renews its budget.
    /// Device inspection stays on the exact link that issued the credential.
    pub fn validate_question_guard(
        &self,
        token: &str,
        herdr_socket: Option<&Path>,
        core: &CoreHandle,
        attested: Option<&StreamAttestation>,
        deadline: Instant,
    ) -> Result<Capability, &'static str> {
        let cap = self.get(token).ok_or("credential_expired")?;
        if cap.binding == Binding::Checkout {
            return Err("agent_pane_required");
        }
        let remaining = || {
            deadline
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
                .ok_or("factory_guard_expired")
        };
        let identity = if let Some(remote) = &cap.remote {
            remote_identity(&cap, remote, attested, remaining()?)?
        } else {
            hide_node::pane_proof::inspect_until(
                herdr_socket.ok_or("pane_unavailable")?,
                &cap.pane_id,
                deadline,
            )?
        };
        if pane_binding(identity) != cap.binding {
            return Err("pane_changed");
        }
        let context = core
            .workspace_query_until(&cap.context.device_id, &cap.pane_id, Query::Info, deadline)
            .map_err(|_| "pane_not_connected")?
            .context;
        remaining()?;
        if context != cap.context || self.get(token).is_none() {
            return Err("pane_changed");
        }
        Ok(cap)
    }

    /// Revokes every credential `node` vouched for over `link`, at once,
    /// when the link ends (B18).
    pub fn revoke_link(&self, node: &str, link: &RemoteHost) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|_, entry| !entry.vouched_by(node, link));
        }
    }

    pub fn revoke_all(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            self.closed.store(true, Ordering::SeqCst);
            for entry in entries.values() {
                remove_local_reference(entry);
            }
            entries.clear();
        }
    }

    /// Drops expired entries, and checkout-bound entries whose checkout no
    /// longer resolves on its device, so an unregistered project stops
    /// counting against the 64-reference cap before its lifetime ends.
    /// The core is asked with the lock released, so a bootstrap or a
    /// validation never waits behind the sweep's round trips.
    pub fn sweep(&self, core: &CoreHandle) {
        let checkouts: Vec<(String, String, String, Context)> = match self.entries.lock() {
            Ok(entries) => entries
                .iter()
                .filter(|(_, entry)| entry.binding == Binding::Checkout && entry_alive(entry))
                .map(|(token, entry)| {
                    (
                        token.clone(),
                        entry.context.device_id.clone(),
                        entry.pane_id.clone(),
                        entry.context.clone(),
                    )
                })
                .collect(),
            Err(_) => return,
        };
        let gone: HashMap<String, &'static str> = checkouts
            .into_iter()
            .filter_map(|(token, device_id, caller_id, context)| {
                match core.workspace_query(&device_id, &caller_id, Query::Info) {
                    Ok(actual) if actual.context == context => None,
                    Ok(_) => Some((token, "checkout_not_registered")),
                    Err(refusal) => Some((token, refusal.reason)),
                }
            })
            .collect();
        if let Ok(mut entries) = self.entries.lock() {
            entries.retain(|token, entry| {
                let mut alive = entry_alive(entry);
                if let (true, Some(reason)) = (alive, gone.get(token)) {
                    note_checkout_capability(
                        "checkout_capability.refused",
                        "sweep",
                        None,
                        Some(!entry.one_shot),
                        Some(&entry.context),
                        Some(reason),
                        None,
                    );
                    alive = false;
                }
                if !alive {
                    remove_local_reference(entry);
                }
                alive
            });
        }
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        self.revoke_all();
    }
}

/// A bootstrap request has no authority until the kernel peer PID is shown
/// to be a descendant of the shell Herdr owns for this pane, or, failing
/// that, to have its working directory inside a registered checkout. The
/// pane id may be absent for a caller that has none.
#[derive(Deserialize)]
pub struct BootstrapRequest {
    #[serde(default)]
    pub pane_id: String,
    pub nonce: String,
    #[serde(default)]
    pub one_shot: bool,
}

#[derive(Clone, Debug)]
pub struct Attestation {
    pane_id: String,
    context: Context,
    binding: Binding,
}

#[cfg(test)]
impl Attestation {
    /// What a bootstrap proves for a caller in `pane_id` whose shell is
    /// `shell_pid`, for a test that issues the credential itself.
    pub(crate) fn pane(pane_id: &str, context: Context, shell_pid: i32) -> Self {
        Self {
            pane_id: pane_id.to_owned(),
            context,
            binding: Binding::Pane {
                terminal_id: "terminal".to_owned(),
                shell_pid,
                shell_started: process_start(shell_pid).expect("a running shell"),
            },
        }
    }
}

/// The bootstrap's attestation. The pane path runs unchanged first; any
/// pane failure, including a Herdr socket hided never had, hands the caller
/// to the checkout fallback, whose reason is the bootstrap's answer when it
/// fails too. Returns the pane reason alongside a checkout-bound attestation.
// Only the Unix bootstrap socket serves it (`serve`); its tests run everywhere.
#[cfg_attr(not(unix), allow(dead_code))]
fn attest_bootstrap(
    peer: i32,
    request: &BootstrapRequest,
    herdr_socket: Option<&Path>,
    core: &CoreHandle,
) -> Result<(Attestation, Option<&'static str>), &'static str> {
    // The encoded checkout caller is hided's own key for the core; a request
    // never names one, whatever Herdr's pane namespace holds in the future.
    if matches!(Caller::parse(&request.pane_id), Caller::Checkout { .. }) {
        return Err("invalid_request");
    }
    let pane = if request.pane_id.is_empty() {
        Err("pane_not_connected")
    } else {
        match herdr_socket {
            Some(socket) => attest_local(peer, &request.pane_id, socket, core),
            None => Err("pane_unavailable"),
        }
    };
    match pane {
        Ok(attestation) => Ok((attestation, None)),
        Err(pane_reason) => attest_checkout(peer, &request.nonce, core)
            .map(|attestation| (attestation, Some(pane_reason)))
            .inspect_err(|reason| {
                note_checkout_capability(
                    "checkout_capability.refused",
                    "bootstrap",
                    Some(peer),
                    Some(!request.one_shot),
                    None,
                    Some(reason),
                    Some(pane_reason),
                );
            }),
    }
}

/// Binds a local caller that is not a pane descendant to the registered,
/// connected checkout holding its working directory. The directory comes
/// from the kernel for the peer pid; the request supplies nothing but the
/// nonce, which becomes the capability's key in the core's caller id.
pub fn attest_checkout(
    peer: i32,
    nonce: &str,
    core: &CoreHandle,
) -> Result<Attestation, &'static str> {
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid_nonce");
    }
    let path = caller_directory(peer)?;
    let caller_id = checkout_caller_id(nonce, &path);
    let context = core
        .workspace_query(core.node().as_str(), &caller_id, Query::Info)
        .map_err(|refusal| refusal.reason)?
        .context;
    Ok(Attestation {
        pane_id: caller_id,
        context,
        binding: Binding::Checkout,
    })
}

/// One record per checkout-bound issuance or refusal, in the core's Logs
/// file. It names the device, checkout and peer pid, never the caller's
/// path, pane id, or reference bytes.
fn note_checkout_capability(
    kind: &str,
    stage: &str,
    peer: Option<i32>,
    persistent: Option<bool>,
    context: Option<&Context>,
    reason: Option<&str>,
    pane_reason: Option<&str>,
) {
    herdr_core::diagnostic!(json!({
        "component": "pane_auth",
        "kind": kind,
        "stage": stage,
        "peer_pid": peer,
        "persistent": persistent,
        "device_id": context.map(|context| context.device_id.as_str()),
        "checkout_id": context.map(|context| context.checkout_id.as_str()),
        "reason": reason,
        "pane_reason": pane_reason,
    }));
}

pub fn attest_remote(
    core: &CoreHandle,
    device_id: &str,
    source_pane_id: &str,
    identity: &PaneIdentity,
) -> Result<Attestation, &'static str> {
    if device_id.is_empty() || source_pane_id.is_empty() || source_pane_id.len() > 256 {
        return Err("invalid_request");
    }
    let pane_id = format!("remote:{device_id}:pane:{source_pane_id}");
    let context = core
        .workspace_query(device_id, &pane_id, Query::Info)
        .map_err(|_| "pane_not_connected")?
        .context;
    Ok(Attestation {
        pane_id,
        context,
        binding: Binding::Pane {
            terminal_id: identity.terminal_id.clone(),
            shell_pid: identity.shell_pid,
            shell_started: identity.shell_started,
        },
    })
}

/// Binds a caller on the screen machine that is in no pane to the
/// registered, connected checkout of `node` holding `path`, its working
/// directory as that node's kernel read it (PRD core-host-node-remote-core
/// D-10). Only `node`'s own checkouts are looked at, so a node can never
/// vouch for a caller in another machine's checkout.
pub fn attest_remote_checkout(
    core: &CoreHandle,
    node: &str,
    nonce: &str,
    path: &str,
) -> Result<Attestation, &'static str> {
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid_nonce");
    }
    if node.is_empty() || !hide_platform::path::is_wire_absolute(path) || path.len() > 4096 {
        return Err("invalid_request");
    }
    let caller_id = checkout_caller_id(nonce, path);
    let context = core
        .workspace_query(node, &caller_id, Query::Info)
        .map_err(|refusal| refusal.reason)?
        .context;
    if context.device_id != node {
        return Err("checkout_not_registered");
    }
    Ok(Attestation {
        pane_id: caller_id,
        context,
        binding: Binding::Checkout,
    })
}

pub fn attest_local(
    peer: i32,
    pane_id: &str,
    herdr_socket: &Path,
    core: &CoreHandle,
) -> Result<Attestation, &'static str> {
    let attestation = inspect_pane(pane_id, herdr_socket, core)?;
    let Binding::Pane { shell_pid, .. } = &attestation.binding else {
        return Err("pane_unavailable");
    };
    if !descends_from(peer, *shell_pid) {
        return Err("caller_not_in_pane");
    }
    Ok(attestation)
}

fn inspect_pane(
    pane_id: &str,
    herdr_socket: &Path,
    core: &CoreHandle,
) -> Result<Attestation, &'static str> {
    let identity = hide_node::pane_proof::inspect(herdr_socket, pane_id)?;
    let context = core
        .workspace_query(core.node().as_str(), pane_id, Query::Info)
        .map_err(|_| "pane_not_connected")?;
    Ok(Attestation {
        pane_id: pane_id.to_owned(),
        context: context.context,
        binding: Binding::Pane {
            terminal_id: identity.terminal_id,
            shell_pid: identity.shell_pid,
            shell_started: identity.shell_started,
        },
    })
}

pub use hide_node::pane_proof::{
    BootstrapListener, bootstrap_socket_path, bootstrap_socket_record,
};

/// Binds the bootstrap socket on this machine and records it in `state_dir`.
pub fn bind(state_dir: &Path) -> Result<(BootstrapListener, PathBuf), String> {
    hide_node::pane_proof::bind(state_dir, new_token)
}

/// Answers bootstrap requests until `shutdown`. The listener accepts on a
/// thread of its own, because an accept blocks; the closer frees it when the
/// daemon stops, and a failed accept ends the bootstrap with a diagnostic.
pub async fn serve(
    listener: BootstrapListener,
    registry: Arc<Registry>,
    core: Arc<CoreHandle>,
    herdr_socket: Option<PathBuf>,
    port: u16,
    shutdown: crate::ending::Ending,
    desktop_renderers: Arc<AtomicUsize>,
) {
    use tokio::sync::Semaphore;
    // Closed however this future ends, a drop included, so the accept
    // thread never outlives it.
    let closing = CloseOnDrop {
        closer: listener.closer(),
        closed: Arc::new(AtomicBool::new(false)),
    };
    let closed = Arc::clone(&closing.closed);
    let (accepted, mut arrivals) = tokio::sync::mpsc::channel(MAX_BOOTSTRAPS);
    let accepting = std::thread::Builder::new()
        .name("pane-bootstrap-accept".to_owned())
        .spawn(move || {
            loop {
                match listener.accept() {
                    Ok(stream) => {
                        if accepted.blocking_send(Ok(stream)).is_err() {
                            return;
                        }
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::ConnectionAborted
                            && closed.load(Ordering::SeqCst) =>
                    {
                        return;
                    }
                    Err(error) => {
                        let _ = accepted.blocking_send(Err(error));
                        return;
                    }
                }
            }
        });
    if let Err(error) = accepting {
        note_bootstrap_stopped(&error);
        return;
    }
    let limit = Arc::new(Semaphore::new(MAX_BOOTSTRAPS));
    let mut sweep = tokio::time::interval(Duration::from_secs(5));
    loop {
        let arrival = tokio::select! {
            arrival = arrivals.recv() => arrival,
            _ = sweep.tick() => {
                let registry = Arc::clone(&registry);
                let core = Arc::clone(&core);
                tokio::task::spawn_blocking(move || registry.sweep(&core));
                continue;
            }
            () = shutdown.ended() => break,
        };
        let stream = match arrival {
            Some(Ok(stream)) => stream,
            Some(Err(error)) => {
                note_bootstrap_stopped(&error);
                break;
            }
            None => break,
        };
        let Ok(permit) = Arc::clone(&limit).try_acquire_owned() else {
            continue;
        };
        let registry = Arc::clone(&registry);
        let core = Arc::clone(&core);
        let herdr_socket = herdr_socket.clone();
        let desktop_renderers = Arc::clone(&desktop_renderers);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            answer_bootstrap(
                stream,
                &registry,
                &core,
                herdr_socket.as_deref(),
                port,
                desktop_renderers.load(Ordering::SeqCst) > 0,
            );
        });
    }
}

struct CloseOnDrop {
    closer: hide_platform::ipc::ListenerCloser,
    /// Set before the close, so the accept thread tells its own stop from an
    /// accept that failed.
    closed: Arc<AtomicBool>,
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
        self.closer.close();
    }
}

/// The bootstrap listener stopped accepting for a reason other than the
/// daemon's own stop; panes cannot get a capability until hided restarts.
fn note_bootstrap_stopped(error: &std::io::Error) {
    herdr_core::diagnostic!(json!({
        "component": "pane_auth",
        "kind": "bootstrap.stopped",
        "message": error.to_string(),
    }));
}

/// Reads one bootstrap request from a caller, attests the caller by the pid
/// the system reports for it, and writes the answer.
fn answer_bootstrap(
    mut stream: hide_platform::ipc::LocalStream,
    registry: &Registry,
    core: &CoreHandle,
    herdr_socket: Option<&Path>,
    port: u16,
    desktop: bool,
) {
    use std::io::{BufRead, BufReader};
    let Some(peer) = stream
        .peer_pid()
        .ok()
        .and_then(|pid| i32::try_from(pid).ok())
    else {
        return;
    };
    if stream.set_read_timeout(Some(BOOTSTRAP_TIMEOUT)).is_err()
        || stream.set_write_timeout(Some(BOOTSTRAP_TIMEOUT)).is_err()
    {
        return;
    }
    let mut line = String::new();
    let read = BufReader::new(&mut stream)
        .take(hide_node_link::panes::HOOK_REQUEST_LIMIT as u64 + 1)
        .read_line(&mut line)
        .map_err(|_| "invalid_request");
    if read.is_ok()
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
        && value.get("hook").is_some()
    {
        let result = (|| {
            if line.len() > hide_node_link::panes::HOOK_REQUEST_LIMIT {
                return Err("invalid_request");
            }
            let request: BootstrapRequest =
                serde_json::from_value(value.clone()).map_err(|_| "invalid_request")?;
            let hook: hide_node_link::panes::HookRequest =
                serde_json::from_value(value["hook"].clone()).map_err(|_| "invalid_request")?;
            if request.one_shot != hook.command.one_shot() || !crate::hook_request::valid(&hook) {
                return Err("invalid_request");
            }
            let (attestation, _) = attest_bootstrap(peer, &request, herdr_socket, core)?;
            let holder = if request.one_shot {
                Some((peer, process_start(peer).ok_or("caller_unavailable")?))
            } else {
                None
            };
            let (path, issued_new, token) =
                registry.issue_with_token(&attestation, &request.nonce, port, holder)?;
            let token = crate::hook_request::Token(registry, token, issued_new);
            let cap = registry.validate(&token.1, herdr_socket, core, None)?;
            if let Some(named) = hook.token.as_deref()
                && registry.get(named).is_some()
            {
                let prior = registry.validate(named, herdr_socket, core, None)?;
                if !cap.same_identity(&prior) {
                    return Err("pane_changed");
                }
            }
            let mut answer = crate::hook_request::answer(core, &cap, &hook, desktop);
            if !request.one_shot && answer["ok"] == true {
                answer["reference"] = json!(
                    hide_platform::path::to_wire(&path).map_err(|_| "reference_unavailable")?
                );
                if answer.to_string().len() + 1 > hide_node_link::panes::HOOK_RESPONSE_LIMIT {
                    return Err("response_too_large");
                }
                Ok((answer, Some(token)))
            } else {
                Ok((answer, None))
            }
        })();
        let (answer, persistent) = match result {
            Ok(answer) => answer,
            Err(reason) => (
                json!({"ok":false,"reason":reason,"next_action":refusal_next_action(reason)}),
                None,
            ),
        };
        if writeln!(stream, "{answer}").is_ok()
            && let Some(token) = persistent
            && registry.claim(&token.1).is_ok()
        {
            token.keep();
        }
        return;
    }
    let result = read
        .and({
            if line.len() <= 4096 {
                Ok(())
            } else {
                Err("invalid_request")
            }
        })
        .and_then(|_| {
            serde_json::from_str::<BootstrapRequest>(&line).map_err(|_| "invalid_request")
        })
        .and_then(|request| {
            let (attestation, pane_reason) = attest_bootstrap(peer, &request, herdr_socket, core)?;
            let holder = if request.one_shot {
                Some((peer, process_start(peer).ok_or("caller_unavailable")?))
            } else {
                None
            };
            let issued = registry.issue(&attestation, &request.nonce, port, holder);
            if let Some(pane_reason) = pane_reason {
                note_checkout_capability(
                    match issued {
                        Ok(_) => "checkout_capability.issued",
                        Err(_) => "checkout_capability.refused",
                    },
                    "bootstrap",
                    Some(peer),
                    Some(!request.one_shot),
                    Some(&attestation.context),
                    issued.as_ref().err().copied(),
                    Some(pane_reason),
                );
            }
            issued
        });
    // The reference crosses in the wire spelling, as the device bridge's
    // does; one without it is refused and withdrawn like an undelivered one.
    let wire = match &result {
        Ok((path, _)) => hide_platform::path::to_wire(path).ok(),
        Err(_) => None,
    };
    let answer = match (&result, &wire) {
        (Ok(_), Some(reference)) => json!({"ok": true, "reference": reference}),
        (Ok(_), None) => json!({"ok": false, "reason": "reference_unavailable"}),
        (Err(reason), _) => json!({"ok": false, "reason": reason}),
    };
    if (writeln!(stream, "{answer}").is_err() || wire.is_none())
        && let Ok((path, true)) = result
        && let Ok(bytes) = fs::read(path)
        && let Ok(reference) = serde_json::from_slice::<Reference>(&bytes)
    {
        registry.revoke(&reference.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn device_attestation(node: &str) -> Attestation {
        Attestation {
            pane_id: format!("remote:{node}:pane:w1:p1"),
            context: Context {
                device_id: node.to_owned(),
                workspace_id: "workspace".to_owned(),
                checkout_id: "checkout".to_owned(),
                checkout_path: "/checkout".to_owned(),
            },
            binding: Binding::Pane {
                terminal_id: "t1".to_owned(),
                shell_pid: 42,
                shell_started: 9,
            },
        }
    }

    fn device_grant(node: &str, link: &RemoteHost) -> RemoteGrant {
        RemoteGrant {
            node: node.to_owned(),
            link: link.clone(),
            source_pane_id: "w1:p1".to_owned(),
            one_shot: false,
        }
    }

    /// B18 and letter-720: a device credential answers only on the link it
    /// was vouched for over: not on another node's link, not on its own
    /// node's next connection, and not once its link closed or was revoked.
    #[test]
    fn a_device_credential_answers_only_on_the_link_that_vouched_for_it() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let link_a = RemoteHost::detached("ssh:a");
        let link_b = RemoteHost::detached("ssh:b");
        let next_a = RemoteHost::detached("ssh:a");
        let (token, issued) = registry
            .issue_remote(
                &device_attestation("node-a"),
                device_grant("node-a", &link_a),
            )
            .unwrap();
        assert!(issued);
        assert!(registry.vouched(&token, "node-a", &link_a).is_some());
        assert!(registry.vouched(&token, "node-b", &link_b).is_none());
        assert!(registry.vouched(&token, "node-b", &link_a).is_none());
        assert!(registry.vouched(&token, "node-a", &next_a).is_none());
        let capability = registry.get(&token).unwrap();
        assert!(capability.question_guard_source_matches(Some(("node-a", &link_a))));
        assert!(!capability.question_guard_source_matches(None));
        assert!(!capability.question_guard_source_matches(Some(("node-b", &link_a))));
        assert!(!capability.question_guard_source_matches(Some(("node-a", &next_a))));
        registry.revoke_link("node-a", &link_a);
        assert!(registry.vouched(&token, "node-a", &link_a).is_none());
        assert!(registry.get(&token).is_none());

        // A link that ends takes its credentials with it, and issues none.
        let (token, _) = registry
            .issue_remote(
                &device_attestation("node-a"),
                device_grant("node-a", &next_a),
            )
            .unwrap();
        next_a.close("lost");
        assert!(registry.get(&token).is_none());
        assert_eq!(
            registry
                .issue_remote(
                    &device_attestation("node-a"),
                    device_grant("node-a", &next_a)
                )
                .err(),
            Some("hide_unavailable")
        );
    }

    #[test]
    fn a_hook_token_ends_on_return_and_unwind_and_cannot_be_reused() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let link = RemoteHost::detached("node-a");
        for unwind in [false, true] {
            let (token, _) = registry
                .issue_remote(
                    &device_attestation("node-a"),
                    RemoteGrant {
                        one_shot: true,
                        ..device_grant("node-a", &link)
                    },
                )
                .unwrap();
            let held = token.clone();
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let _request = crate::hook_request::Token(&registry, held, true);
                assert!(registry.vouched(&token, "node-a", &link).is_some());
                if unwind {
                    panic!("request unwound");
                }
            }));
            assert_eq!(result.is_err(), unwind);
            assert!(registry.vouched(&token, "node-a", &link).is_none());
            assert_eq!(registry.claim(&token), Err("credential_expired"));
        }
    }

    #[test]
    fn a_named_hook_reference_constrains_the_proved_pane_and_identity() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let link = RemoteHost::detached("node-a");
        let (token, _) = registry
            .issue_remote(&device_attestation("node-a"), device_grant("node-a", &link))
            .unwrap();
        let cap = registry.get(&token).unwrap();
        let mut other = cap.clone();
        other.pane_id = "remote:node-a:pane:w1:p2".to_owned();
        assert!(!cap.same_identity(&other));
        other = cap.clone();
        other.binding = Binding::Pane {
            terminal_id: "replaced".to_owned(),
            shell_pid: 42,
            shell_started: 10,
        };
        assert!(!cap.same_identity(&other));
        other = cap.clone();
        other.context.device_id = "node-b".to_owned();
        assert!(!cap.same_identity(&other));
        assert!(cap.same_identity(&cap));
    }

    #[test]
    fn a_named_hook_token_from_another_node_or_link_is_refused_but_a_gone_one_is_not_authority() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let link = RemoteHost::detached("node-a");
        let other = RemoteHost::detached("node-b");
        let (token, _) = registry
            .issue_remote(&device_attestation("node-a"), device_grant("node-a", &link))
            .unwrap();
        for (node, connection) in [("node-b", &other), ("node-a", &other), ("node-b", &link)] {
            assert_eq!(
                registry
                    .hook_reference(&token, node, connection, "hook-1")
                    .err(),
                Some("credential_expired")
            );
        }
        assert!(
            registry
                .hook_reference(&token, "node-a", &link, "hook-1")
                .unwrap()
                .is_some()
        );
        registry.revoke(&token);
        assert!(
            registry
                .hook_reference(&token, "node-a", &link, "hook-2")
                .unwrap()
                .is_none()
        );
        let (fresh, _) = registry
            .issue_remote(&device_attestation("node-a"), device_grant("node-a", &link))
            .unwrap();
        assert!(registry.vouched(&fresh, "node-a", &link).is_some());
        assert!(registry.get(&token).is_none());
    }

    #[test]
    fn late_guidance_is_not_claimed_and_releases_only_the_new_token() {
        use hide_node_link::panes::ProofAnswer;
        use hide_node_link::protocol::{Outcome, Request, Response};
        use std::io::BufRead;
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let (core_end, node_end) = hide_platform::ipc::LocalStream::pair().unwrap();
        // The real node has already forgotten the waiting caller. Its false
        // receipt travels back over the real link to the core's claim path.
        let node = std::thread::spawn(move || {
            let panes = hide_host::panes::Panes::new();
            let mut writer = node_end.duplicate();
            for line in std::io::BufReader::new(node_end).lines() {
                let Ok(line) = line else {
                    break;
                };
                let request: Request = serde_json::from_str(&line).unwrap();
                let Call::PaneProofAnswer {
                    request: id,
                    answer,
                } = request.call
                else {
                    panic!("unexpected call");
                };
                if id == 100 {
                    break; // The link ends before the receipt can be sent.
                }
                let accepted = panes.answer_proof(id, answer).unwrap();
                assert_eq!(accepted, json!(false));
                let response = Response {
                    id: request.id,
                    outcome: Outcome::Ok(accepted),
                };
                if writeln!(writer, "{}", serde_json::to_string(&response).unwrap()).is_err() {
                    break;
                }
            }
        });
        let link =
            hide_node::ssh::host::over_local_stream("node-a", core_end, None, Box::new(|_| {}))
                .unwrap();
        for issued_new in [true, false] {
            let (token, _) = registry
                .issue_remote(&device_attestation("node-a"), device_grant("node-a", &link))
                .unwrap();
            assert_eq!(registry.claimed(&token), Some(false));
            let delivered = crate::node_panes::send_proof(
                Some(&registry),
                "node-a",
                &link,
                99,
                ProofAnswer::WorkspaceInfo {
                    token: token.clone(),
                    issued_new,
                    answer: json!({"ok":true}),
                },
            );
            assert!(!delivered);
            assert_eq!(
                registry.claimed(&token),
                if issued_new { None } else { Some(false) }
            );
            if issued_new {
                assert_eq!(registry.claim(&token), Err("credential_expired"));
            }
            registry.revoke(&token);
        }
        let (lost, _) = registry
            .issue_remote(&device_attestation("node-a"), device_grant("node-a", &link))
            .unwrap();
        assert!(!crate::node_panes::send_proof(
            Some(&registry),
            "node-a",
            &link,
            100,
            ProofAnswer::WorkspaceInfo {
                token: lost.clone(),
                issued_new: true,
                answer: json!({"ok":true})
            }
        ));
        assert_eq!(registry.claimed(&lost), None);
        assert_eq!(registry.claim(&lost), Err("credential_expired"));
        link.close("fixture finished");
        drop(link);
        node.join().unwrap();
    }

    /// A device's node on a local link that answers `Call::PaneInspect`
    /// with `identity`, counting how often it was asked.
    fn inspecting_node(identity: PaneIdentity) -> (RemoteHost, Arc<AtomicUsize>) {
        use hide_node_link::protocol::{Outcome, Request, Response};
        use std::io::BufRead;
        let (core_end, node_end) = hide_platform::ipc::LocalStream::pair().unwrap();
        let asked = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&asked);
        std::thread::spawn(move || {
            let mut writer = node_end.duplicate();
            let mut reader = std::io::BufReader::new(node_end);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|read| read > 0) {
                let request: Request = serde_json::from_str(&line).unwrap();
                line.clear();
                let answer = match request.call {
                    Call::PaneInspect { .. } => {
                        counted.fetch_add(1, Ordering::SeqCst);
                        serde_json::to_value(&identity).unwrap()
                    }
                    _ => serde_json::Value::Null,
                };
                let response = Response {
                    id: request.id,
                    outcome: Outcome::Ok(answer),
                };
                if writeln!(writer, "{}", serde_json::to_string(&response).unwrap()).is_err() {
                    return;
                }
            }
        });
        let link =
            hide_node::ssh::host::over_local_stream("node-a", core_end, None, Box::new(|_| {}))
                .unwrap();
        (link, asked)
    }

    /// A device credential's pane is checked against what its node proved
    /// when the command's stream opened, with no call back down the link: a
    /// matching proof asks the node nothing, a proof of another shell is
    /// refused `pane_changed` and ends the credential, and a proof the
    /// credential cannot take (none, or another link's, node's or pane's)
    /// leaves the check to `PaneInspect`, as before.
    #[test]
    fn a_node_attested_stream_is_checked_without_asking_the_node_again() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let core = CoreHandle::bare(directory.path());
        // The identity `device_attestation` binds.
        let proved = PaneIdentity {
            terminal_id: "t1".to_owned(),
            shell_pid: 42,
            shell_started: 9,
        };
        let (link, asked) = inspecting_node(proved.clone());
        let asked = || asked.load(Ordering::SeqCst);
        let issue = || {
            let grant = RemoteGrant {
                one_shot: true,
                ..device_grant("node-a", &link)
            };
            registry
                .issue_remote(&device_attestation("node-a"), grant)
                .unwrap()
                .0
        };
        let stream = |node: &str, link: &RemoteHost, pane_id: &str, identity: &PaneIdentity| {
            StreamAttestation {
                node: node.to_owned(),
                link: link.clone(),
                caller: StreamCaller {
                    pane_id: pane_id.to_owned(),
                    identity: identity.clone(),
                },
                received: Instant::now(),
            }
        };
        let guard_deadline = || Instant::now() + Duration::from_secs(10);
        // This core knows no device, so a command that passed the pane check
        // stops at the context the core holds for it.
        let past_the_pane = Some("pane_not_connected");

        let token = issue();
        let same = stream("node-a", &link, "w1:p1", &proved);
        let persistent = registry
            .issue_remote(&device_attestation("node-a"), device_grant("node-a", &link))
            .unwrap()
            .0;
        assert_eq!(
            registry
                .validate(&persistent, None, &core, Some(&same))
                .err(),
            past_the_pane
        );
        assert_eq!(asked(), 0, "a persistent reference uses the same proof");
        assert_eq!(
            registry.validate(&token, None, &core, Some(&same)).err(),
            past_the_pane
        );
        assert_eq!(
            registry
                .validate_question_guard(&token, None, &core, Some(&same), guard_deadline())
                .err(),
            past_the_pane
        );
        assert_eq!(asked(), 0);
        let restarted = PaneIdentity {
            shell_started: 10,
            ..proved.clone()
        };
        let changed = stream("node-a", &link, "w1:p1", &restarted);
        assert_eq!(
            registry
                .validate(&persistent, None, &core, Some(&changed))
                .err(),
            Some("pane_changed")
        );
        assert!(registry.get(&persistent).is_none());
        assert_eq!(
            registry
                .validate_question_guard(&token, None, &core, Some(&changed), guard_deadline())
                .err(),
            Some("pane_changed")
        );
        assert_eq!(
            registry.validate(&token, None, &core, Some(&changed)).err(),
            Some("pane_changed")
        );
        assert!(registry.get(&token).is_none());
        assert_eq!(asked(), 0);

        let token = issue();
        let next = RemoteHost::detached("node-a");
        let elsewhere = [
            ("another link", stream("node-a", &next, "w1:p1", &proved)),
            ("another node", stream("node-b", &link, "w1:p1", &proved)),
            ("another pane", stream("node-a", &link, "w1:p2", &proved)),
        ];
        for (why, attested) in &elsewhere {
            let before = asked();
            assert_eq!(
                registry.validate(&token, None, &core, Some(attested)).err(),
                past_the_pane,
                "{why}"
            );
            assert_eq!(asked(), before + 1, "{why}");
        }
        assert_eq!(
            registry.validate(&token, None, &core, None).err(),
            past_the_pane
        );
        assert_eq!(asked(), 4);
        let delayed = StreamAttestation {
            received: Instant::now() - PANE_INSPECT_TIMEOUT,
            ..same
        };
        assert_eq!(
            registry.validate(&token, None, &core, Some(&delayed)).err(),
            past_the_pane
        );
        assert_eq!(asked(), 5, "a delayed stream must re-read its pane");
        core.shutdown();
        next.close("fixture finished");
        link.close("fixture finished");
    }

    #[tokio::test]
    async fn long_state_directory_keeps_a_short_private_bootstrap_socket() {
        let directory = tempfile::tempdir().unwrap();
        // Long enough to pass a Unix socket's limit of about a hundred bytes.
        // Windows has no such limit, and hide-platform's private files there
        // take paths below MAX_PATH (260), so the folder stays under it.
        let depth = if cfg!(unix) { 12 } else { 4 };
        let state_dir = (0..depth).fold(directory.path().to_path_buf(), |path, _| {
            path.join("long-state-segment")
        });
        fs::create_dir_all(state_dir.join("pane-capabilities")).unwrap();
        let (listener, socket) = bind(&state_dir).unwrap();
        assert!(!socket.starts_with(&state_dir));
        if cfg!(unix) {
            assert!(socket.as_os_str().len() < 100);
        }
        assert_eq!(socket, bootstrap_socket_path(&state_dir).unwrap());
        assert!(private::is_private(socket.parent().unwrap()).unwrap());
        // The listener takes its endpoint and its private folder with it.
        drop(listener);
        assert!(!socket.exists());
        assert!(!socket.parent().unwrap().exists());
        let (listener, next) = bind(&state_dir).unwrap();
        assert_ne!(socket, next);
        assert_eq!(next, bootstrap_socket_path(&state_dir).unwrap());
        drop(listener);
        assert!(!next.parent().unwrap().exists());
    }

    #[test]
    fn registry_releases_references_and_refuses_issuance_after_shutdown() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let pid = std::process::id() as i32;
        let attestation = Attestation {
            pane_id: "pane".to_owned(),
            context: Context {
                device_id: "local".to_owned(),
                workspace_id: "workspace".to_owned(),
                checkout_id: "checkout".to_owned(),
                checkout_path: "/checkout".to_owned(),
            },
            binding: Binding::Pane {
                terminal_id: "terminal".to_owned(),
                shell_pid: pid,
                shell_started: process_start(pid).unwrap(),
            },
        };
        let nonce = "0123456789abcdef0123456789abcdef";
        let path = registry.issue(&attestation, nonce, 12345, None).unwrap().0;
        assert!(private::is_private(&path).unwrap());
        let reference: Reference = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(registry.get(&reference.token).is_some());
        registry.revoke_all();
        assert!(!path.exists());
        assert!(registry.get(&reference.token).is_none());
        assert_eq!(
            registry
                .issue(&attestation, nonce, 12345, None)
                .unwrap_err(),
            "hide_unavailable"
        );
    }

    #[test]
    fn bootstrap_retries_do_not_fill_the_registry_when_no_client_claims() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let core = CoreHandle::bare(directory.path());
        let pid = std::process::id() as i32;
        let attestation = Attestation {
            pane_id: "pane".to_owned(),
            context: Context {
                device_id: "local".to_owned(),
                workspace_id: "workspace".to_owned(),
                checkout_id: "checkout".to_owned(),
                checkout_path: "/checkout".to_owned(),
            },
            binding: Binding::Pane {
                terminal_id: "terminal".to_owned(),
                shell_pid: pid,
                shell_started: process_start(pid).unwrap(),
            },
        };
        for attempt in 0..(MAX_CAPABILITIES * 2) {
            let nonce = format!("{attempt:032x}");
            let path = registry.issue(&attestation, &nonce, 12345, None).unwrap().0;
            let reference: Reference = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            registry
                .entries
                .lock()
                .unwrap()
                .get_mut(&reference.token)
                .unwrap()
                .created = Instant::now() - UNCLAIMED_LIFETIME;
            registry.sweep(&core);
            assert!(!path.exists());
        }
        assert!(registry.entries.lock().unwrap().is_empty());
        assert_eq!(
            fs::read_dir(directory.path().join("pane-capabilities"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn repeated_claimed_bootstrap_reuses_one_pane_capability() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let pid = std::process::id() as i32;
        let attestation = Attestation {
            pane_id: "pane".to_owned(),
            context: Context {
                device_id: "local".to_owned(),
                workspace_id: "workspace".to_owned(),
                checkout_id: "checkout".to_owned(),
                checkout_path: "/checkout".to_owned(),
            },
            binding: Binding::Pane {
                terminal_id: "terminal".to_owned(),
                shell_pid: pid,
                shell_started: process_start(pid).unwrap(),
            },
        };
        let (path, created) = registry
            .issue(&attestation, &format!("{:032x}", 0), 12345, None)
            .unwrap();
        assert!(created);
        let reference: Reference = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        registry.claim(&reference.token).unwrap();
        for attempt in 1..=MAX_CAPABILITIES + 1 {
            let (again, created) = registry
                .issue(&attestation, &format!("{attempt:032x}"), 12345, None)
                .unwrap();
            assert_eq!(again, path);
            assert!(!created);
        }
        let (one_shot, created) = registry
            .issue(
                &attestation,
                &format!("{:032x}", MAX_CAPABILITIES + 2),
                12345,
                Some((pid, process_start(pid).unwrap())),
            )
            .unwrap();
        assert!(created);
        assert_ne!(one_shot, path);
        assert_eq!(registry.entries.lock().unwrap().len(), 2);
    }

    // Windows cannot read a process's working directory (`cwd_of`).
    #[cfg(unix)]
    #[test]
    fn bootstrap_without_a_herdr_socket_still_reaches_the_checkout_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let core = CoreHandle::bare(directory.path());
        let peer = std::process::id() as i32;
        let request = |pane_id: &str| BootstrapRequest {
            pane_id: pane_id.to_owned(),
            nonce: format!("{:032x}", 7),
            one_shot: true,
        };
        // A stale pane id with no Herdr socket used to answer pane_unavailable
        // before the fallback ran; the fallback's own answer is the proof it ran
        // (this test's cwd is inside no registered checkout).
        assert_eq!(
            attest_bootstrap(peer, &request("w9J:p17"), None, &core).unwrap_err(),
            "checkout_not_registered"
        );
        assert_eq!(
            attest_bootstrap(peer, &request(""), None, &core).unwrap_err(),
            "checkout_not_registered"
        );
        // A request cannot name the core's encoded checkout caller itself.
        assert_eq!(
            attest_bootstrap(peer, &request("checkout:abcd:/tmp"), None, &core).unwrap_err(),
            "invalid_request"
        );
        // An unreadable caller fails closed before any lookup.
        assert_eq!(
            attest_checkout(i32::MAX, &format!("{:032x}", 8), &core).unwrap_err(),
            "caller_unavailable"
        );
        assert_eq!(
            attest_checkout(peer, "not-a-nonce", &core).unwrap_err(),
            "invalid_nonce"
        );
        core.shutdown();
    }

    fn checkout_attestation(nonce: &str, checkout: &str) -> Attestation {
        Attestation {
            pane_id: checkout_caller_id(nonce, checkout),
            context: Context {
                device_id: "local".to_owned(),
                workspace_id: "workspace".to_owned(),
                checkout_id: format!("checkout{checkout}"),
                checkout_path: checkout.to_owned(),
            },
            binding: Binding::Checkout,
        }
    }

    #[test]
    fn sweep_drops_a_checkout_capability_whose_checkout_no_longer_resolves() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let core = CoreHandle::bare(directory.path());
        let pid = std::process::id() as i32;
        let gone = checkout_attestation(&format!("{:032x}", 3), "/srv/gone");
        let (path, created) = registry
            .issue(&gone, &format!("{:032x}", 3), 12345, None)
            .unwrap();
        assert!(created);
        let reference: Reference = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        registry.claim(&reference.token).unwrap();
        let pane = Attestation {
            pane_id: "w1:p1".to_owned(),
            context: Context {
                device_id: "local".to_owned(),
                workspace_id: "workspace".to_owned(),
                checkout_id: "checkout".to_owned(),
                checkout_path: "/srv/pane".to_owned(),
            },
            binding: Binding::Pane {
                terminal_id: "terminal".to_owned(),
                shell_pid: pid,
                shell_started: process_start(pid).unwrap(),
            },
        };
        let (pane_path, _) = registry
            .issue(&pane, &format!("{:032x}", 4), 12345, None)
            .unwrap();
        assert_eq!(registry.entries.lock().unwrap().len(), 2);
        // The bare core registers no checkout, so the checkout-bound entry is
        // what an unregistered project looks like; the pane entry is not
        // re-resolved by the sweep and stays until its shell ends.
        registry.sweep(&core);
        assert_eq!(registry.entries.lock().unwrap().len(), 1);
        assert!(!path.exists());
        assert!(pane_path.exists());
        assert_eq!(
            registry.validate(&reference.token, None, &core, None).err(),
            Some("credential_expired")
        );
        core.shutdown();
    }

    #[test]
    fn checkout_bound_bootstraps_reuse_one_persistent_capability_across_nonces() {
        let directory = tempfile::tempdir().unwrap();
        let registry = Registry::new(directory.path()).unwrap();
        let first = format!("{:032x}", 1);
        let (path, created) = registry
            .issue(
                &checkout_attestation(&first, "/checkout"),
                &first,
                12345,
                None,
            )
            .unwrap();
        assert!(created);
        let reference: Reference = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        // Alive without any pane shell behind it.
        let capability = registry.get(&reference.token).unwrap();
        assert_eq!(capability.pane_id, checkout_caller_id(&first, "/checkout"));
        assert_eq!(capability.context.checkout_path, "/checkout");
        let core = CoreHandle::bare(directory.path());
        assert!(capability.question_guard_source_matches(None));
        assert!(
            !capability
                .question_guard_source_matches(Some(("node-a", &RemoteHost::detached("ssh:a"))))
        );
        assert!(matches!(registry.validate_question_guard(
            &reference.token, None, &core, None, Instant::now() + Duration::from_secs(2),
        ), Err(reason) if reason == "agent_pane_required"));
        core.shutdown();
        registry.claim(&reference.token).unwrap();

        let second = format!("{:032x}", 2);
        let (again, created) = registry
            .issue(
                &checkout_attestation(&second, "/checkout"),
                &second,
                12345,
                None,
            )
            .unwrap();
        assert_eq!(again, path);
        assert!(!created);

        let other = format!("{:032x}", 3);
        let (elsewhere, created) = registry
            .issue(&checkout_attestation(&other, "/other"), &other, 12345, None)
            .unwrap();
        assert!(created);
        assert_ne!(elsewhere, path);

        let pid = std::process::id() as i32;
        let fourth = format!("{:032x}", 4);
        let (one_shot, created) = registry
            .issue(
                &checkout_attestation(&fourth, "/checkout"),
                &fourth,
                12345,
                Some((pid, process_start(pid).unwrap())),
            )
            .unwrap();
        assert!(created);
        assert_ne!(one_shot, path);
        assert_eq!(registry.entries.lock().unwrap().len(), 3);
    }
}
