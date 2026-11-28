//! The connection between two Hide machines, above whatever carries it.
//!
//! Each machine runs its own core; pairing lets one (the guest) run work on
//! the other (the host) and look at its screen. One connection per pair of
//! machines carries both directions, and each direction has its own grant.
//!
//! - [`key`]: the machine key, one per state folder, and the proof a
//!   transport without its own authentication uses.
//! - [`store`]: `peers/grants.json` (what this machine grants, saved before
//!   anything acts on it) and `peers/known.json` (the hosts it is a guest of).
//! - [`gate`]: each direction's generation, read without a lock.
//! - [`transport`]: the seam. [`loopback`] is the in-process transport the
//!   tests run on; `hide-peer-iroh` is the real one, and [`pipe`] the one
//!   beside it for a WSL distribution and its PC, whose carrier is
//!   `wsl.exe`'s one byte stream. [`proof`] is the first exchange of the
//!   two that do not authenticate by themselves.
//! - [`peers`]: the first exchange, admission, which of two connections
//!   stays, the streams of a connection and who may open them.
//! - [`pairing`]: how two machines become a pair and stop being one: the
//!   one-time [`link`], the [`words`] both operators compare, the requests
//!   that wait for an operator, and the confirm and removal that are sent
//!   until the host answers, each counted by [`rate`].
//! - [`inbox`]: the guest's durable record of a host's events.
//! - [`blocking`]: a stream as blocking `Read` and `Write`, for NodeLink.
//! - [`mux`]: many streams over a carrier that is one byte stream.
//! - [`limits`]: every cap, in one table.
//! - [`port`]: the UDP port a state folder's endpoint binds, chosen once.

pub mod blocking;
pub mod gate;
pub mod inbox;
pub mod key;
pub mod limits;
pub mod link;
pub mod log;
pub mod loopback;
pub mod mux;
pub mod pairing;
pub mod peers;
pub mod pipe;
pub mod port;
pub mod proof;
pub mod rate;
pub mod store;
pub mod transport;
pub mod wire;
pub mod words;

pub use hide_peer_protocol as protocol;
pub use key::MachineKey;
pub use peers::{Clock, Link, PeerEvent, PeerIdentity, PeerStream, Peers};
pub use transport::{DialBound, PeerConn, PeerTransport, RelayState, Route, TransportError};
