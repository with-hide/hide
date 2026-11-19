//! The first exchange of a connection (PRD D-40, D-65).
//!
//! It is fixed and does not depend on any number: each side writes
//! [`MAGIC`], a big-endian `u32` length and one JSON [`Hello`] on the
//! connection's first stream, so two builds that share no device connection
//! number can still tell each other so and ask for an update
//! ([`Hello::update`]).
//! After it the same stream carries [`HelloFrame`]s for the connection's
//! whole life.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::frame::{self, FrameError};

/// The eight bytes before a [`Hello`]. They never change.
pub const MAGIC: [u8; 8] = *b"HIDEPEER";

/// The most bytes a [`Hello`]'s JSON takes.
pub const MAX_HELLO: usize = 16 * 1024;

/// The number of the stream header and message frame layout
/// ([`crate::stream`], [`crate::frame`]).
pub const FRAMING: u32 = 1;

/// The device connection numbers this build speaks. A number rises only
/// when an older reader would break, at most once a release, and the oldest
/// is the number of the oldest release CI still connects to (D-40).
pub const OLDEST_LINK: u32 = 1;
pub const NEWEST_LINK: u32 = 1;

/// What this build can do beyond its device connection number. A sender
/// uses an act only when the peer listed its capability.
pub const CAPABILITIES: &[&str] = &[];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkRange {
    pub oldest: u32,
    pub newest: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub framing: u32,
    pub link: LinkRange,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// The sender's Hide version, to name in an update notice.
    pub build: String,
    /// The sender's product device id. It is a claim until admission pairs
    /// it with the connection's key (D-29).
    pub device: String,
    /// The hash of the sender's contract file. Only a build that is not a
    /// release sends it, and two such builds connect only when it matches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<String>,
    /// "Check the official release, install it and restart": what
    /// [`HelloFrame::Update`] says on a connection that lives, said here
    /// for one that does not, because the two builds share no number and
    /// the connection ends with this exchange. Only a side that holds an
    /// active grant on the reader is heard (D-40).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub update: bool,
}

impl Hello {
    /// This build's hello. `release` is whether the build is an official
    /// release, which is governed by the numbers alone.
    pub fn ours(build: &str, device: &str, release: bool) -> Self {
        Self {
            framing: FRAMING,
            link: LinkRange {
                oldest: OLDEST_LINK,
                newest: NEWEST_LINK,
            },
            capabilities: CAPABILITIES.iter().map(|name| (*name).to_owned()).collect(),
            build: build.to_owned(),
            device: device.to_owned(),
            contract: (!release).then(crate::contract::hash),
            update: false,
        }
    }

    /// The magic, the length and the JSON, ready to write.
    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        let body = serde_json::to_vec(self).map_err(FrameError::Json)?;
        if body.len() > MAX_HELLO {
            return Err(FrameError::TooLong {
                length: body.len(),
                max: MAX_HELLO,
            });
        }
        let mut bytes = Vec::with_capacity(PREFIX_BYTES + body.len());
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&body);
        Ok(bytes)
    }

    pub fn decode(body: &[u8]) -> Result<Self, FrameError> {
        frame::decode(body)
    }
}

/// The magic and the length before a hello's JSON.
pub const PREFIX_BYTES: usize = MAGIC.len() + frame::LENGTH_BYTES;

/// How many bytes of JSON follow `prefix`, or `None` when the stream does
/// not start with [`MAGIC`] or names more than [`MAX_HELLO`].
pub fn body_length(prefix: [u8; PREFIX_BYTES]) -> Option<usize> {
    let (magic, length) = prefix.split_at(MAGIC.len());
    if magic != MAGIC {
        return None;
    }
    let length = u32::from_be_bytes(length.try_into().ok()?) as usize;
    (length <= MAX_HELLO).then_some(length)
}

/// What two builds agreed to speak.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Agreed {
    /// The newest device connection number both speak.
    pub link: u32,
    /// The capabilities both listed.
    pub capabilities: BTreeSet<String>,
}

impl Agreed {
    pub fn has(&self, capability: &str) -> bool {
        self.capabilities.contains(capability)
    }
}

/// Which side has to update before the two connect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behind {
    Ours,
    Theirs,
    /// Two builds that are not releases and whose contract files differ:
    /// neither is older by a number.
    Either,
}

/// Decides what two hellos share, the same way on both sides. `Err` names
/// the side that is behind; the connection is closed
/// [`crate::CloseCode::UpdateNeeded`] and both show an update notice.
pub fn agree(ours: &Hello, theirs: &Hello) -> Result<Agreed, Behind> {
    if ours.framing != theirs.framing {
        return Err(if ours.framing < theirs.framing {
            Behind::Ours
        } else {
            Behind::Theirs
        });
    }
    if ours.link.newest < theirs.link.oldest {
        return Err(Behind::Ours);
    }
    if theirs.link.newest < ours.link.oldest {
        return Err(Behind::Theirs);
    }
    if let (Some(ours), Some(theirs)) = (&ours.contract, &theirs.contract)
        && ours != theirs
    {
        return Err(Behind::Either);
    }
    let theirs_can: BTreeSet<&str> = theirs.capabilities.iter().map(String::as_str).collect();
    Ok(Agreed {
        link: ours.link.newest.min(theirs.link.newest),
        capabilities: ours
            .capabilities
            .iter()
            .filter(|name| theirs_can.contains(name.as_str()))
            .cloned()
            .collect(),
    })
}

