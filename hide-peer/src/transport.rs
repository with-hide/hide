//! The seam (PRD D-63, D-79): authenticated, ordered byte streams with a
//! peer known by its public key.
//!
//! Everything above it is the same whatever carries the bytes; iroh and noq
//! are behind it in `hide-peer-iroh` and nowhere else. With the streams the
//! seam carries what a caller needs to know about the carrier and no more:
//! the kind of path, a stream's priority, addresses in Hide's own format,
//! and why a connection closed.
//!
//! A transport authenticates the peer's key before it hands a connection
//! over. A transport that holds more than one carrier (`crate::pipe::Beside`)
//! chooses between them by [`PathKind::rank`] before anything is admitted,
//! below the seam; which of two admitted connections stays, who may open
//! what, generations and limits are decided above it ([`crate::peers`]).

use std::fmt;
use std::future::Future;

use hide_peer_protocol::{Candidate, CloseCode, PathKind, PeerAddr, PeerKey};
use tokio::io::{AsyncRead, AsyncWrite};

#[derive(Debug)]
pub enum TransportError {
    /// The transport itself is closed; nothing more will be dialed or
    /// accepted.
    Closed,
    /// The peer could not be reached, or did not prove its key.
    Unreachable(String),
    /// The connection ended.
    Ended(CloseCode),
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => f.write_str("the transport is closed"),
            Self::Unreachable(why) => write!(f, "the peer could not be reached: {why}"),
            Self::Ended(code) => write!(f, "the connection ended ({})", code.name()),
        }
    }
}

impl std::error::Error for TransportError {}

/// What a transport knows of the relay this machine connects through: the
/// reason a machine out of direct reach gives for "not connected" (PRD B33,
/// B34).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayState {
    /// The transport uses no relay.
    None,
    /// This machine is attached to its relay.
    Attached,
    /// The relay answers that it stopped relaying for the rest of the month
    /// (B34). Nothing is gained by dialing through it until it resumes.
    MonthlyLimit,
    /// The relay could not be reached, or answered something else.
    Unreachable,
}

/// Where another carrier to a machine stands, beside the transport's own
/// paths: its SSH route.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Route {
    /// None: no alias, or a transport that makes no such carrier.
    None,
    /// A sign-in the first dial stopped waiting for runs, or proved: the
    /// climb takes it at once.
    Handed,
    /// Its last attempt proved, or none was made since the daemon started:
    /// a first dial tries it.
    Ready,
    /// Its wait ran out: tried in the background once a connection is up,
    /// never by a first dial.
    Due,
    /// It waits after a failure, or is parked until the operator acts.
    Held,
}

pub trait PeerTransport: Send + Sync + 'static {
    type Conn: PeerConn;

    /// This machine's key.
    fn key(&self) -> PeerKey;

    /// Where this machine can be reached now, in the order to try.
    fn candidates(&self) -> Vec<Candidate>;

    /// Connects to `to.key` and returns once the peer proved that key.
    /// Dropping the future gives the dial up.
    fn dial(
        &self,
        to: &PeerAddr,
    ) -> impl Future<Output = Result<Self::Conn, TransportError>> + Send;

    /// The next connection a peer dialed, its key proved.
    /// [`TransportError::Closed`] once the transport is closed. It never
    /// waits on one peer's proof: a peer that starts one and does not
    /// finish it holds up no other peer's connection.
    fn accept(&self) -> impl Future<Output = Result<Self::Conn, TransportError>> + Send;

    /// The relay's state now. A transport may ask the relay to answer, so
    /// this is called to explain a machine that could not be reached, never
    /// on a timer.
    fn relay(&self) -> impl Future<Output = RelayState> + Send;

    /// Dials `to` over the other carrier alone, its route's: the keeper's
    /// climb from the relay. The transport's own dial by default.
    fn dial_route(
        &self,
        to: &PeerAddr,
    ) -> impl Future<Output = Result<Self::Conn, TransportError>> + Send {
        self.dial(to)
    }

    /// Where another carrier to `to` stands: what the keeper's climb reads
    /// before it moves a live connection. None by default.
    fn route(&self, _to: &PeerAddr) -> Route {
        Route::None
    }

    /// A connection to `key` over that other carrier, or one a move
    /// installed, ended within [`crate::limits::STAYED_AFTER`].
    fn route_fell(&self, _key: &PeerKey) {}

    /// A connection to `key` over that other carrier, or one a move
    /// installed, stayed [`crate::limits::STAYED_AFTER`] and more: the
    /// route's wait starts over.
    fn route_stayed(&self, _key: &PeerKey) {}

    /// The machine woke or its network changed: what waits after a failure
    /// is tried again.
    fn network_changed(&self) {}

    /// Closes every connection and stops accepting.
    fn close(&self) -> impl Future<Output = ()> + Send;
}

pub trait PeerConn: Clone + Send + Sync + 'static {
    type Send: AsyncWrite + Send + Unpin + 'static;
    type Recv: AsyncRead + Send + Unpin + 'static;

    /// The peer's proved key.
    fn peer(&self) -> PeerKey;

    /// A number no other connection of this transport has had.
    fn id(&self) -> u64;

    /// Whether this side dialed.
    fn dialed(&self) -> bool;

    /// The path the connection's bytes take now.
    fn path(&self) -> PathKind;

    /// The path once the transport chose one; `None` while [`PeerConn::path`]
    /// reads a default it has not chosen yet, so a choice between carriers
    /// never takes that default for a direct path.
    fn chosen_path(&self) -> Option<PathKind> {
        Some(self.path())
    }

    /// Resolves with the path once it is no longer `from`. It may wait as
    /// long as the connection lives; a caller races it with
    /// [`PeerConn::closed`].
    fn path_changed(&self, from: PathKind) -> impl Future<Output = PathKind> + Send;

    /// Opens a stream both ways. A larger `priority` is sent first when
    /// streams compete.
    fn open(
        &self,
        priority: i32,
    ) -> impl Future<Output = Result<(Self::Send, Self::Recv), TransportError>> + Send;

    /// The next stream the peer opened.
    fn accept(
        &self,
    ) -> impl Future<Output = Result<(Self::Send, Self::Recv), TransportError>> + Send;

    /// Closes the connection; the peer reads `code`.
    fn close(&self, code: CloseCode);

    /// The code the connection ended with, once it has ended. A connection
    /// that answers `Some` is no longer live, whoever still holds it.
    fn ended(&self) -> Option<CloseCode>;

    /// Resolves once the connection is over on this side, with the code it
    /// ended with. A connection is dialed again only after this (D-62).
    fn closed(&self) -> impl Future<Output = CloseCode> + Send;
}
