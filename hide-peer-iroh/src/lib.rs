//! An iroh endpoint as a [`PeerTransport`] (PRD device-connect-iroh D-63,
//! D-64). This is the one crate that links iroh and noq.
//!
//! It keeps no product state. What it decides is only what iroh needs
//! decided:
//! - no default features, no port mapping (`PortmapperConfig::Disabled`),
//!   the proxy read from the environment;
//! - a path that carries nothing is given up after about three seconds, so
//!   a dead direct path does not hold a connection for iroh's default;
//! - mDNS lookup can be turned off, and relay and saved addresses still
//!   work without it;
//! - each incoming connection's handshake runs in a task of its own, under
//!   a cap and a time, so a peer that starts one and never finishes it holds
//!   up nobody else's;
//! - an endpoint that closed without being asked is bound again, and
//!   [`IrohTransport::rebuild`] does the same for one that is stuck
//!   (iroh#4656, #4476);
//! - a connection that had a relay path and lost it while this machine is
//!   still attached to the relay is closed [`CloseCode::NoRelayPath`], so
//!   `hide-peer` dials it again: iroh does not reopen a relay path, and a
//!   connection without one dies with its direct path;
//! - among the addresses this machine found for itself, Tailscale's range
//!   is listed last (D-04);
//! - a relay this machine is not attached to is asked why once, when a
//!   caller wants the reason: the device relay answers 503 and "monthly
//!   limit reached" while it has stopped for the month (B34). iroh keeps its
//!   own relay failure as text to print, never to decide by.

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use hide_peer::limits;
use hide_peer::transport::{PeerConn, PeerTransport, RelayState, TransportError};
use hide_peer_protocol::addr::rank_self_discovered;
use hide_peer_protocol::{ALPN, Candidate, CloseCode, PathKind, PeerAddr, PeerKey};
use iroh::endpoint::{
    BindOpts, Connection, ConnectionError, PortmapperConfig, QuicTransportConfig, RecvStream,
    SendStream, VarInt, presets,
};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey,
    TransportAddr, Watcher,
};
use tokio::sync::{Semaphore, mpsc, watch};

/// A path that carried nothing for this long is given up, and a path is
/// kept alive at this interval (S1: about three seconds and one second).
pub const PATH_IDLE: Duration = Duration::from_secs(3);
pub const PATH_KEEP_ALIVE: Duration = Duration::from_secs(1);

/// How often a connection's paths are read, and how long a lost relay path
/// may stay lost before the connection is dialed again.
const PATH_CHECK: Duration = Duration::from_secs(1);
const RELAY_PATH_GRACE: Duration = Duration::from_secs(5);

/// The relay a release attaches to (PRD D-44). It answers address discovery
/// on iroh's default UDP port, which [`Options::relay`] always asks for.
pub const DEVICE_RELAY_URL: &str = "https://device-relay.withhide.dev";

/// What the relay's host answers every request with while the relay has
/// stopped for the rest of the month (PRD B34).
const LIMIT_STATUS: u16 = 503;
const LIMIT_WORDS: &str = "monthly limit reached";

/// How long the relay's host is given to answer, and how much of its answer
/// is read.
const RELAY_ANSWERS_WITHIN: Duration = Duration::from_secs(10);
const RELAY_ANSWER_BYTES: usize = 1024;

/// How this machine's endpoint is set up.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The URL of the relay this build attaches to, [`DEVICE_RELAY_URL`] in
    /// a release. `None` runs without one: only direct addresses work.
    pub relay: Option<String>,
    /// Finding other machines on the local network by mDNS.
    pub mdns: bool,
    /// The one address to bind. `None` binds every interface.
    pub bind: Option<SocketAddr>,
    /// With no [`Options::bind`], the UDP port every interface binds, IPv4
    /// required and IPv6 where the system has it: the state folder's port,
    /// which the Windows firewall rule names (PRD D-38). `None` is any free
    /// port.
    pub port: Option<u16>,
}

