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
//! - an endpoint that closed without being asked is bound again, and
//!   [`IrohTransport::rebuild`] does the same for one that is stuck
//!   (iroh#4656, #4476);
//! - a connection that had a relay path and lost it while this machine is
//!   still attached to the relay is closed [`CloseCode::NoRelayPath`], so
//!   `hide-peer` dials it again: iroh does not reopen a relay path, and a
//!   connection without one dies with its direct path;
//! - among the addresses this machine found for itself, Tailscale's range
//!   is listed last (D-04).

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

use hide_peer::limits;
use hide_peer::transport::{PeerConn, PeerTransport, TransportError};
use hide_peer_protocol::addr::rank_self_discovered;
use hide_peer_protocol::{ALPN, Candidate, CloseCode, PathKind, PeerAddr, PeerKey};
use iroh::endpoint::{
    Connection, ConnectionError, PortmapperConfig, QuicTransportConfig, RecvStream, SendStream,
    VarInt, presets,
};
use iroh::{
    Endpoint, EndpointAddr, EndpointId, RelayConfig, RelayMap, RelayMode, RelayUrl, SecretKey,
    TransportAddr, Watcher,
};
use tokio::sync::watch;

/// A path that carried nothing for this long is given up, and a path is
/// kept alive at this interval (S1: about three seconds and one second).
pub const PATH_IDLE: Duration = Duration::from_secs(3);
pub const PATH_KEEP_ALIVE: Duration = Duration::from_secs(1);

/// How often a connection's paths are read, and how long a lost relay path
/// may stay lost before the connection is dialed again.
const PATH_CHECK: Duration = Duration::from_secs(1);
const RELAY_PATH_GRACE: Duration = Duration::from_secs(5);

/// How this machine's endpoint is set up.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// The URL of the relay this build attaches to. `None` runs without
    /// one: only direct addresses work.
    pub relay: Option<String>,
    /// Finding other machines on the local network by mDNS.
    pub mdns: bool,
    /// The one address to bind. `None` binds every interface.
    pub bind: Option<SocketAddr>,
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
            builder.relay_mode(RelayMode::Custom(RelayMap::from(RelayConfig::new(
                url, None,
            ))))
        }
        None => builder.relay_mode(RelayMode::Disabled),
    };
    if let Some(address) = options.bind {
        builder = builder
            .clear_ip_transports()
            .bind_addr(address)
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
        Ok(Self {
            secret,
            options,
            endpoint: RwLock::new(endpoint),
            rebuilt: watch::channel(0).0,
            closed: AtomicBool::new(false),
            next_id: Arc::new(AtomicU64::new(0)),
        })
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
        loop {
            if self.closed.load(Ordering::Acquire) {
                return Err(TransportError::Closed);
            }
            let endpoint = self.endpoint();
            let Some(incoming) = endpoint.accept().await else {
                if self.closed.load(Ordering::Acquire) {
                    return Err(TransportError::Closed);
                }
                // The endpoint ended. A rebuild in progress announces the
                // new one; one that ended by itself is bound again here.
                if !rebuilt.has_changed().unwrap_or(false) {
                    self.rebuild()
                        .await
                        .map_err(|error| TransportError::Unreachable(error.to_string()))?;
                }
                rebuilt.mark_unchanged();
                continue;
            };
            // A peer that fails its handshake never reaches the caller.
            let Ok(accepting) = incoming.accept() else {
                continue;
            };
            if let Ok(conn) = accepting.await {
                return Ok(self.wrap(&endpoint, conn, false));
            }
        }
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
