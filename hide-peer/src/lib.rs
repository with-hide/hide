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
//!   tests run on; `hide-peer-iroh` is the real one.
//! - [`peers`]: the first exchange, admission, which of two connections
//!   stays, the streams of a connection and who may open them.
//! - [`inbox`]: the guest's durable record of a host's events.
//! - [`blocking`]: a stream as blocking `Read` and `Write`, for NodeLink.
//! - [`mux`]: many streams over a carrier that is one byte stream.
//! - [`limits`]: every cap, in one table.

pub mod blocking;
pub mod gate;
pub mod inbox;
pub mod key;
pub mod limits;
pub mod log;
pub mod loopback;
pub mod mux;
pub mod peers;
pub mod store;
pub mod transport;
pub mod wire;

pub use hide_peer_protocol as protocol;
pub use key::MachineKey;
pub use peers::{Clock, Link, PeerEvent, PeerIdentity, PeerStream, Peers};
pub use transport::{PeerConn, PeerTransport, RelayState, TransportError};