/// Why an endpoint could not be opened.
#[derive(Debug)]
pub struct BindFailed(String);

impl std::fmt::Display for BindFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the peer endpoint could not be opened: {}", self.0)
    }
}

impl std::error::Error for BindFailed {}

impl From<BindFailed> for io::Error {
    fn from(error: BindFailed) -> Self {
        io::Error::other(error)
    }
}

pub struct IrohTransport {
    secret: SecretKey,
    options: Options,
    endpoint: RwLock<Endpoint>,
    /// Counts endpoints; an accept that outlived its endpoint waits on it.
    rebuilt: watch::Sender<u64>,
    closed: AtomicBool,
    next_id: Arc<AtomicU64>,
    /// Incoming connections whose handshake runs. At the cap the next one
    /// is refused before any handshake starts.
    handshaking: Arc<Semaphore>,
    /// Connections whose handshake finished, with the endpoint they came in
    /// on, waiting for [`PeerTransport::accept`].
    proved: mpsc::Sender<(Endpoint, Connection)>,
    proving: tokio::sync::Mutex<mpsc::Receiver<(Endpoint, Connection)>>,
}

async fn bind(secret: &SecretKey, options: &Options) -> Result<Endpoint, BindFailed> {
    let failed = |error: &dyn std::fmt::Display| BindFailed(error.to_string());
    let stream_cap = VarInt::from_u32(limits::MAX_STREAMS as u32);
    let transport = QuicTransportConfig::builder()
        .default_path_max_idle_timeout(PATH_IDLE)
        .default_path_keep_alive_interval(PATH_KEEP_ALIVE)
        .max_concurrent_bidi_streams(stream_cap)
        .build();
    let mut builder = Endpoint::builder(presets::Minimal)
        .secret_key(secret.clone())
        .alpns(vec![ALPN.to_vec()])
        .portmapper_config(PortmapperConfig::Disabled)
        .transport_config(transport)
        .proxy_from_env();
    builder = match &options.relay {
        Some(url) => {
            let url: RelayUrl = url.parse().map_err(|error| failed(&error))?;
            // With the relay's address discovery, which is how this machine
            // learns the address a peer behind another NAT can dial.
            builder.relay_mode(RelayMode::Custom(RelayMap::from(RelayConfig::from(url))))
        }
        None => builder.relay_mode(RelayMode::Disabled),
    };
    if let Some(address) = options.bind {
        builder = builder
            .clear_ip_transports()
            .bind_addr(address)
            .map_err(|error| failed(&error))?;
    } else if let Some(port) = options.port {
        builder = builder
            .clear_ip_transports()
            .bind_addr(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)))
            .map_err(|error| failed(&error))?
            .bind_addr_with_opts(
                SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)),
                BindOpts::default().set_is_required(false),
            )
            .map_err(|error| failed(&error))?;
    }
    if options.mdns {
        builder = builder.address_lookup(iroh_mdns_address_lookup::MdnsAddressLookup::builder());
    }
    builder.bind().await.map_err(|error| failed(&error))
}

impl IrohTransport {
    /// Opens this machine's endpoint under its machine key. `secret` is the
    /// raw key the state folder keeps (`hide_peer::MachineKey::secret`);
    /// iroh's own key type never leaves this crate.
    pub async fn open(secret: &[u8; 32], options: Options) -> Result<Self, BindFailed> {
        let secret = SecretKey::from_bytes(secret);
        let endpoint = bind(&secret, &options).await?;
        let (proved, proving) = mpsc::channel(limits::MAX_ADMITTING);
        Ok(Self {
            secret,
            options,
            endpoint: RwLock::new(endpoint),
            rebuilt: watch::channel(0).0,
            closed: AtomicBool::new(false),
            next_id: Arc::new(AtomicU64::new(0)),
            handshaking: Arc::new(Semaphore::new(limits::MAX_ADMITTING)),
            proved,
            proving: tokio::sync::Mutex::new(proving),
        })
    }