/// The state of the grant the sender gives the reader, as the reader may
/// know it. A machine learns nothing about another machine's grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantState {
    /// No grant, and none that was taken away.
    None,
    /// Saved by the host, not yet confirmed by the guest: it opens nothing.
    Pending,
    Active,
    Paused,
    /// The host took it away.
    Revoked,
}

/// What travels on the hello stream after the first exchange.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum HelloFrame {
    /// The grant the sender gives the reader, sent once after the first
    /// exchange and again whenever it changes. A stream the reader opens
    /// carries `generation` (D-62, D-66).
    Grant { state: GrantState, generation: u64 },
    /// "Check the official release, install it and restart." Only a side
    /// that holds an active grant on the reader sends it (D-40).
    Update,
    UpdateAnswer {
        started: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

impl HelloFrame {
    pub const TYPES: [&'static str; 3] = ["grant", "update", "update_answer"];

    /// `None` for a type this build does not know.
    pub fn parse(body: &[u8]) -> Result<Option<Self>, FrameError> {
        match frame::message_type(body)? {
            Some(name) if Self::TYPES.contains(&name.as_str()) => frame::decode(body).map(Some),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello(oldest: u32, newest: u32, capabilities: &[&str], contract: Option<&str>) -> Hello {
        Hello {
            framing: FRAMING,
            link: LinkRange { oldest, newest },
            capabilities: capabilities.iter().map(|name| (*name).to_owned()).collect(),
            build: "1.3.0".into(),
            device: "device".into(),
            contract: contract.map(str::to_owned),
            update: false,
        }
    }

    #[test]
    fn overlapping_numbers_connect_at_the_newest_both_speak() {
        let agreed = agree(
            &hello(1, 3, &["a", "b"], None),
            &hello(2, 5, &["b", "c"], None),
        )
        .expect("the ranges overlap");
        assert_eq!(agreed.link, 3);
        assert!(agreed.has("b") && !agreed.has("a") && !agreed.has("c"));
    }

    #[test]
    fn the_side_whose_newest_is_below_the_others_oldest_is_behind() {
        assert_eq!(
            agree(&hello(1, 1, &[], None), &hello(2, 3, &[], None)),
            Err(Behind::Ours)
        );
        assert_eq!(
            agree(&hello(2, 3, &[], None), &hello(1, 1, &[], None)),
            Err(Behind::Theirs)
        );
    }

    #[test]
    fn two_development_builds_connect_only_with_the_same_contract() {
        let ours = hello(1, 1, &[], Some("aa"));
        assert_eq!(
            agree(&ours, &hello(1, 1, &[], Some("bb"))),
            Err(Behind::Either)
        );
        assert!(agree(&ours, &hello(1, 1, &[], Some("aa"))).is_ok());
        // A release is governed by its numbers alone.
        assert!(agree(&ours, &hello(1, 1, &[], None)).is_ok());
    }

    #[test]
    fn a_hello_reads_back_and_ignores_what_it_does_not_know() {
        let ours = Hello::ours("1.3.0", "device", false);
        assert!(ours.contract.is_some());
        let bytes = ours.encode().unwrap();
        let prefix: [u8; PREFIX_BYTES] = bytes[..PREFIX_BYTES].try_into().unwrap();
        let length = body_length(prefix).expect("our own prefix");
        assert_eq!(
            Hello::decode(&bytes[PREFIX_BYTES..PREFIX_BYTES + length]).unwrap(),
            ours
        );

        let later = br#"{"framing":1,"link":{"oldest":1,"newest":9,"note":"x"},"build":"9","device":"d","mood":"fine"}"#;
        let read = Hello::decode(later).expect("unknown fields are ignored");
        assert_eq!(read.link.newest, 9);
        // An update is asked for only by a hello that says so, and a hello
        // that asks for none is the hello an earlier reader knows.
        assert!(!read.update);
        assert!(!String::from_utf8_lossy(&bytes).contains("update"));
        let asking = Hello {
            update: true,
            ..ours.clone()
        };
        let bytes = asking.encode().unwrap();
        assert!(Hello::decode(&bytes[PREFIX_BYTES..]).unwrap().update);
        assert_eq!(HelloFrame::parse(br#"{"t":"later_thing"}"#).unwrap(), None);
        assert_eq!(body_length(*b"NOTHIDE!\0\0\0\x01"), None);
    }
}
