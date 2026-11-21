//! The kinds of stream one connection carries (PRD D-62).
//!
//! A stream starts with [`HEADER_BYTES`] from the side that opened it: the
//! kind's tag and the generation of the grant it runs under. The side that
//! accepts answers [`ANSWER_BYTES`]: a [`Status`] and the generation it holds
//! now. After that the bytes are the kind's own.

use serde::{Deserialize, Serialize};

/// What a stream is for. The tag is the first byte on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamKind {
    /// The first stream of a connection: the fixed first exchange, then each
    /// side's grant state and update requests.
    Hello,
    Pairing,
    Revoke,
    /// Host and guest management messages; opened by the grant holder,
    /// written by both.
    Management,
    NodeLink,
    /// Key input, apart from terminal output and ahead of everything.
    Keys,
    Terminal,
    File,
    Page,
    /// Another machine's view of this one: its socket's control and
    /// terminal frames (PRD D-67).
    Screen,
    /// The same view's page, its assets and its file bytes, behind
    /// everything else, so neither a page load nor a file read holds a
    /// key's echo (D-67).
    ScreenBulk,
}

impl StreamKind {
    pub const ALL: [Self; 11] = [
        Self::Hello,
        Self::Pairing,
        Self::Revoke,
        Self::Management,
        Self::NodeLink,
        Self::Keys,
        Self::Terminal,
        Self::File,
        Self::Page,
        Self::Screen,
        Self::ScreenBulk,
    ];

    pub const fn tag(self) -> u8 {
        match self {
            Self::Hello => 1,
            Self::Pairing => 2,
            Self::Revoke => 3,
            Self::Management => 4,
            Self::NodeLink => 5,
            Self::Keys => 6,
            Self::Terminal => 7,
            Self::File => 8,
            Self::Page => 9,
            Self::Screen => 10,
            Self::ScreenBulk => 12,
        }
    }

    pub fn from_tag(tag: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.tag() == tag)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Hello => "hello",
            Self::Pairing => "pairing",
            Self::Revoke => "revoke",
            Self::Management => "management",
            Self::NodeLink => "node_link",
            Self::Keys => "keys",
            Self::Terminal => "terminal",
            Self::File => "file",
            Self::Page => "page",
            Self::Screen => "screen",
            Self::ScreenBulk => "screen_bulk",
        }
    }

    /// The order a transport sends in when streams compete: a larger number
    /// first. Keys are highest, because a key that shares a stream with a
    /// flood of output arrived seconds late over the relay; the bulk kinds
    /// are lowest (D-62).
    pub const fn priority(self) -> i32 {
        match self {
            Self::Keys => 100,
            Self::Hello | Self::Pairing | Self::Revoke | Self::Management => 75,
            Self::NodeLink => 50,
            Self::Terminal | Self::Screen => 0,
            Self::File | Self::Page | Self::ScreenBulk => -100,
        }
    }

    /// Whether a side opens this kind only under an active grant it holds.
    /// A side without one opens the others and nothing else (D-62).
    pub const fn needs_grant(self) -> bool {
        !matches!(self, Self::Hello | Self::Pairing | Self::Revoke)
    }
}

/// The bytes that open a stream: the kind's tag, then the generation as a
/// big-endian `u64` (zero for a kind that needs no grant).
pub const HEADER_BYTES: usize = 9;

/// The bytes that answer: a [`Status`], then the generation the accepting
/// side holds now as a big-endian `u64`.
pub const ANSWER_BYTES: usize = 9;

pub fn encode_header(kind: StreamKind, generation: u64) -> [u8; HEADER_BYTES] {
    let mut bytes = [0u8; HEADER_BYTES];
    bytes[0] = kind.tag();
    bytes[1..].copy_from_slice(&generation.to_be_bytes());
    bytes
}

/// The tag is returned as it came: an unknown one is answered
/// [`Status::UnknownKind`], not treated as a broken connection.
pub fn decode_header(bytes: [u8; HEADER_BYTES]) -> (u8, u64) {
    let mut generation = [0u8; 8];
    generation.copy_from_slice(&bytes[1..]);
    (bytes[0], u64::from_be_bytes(generation))
}

/// What the accepting side says to a stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Accepted,
    /// The opener holds no active grant for this kind.
    NoGrant,
    /// The grant changed since the opener read its generation; the answer
    /// carries the one that holds now.
    Stale,
    /// The connection or the kind is at its stream limit.
    Limit,
    /// This build does not know the kind.
    UnknownKind,
}

impl Status {
    pub const ALL: [Self; 5] = [
        Self::Accepted,
        Self::NoGrant,
        Self::Stale,
        Self::Limit,
        Self::UnknownKind,
    ];

    pub const fn tag(self) -> u8 {
        match self {
            Self::Accepted => 0,
            Self::NoGrant => 1,
            Self::Stale => 2,
            Self::Limit => 3,
            Self::UnknownKind => 4,
        }
    }

    pub fn from_tag(tag: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.tag() == tag)
    }
}

pub fn encode_answer(status: Status, generation: u64) -> [u8; ANSWER_BYTES] {
    let mut bytes = [0u8; ANSWER_BYTES];
    bytes[0] = status.tag();
    bytes[1..].copy_from_slice(&generation.to_be_bytes());
    bytes
}

pub fn decode_answer(bytes: [u8; ANSWER_BYTES]) -> Option<(Status, u64)> {
    let mut generation = [0u8; 8];
    generation.copy_from_slice(&bytes[1..]);
    Some((Status::from_tag(bytes[0])?, u64::from_be_bytes(generation)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_go_first_and_bulk_goes_last() {
        for kind in StreamKind::ALL {
            assert_eq!(StreamKind::from_tag(kind.tag()), Some(kind));
            if kind != StreamKind::Keys {
                assert!(kind.priority() < StreamKind::Keys.priority(), "{kind:?}");
            }
            if !matches!(
                kind,
                StreamKind::File | StreamKind::Page | StreamKind::ScreenBulk
            ) {
                assert!(kind.priority() > StreamKind::File.priority(), "{kind:?}");
            }
        }
        assert_eq!(StreamKind::File.priority(), StreamKind::Page.priority());
    }

    #[test]
    fn a_header_and_its_answer_round_trip() {
        let header = encode_header(StreamKind::Keys, 7);
        assert_eq!(decode_header(header), (StreamKind::Keys.tag(), 7));
        let answer = encode_answer(Status::Stale, 8);
        assert_eq!(decode_answer(answer), Some((Status::Stale, 8)));
        assert_eq!(decode_answer([9; ANSWER_BYTES]), None);
    }
}