    /// Starts one incoming connection's handshake in a task of its own. The
    /// task ends with the handshake, at [`limits::HELLO_WITHIN`] at the
    /// latest, or when nobody accepts any more.
    fn prove(&self, endpoint: &Endpoint, incoming: iroh::endpoint::Incoming) {
        let Ok(permit) = Arc::clone(&self.handshaking).try_acquire_owned() else {
            incoming.refuse();
            return;
        };
        // A peer that fails its handshake never reaches the caller.
        let Ok(accepting) = incoming.accept() else {
            return;
        };
        let proved = self.proved.clone();
        let endpoint = endpoint.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Ok(Ok(conn)) = tokio::time::timeout(limits::HELLO_WITHIN, accepting).await {
                // The receiver is gone only with the transport.
                let _ = proved.send((endpoint, conn)).await;
            }
        });
    }

    fn endpoint(&self) -> Endpoint {
        self.endpoint
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Closes the endpoint and binds a new one under the same key. Every
    /// connection of the old one ends; `hide-peer` dials them again.
    pub async fn rebuild(&self) -> Result<(), BindFailed> {
        let old = self.endpoint();
        old.close().await;
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        let new = bind(&self.secret, &self.options).await?;
        *self
            .endpoint
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = new;
        self.rebuilt.send_modify(|count| *count += 1);
        Ok(())
    }

    fn wrap(&self, endpoint: &Endpoint, conn: Connection, dialed: bool) -> IrohConn {
        let conn = IrohConn {
            conn,
            dialed,
            id: self.next_id.fetch_add(1, Ordering::AcqRel) + 1,
            end: Arc::new(OnceLock::new()),
        };
        if self.options.relay.is_some() {
            tokio::spawn(watch_relay_path(endpoint.clone(), conn.clone()));
        }
        conn
    }
}

fn to_endpoint_addr(to: &PeerAddr) -> Result<EndpointAddr, TransportError> {
    let unreachable = |why: String| TransportError::Unreachable(why);
    let id = EndpointId::from_bytes(to.key.as_bytes())
        .map_err(|error| unreachable(format!("not a key iroh can dial: {error}")))?;
    let mut addrs = Vec::with_capacity(to.candidates.len());
    for candidate in &to.candidates {
        addrs.push(match candidate {
            Candidate::Ip(address) => TransportAddr::Ip(*address),
            Candidate::Relay(url) => TransportAddr::Relay(
                url.parse()
                    .map_err(|error| unreachable(format!("not a relay address: {error}")))?,
            ),
        });
    }
    Ok(EndpointAddr::from_parts(id, addrs))
}

impl PeerTransport for IrohTransport {
    type Conn = IrohConn;

    fn key(&self) -> PeerKey {
        PeerKey::from_bytes(*self.secret.public().as_bytes())
    }

    fn candidates(&self) -> Vec<Candidate> {
        let addr = self.endpoint().addr();
        let mut found: Vec<Candidate> = addr.ip_addrs().map(|ip| Candidate::Ip(*ip)).collect();
        found = rank_self_discovered(found);
        found.extend(
            addr.relay_urls()
                .map(|url| Candidate::Relay(url.to_string())),
        );
        found
    }

