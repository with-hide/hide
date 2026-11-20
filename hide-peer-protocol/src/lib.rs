//! The contract between two Hide machines (PRD device-connect-iroh D-65).
//!
//! It is small and grows by capability: a fixed first exchange ([`hello`]),
//! the stream kinds one connection carries ([`stream`]), the management
//! messages ([`manage`]), pairing ([`pairing`]), and what a host says to the
//! guest node process it runs ([`launch`], [`tap`], [`panes`], [`event`]).
//! A reader ignores a field it does not know; a sender acts only on a
//! capability the peer announced.
//!
//! `contracts/peer-link.json` states the same contract as a file.
//! [`contract::shape`] derives that file's content from these types, and a
//! test fails until the two agree.

pub mod addr;
pub mod build;
pub mod close;
pub mod contract;
pub mod event;
pub mod frame;
pub mod hello;
pub mod key;
pub mod launch;
pub mod manage;
pub mod pairing;
pub mod panes;
pub mod stream;
pub mod tap;

pub use addr::{Candidate, PathKind, PeerAddr};
pub use close::CloseCode;
pub use key::PeerKey;
pub use stream::StreamKind;

/// The one protocol name a pair of machines connects under (D-62).
pub const ALPN: &[u8] = b"hide/peer/1";
