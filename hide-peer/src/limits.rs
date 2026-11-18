//! Every cap of a peer connection, in one table (PRD D-62).
//!
//! The order streams are sent in is beside it: [`StreamKind::priority`],
//! which `contracts/peer-link.json` states. The assertions at the bottom
//! keep the numbers from contradicting each other, at build time.

use std::time::Duration;

use hide_peer_protocol::StreamKind;

/// The most machines a host grants at once; the next is refused "too many
/// connected machines" (B6). Pending, active and paused grants all count.
pub const MAX_GRANTS: usize = 32;

/// The most connections held at once that have no grant in either
/// direction: machines in the middle of pairing.
pub const MAX_UNGRANTED: usize = 16;

/// The most connections whose first exchange runs at once; the next is
/// closed before any of its bytes are read.
pub const MAX_ADMITTING: usize = 16;

/// How long a connection with no grant in either direction may stay: the
/// life of a pending grant.
pub const UNGRANTED_FOR: Duration = PENDING_FOR;

/// How long a grant waits for its guest's confirm before it is dropped
/// (D-30).
pub const PENDING_FOR: Duration = Duration::from_secs(10 * 60);

/// How long a removed grant is remembered, so its guest is told "removed"
/// instead of "unknown", and how many are.
pub const TOMBSTONE_FOR: Duration = Duration::from_secs(30 * 24 * 60 * 60);
pub const MAX_TOMBSTONES: usize = 256;

/// The most hosts a guest remembers.
pub const MAX_KNOWN: usize = 64;

/// The most streams one connection carries at once, of every kind: what a
/// transport is told to allow. It is at least the sum of the kinds' caps, so
/// a kind at its cap never takes another kind's place.
pub const MAX_STREAMS: usize = 72;

/// The most streams of one kind a connection carries at once.
pub const fn max_streams_of(kind: StreamKind) -> usize {
    match kind {
        // One of each control stream: a second one is a peer misbehaving.
        StreamKind::Hello
        | StreamKind::Pairing
        | StreamKind::Revoke
        | StreamKind::Management
        | StreamKind::NodeLink
        | StreamKind::Keys => 1,
        // One per pane a window attaches.
        StreamKind::Terminal => 32,
        StreamKind::File => 4,
        StreamKind::Page => 16,
        // The remote screen's two streams, and its parallel asset requests.
        StreamKind::Screen => 8,
    }
}

/// How long a side waits for the other's part of the first exchange, and
/// for a stream's header or its answer.
pub const HELLO_WITHIN: Duration = Duration::from_secs(10);

/// The wait before dialing again after a connection dropped: it starts at
/// the first and doubles to the second, and goes back to the first when the
/// machine wakes or its network changes (B15).
pub const REDIAL_FIRST: Duration = Duration::from_secs(1);
pub const REDIAL_MOST: Duration = Duration::from_secs(60);

/// How long one dial may take before it counts as failed.
pub const DIAL_WITHIN: Duration = Duration::from_secs(30);

/// How long a blocking write waits on a peer that reads nothing before the
/// stream is given up ([`crate::blocking`]).
pub const WRITE_WITHIN: Duration = Duration::from_secs(30);

/// The inbox keeps at most this many events between "recorded" and
/// "applied": what a host's box can hold (B13).
pub const MAX_UNAPPLIED: usize = hide_peer_protocol::event::BOX_MAX_EVENTS;

/// How many connection events a slow listener may fall behind before it
/// skips to the newest.
pub const MAX_EVENTS: usize = 64;

// The caps may not contradict each other. A contradiction fails the build,
// so no change to one number can ship without the others being looked at.
const _: () = {
    // Every machine in the middle of pairing can still be granted.
    assert!(MAX_UNGRANTED <= MAX_GRANTS);
    assert!(MAX_ADMITTING <= MAX_UNGRANTED);
    // A kind at its cap never takes another kind's place: a flood of pages
    // leaves the key stream its own.
    let mut every_kind = 0;
    let mut index = 0;
    while index < StreamKind::ALL.len() {
        let of_kind = max_streams_of(StreamKind::ALL[index]);
        assert!(of_kind >= 1);
        every_kind += of_kind;
        index += 1;
    }
    assert!(every_kind <= MAX_STREAMS);
    assert!(REDIAL_FIRST.as_secs() < REDIAL_MOST.as_secs());
    assert!(HELLO_WITHIN.as_secs() < UNGRANTED_FOR.as_secs());
};