    async fn dial(&self, to: &PeerAddr) -> Result<IrohConn, TransportError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(TransportError::Closed);
        }
        let endpoint = self.endpoint();
        let conn = endpoint
            .connect(to_endpoint_addr(to)?, ALPN)
            .await
            .map_err(|error| TransportError::Unreachable(error.to_string()))?;
        Ok(self.wrap(&endpoint, conn, true))
    }

    async fn accept(&self) -> Result<IrohConn, TransportError> {
        let mut rebuilt = self.rebuilt.subscribe();
        let mut proving = self.proving.lock().await;
        loop {
            if self.closed.load(Ordering::Acquire) {
                return Err(TransportError::Closed);
            }
            let endpoint = self.endpoint();
            // Never waits on one peer's handshake: a connection that
            // finished its own is returned, and every other one that
            // arrives meanwhile starts.
            let incoming = tokio::select! {
                proved = proving.recv() => {
                    // The transport holds a sender, so this is a connection.
                    let Some((endpoint, conn)) = proved else {
                        return Err(TransportError::Closed);
                    };
                    return Ok(self.wrap(&endpoint, conn, false));
                }
                incoming = endpoint.accept() => incoming,
            };
            let Some(incoming) = incoming else {
                if self.closed.load(Ordering::Acquire) {
                    return Err(TransportError::Closed);
                }
                // The endpoint ended. A rebuild in progress announces the
                // new one; one that ended by itself is bound again here.
                // The caller waits before it asks again when that fails.
                if !rebuilt.has_changed().unwrap_or(false) {
                    self.rebuild()
                        .await
                        .map_err(|error| TransportError::Unreachable(error.to_string()))?;
                }
                rebuilt.mark_unchanged();
                continue;
            };
            self.prove(&endpoint, incoming);
        }
    }

    async fn relay(&self) -> RelayState {
        let Some(url) = &self.options.relay else {
            return RelayState::None;
        };
        let endpoint = self.endpoint();
        let attached = endpoint
            .home_relay_status()
            .get()
            .iter()
            .any(|relay| relay.is_connected());
        if attached {
            return RelayState::Attached;
        }
        relay_answer(url, &endpoint).await
    }

    async fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.endpoint().close().await;
    }
}

#[derive(Clone)]
pub struct IrohConn {
    conn: Connection,
    dialed: bool,
    id: u64,
    /// The code the connection ended with, decided once: the peer's when
    /// the peer closed first, this side's otherwise. iroh reports this
    /// side's own close only as "closed locally", and reports that for a
    /// close made after the peer's too.
    end: Arc<OnceLock<CloseCode>>,
}

fn code_of(error: &ConnectionError) -> CloseCode {
    match error {
        ConnectionError::ApplicationClosed(close) => u32::try_from(close.error_code.into_inner())
            .map_or(CloseCode::Lost, CloseCode::from_number),
        ConnectionError::LocallyClosed => CloseCode::Shutdown,
        _ => CloseCode::Lost,
    }
}

impl IrohConn {
    fn end(&self, error: &ConnectionError) -> CloseCode {
        *self.end.get_or_init(|| code_of(error))
    }

    fn error(&self, error: &ConnectionError) -> TransportError {
        TransportError::Ended(self.end(error))
    }

    fn has_relay_path(&self) -> bool {
        self.conn.paths().iter().any(|path| path.is_relay())
    }
}

impl PeerConn for IrohConn {
    type Send = SendStream;
    type Recv = RecvStream;

    fn peer(&self) -> PeerKey {
        PeerKey::from_bytes(*self.conn.remote_id().as_bytes())
    }

    fn id(&self) -> u64 {
        self.id
    }

    fn dialed(&self) -> bool {
        self.dialed
    }

    fn path(&self) -> PathKind {
        let relayed = self
            .conn
            .paths()
            .iter()
            .any(|path| path.is_selected() && path.is_relay());
        if relayed {
            PathKind::Relay
        } else {
            PathKind::Direct
        }
    }

    async fn open(&self, priority: i32) -> Result<(SendStream, RecvStream), TransportError> {
        let (send, recv) = self
            .conn
            .open_bi()
            .await
            .map_err(|error| self.error(&error))?;
        send.set_priority(priority)
            .map_err(|_| TransportError::Ended(CloseCode::Lost))?;
        Ok((send, recv))
    }

    async fn accept(&self) -> Result<(SendStream, RecvStream), TransportError> {
        self.conn
            .accept_bi()
            .await
            .map_err(|error| self.error(&error))
    }

    fn close(&self, code: CloseCode) {
        match self.conn.close_reason() {
            // The peer closed first: its code stands.
            Some(error) => {
                self.end(&error);
            }
            None => {
                let _ = self.end.set(code);
                self.conn
                    .close(VarInt::from_u32(code.number()), code.name().as_bytes());
            }
        }
    }

    fn ended(&self) -> Option<CloseCode> {
        self.conn.close_reason().map(|error| self.end(&error))
    }

    async fn closed(&self) -> CloseCode {
        let error = self.conn.closed().await;
        self.end(&error)
    }
}

/// Asks the relay's host over HTTPS why this machine is not attached, under
/// the trust anchors iroh holds the relay's certificate to and through the
/// proxy the environment names.
async fn relay_answer(url: &str, endpoint: &Endpoint) -> RelayState {
    let client = reqwest::Client::builder()
        .use_preconfigured_tls(endpoint.tls_config().clone())
        .timeout(RELAY_ANSWERS_WITHIN)
        .build();
    let Ok(client) = client else {
        return RelayState::Unreachable;
    };
    let Ok(mut answer) = client.get(url).send().await else {
        return RelayState::Unreachable;
    };
    if answer.status().as_u16() != LIMIT_STATUS {
        return RelayState::Unreachable;
    }
    let mut body = Vec::new();
    while body.len() < RELAY_ANSWER_BYTES {
        match answer.chunk().await {
            Ok(Some(chunk)) => body.extend_from_slice(&chunk),
            _ => break,
        }
    }
    body.truncate(RELAY_ANSWER_BYTES);
    if String::from_utf8_lossy(&body).contains(LIMIT_WORDS) {
        RelayState::MonthlyLimit
    } else {
        RelayState::Unreachable
    }
}

/// Whether a connection is dialed again for its relay path: it had one, has
/// none now, and this machine is attached to the relay, so a new connection
/// would get one. While the relay is down nothing is gained by dialing.
fn redial_for_relay_path(had: bool, has: bool, relay_attached: bool) -> bool {
    had && !has && relay_attached
}

async fn watch_relay_path(endpoint: Endpoint, conn: IrohConn) {
    let mut check = tokio::time::interval(PATH_CHECK);
    let mut had = false;
    let mut lost_for = Duration::ZERO;
    loop {
        tokio::select! {
            _ = conn.conn.closed() => return,
            _ = check.tick() => {}
        }
        let has = conn.has_relay_path();
        had |= has;
        let attached = endpoint
            .home_relay_status()
            .get()
            .iter()
            .any(|relay| relay.is_connected());
        if !redial_for_relay_path(had, has, attached) {
            lost_for = Duration::ZERO;
            continue;
        }
        lost_for += PATH_CHECK;
        if lost_for >= RELAY_PATH_GRACE {
            conn.close(CloseCode::NoRelayPath);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connection_is_dialed_again_only_for_a_relay_path_it_lost_and_can_regain() {
        assert!(redial_for_relay_path(true, false, true));
        // It never had one: a connection made over a saved direct address.
        assert!(!redial_for_relay_path(false, false, true));
        // It still has one.
        assert!(!redial_for_relay_path(true, true, true));
        // The relay is down: a new connection would get none either.
        assert!(!redial_for_relay_path(true, false, false));
    }

    #[test]
    fn a_peer_address_becomes_iroh_s_and_a_bad_relay_address_is_refused() {
        let key = PeerKey::from_bytes(*SecretKey::from_bytes(&[7; 32]).public().as_bytes());
        let to = PeerAddr::new(
            key,
            vec![
                Candidate::Ip("192.168.0.9:4433".parse().unwrap()),
                Candidate::Relay("https://relay.example./".into()),
            ],
        );
        let addr = to_endpoint_addr(&to).unwrap();
        assert_eq!(addr.ip_addrs().count(), 1);
        assert_eq!(addr.relay_urls().count(), 1);
        let bad = PeerAddr::new(key, vec![Candidate::Relay("not a url".into())]);
        assert!(matches!(
            to_endpoint_addr(&bad),
            Err(TransportError::Unreachable(_))
        ));
    }
}
