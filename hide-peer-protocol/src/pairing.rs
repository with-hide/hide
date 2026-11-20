//! What the pairing and revoke streams carry (PRD D-30).
//!
//! Pairing settles in two steps: the host saves a `pending` grant that
//! opens nothing and answers [`Pairing::Granted`]; the guest saves its side
//! and sends [`Pairing::Confirm`], which makes the grant `active`.
//! A confirm is accepted with no offer open, answers the same however often
//! it is sent, and never revives a pause or a removal.
//!
//! Who asks first decides which pairing it is. A guest with a link sends
//! [`Pairing::Pair`], and the host answers [`Pairing::Asking`] once the
//! secret is the offer's, so both operators compare the same words while
//! the host's decides.
//!
//! The words come from one random value of each side, and neither side can
//! choose its own after seeing the other's: the guest's `pair` carries only
//! a commitment to its value, the host's `asking` carries the host's value,
//! and the guest's [`Pairing::Reveal`] then opens the commitment. A machine
//! in the middle has to send its value to one side before it learns the
//! other's, so it cannot make the two screens agree. The link's secret says
//! only that the guest holds the link; the words are not made from it.
//!
//! A machine that already hosts the other sends
//! [`Pairing::Request`] over the connection they share to be let in the
//! other way; no words are compared, because that connection is already
//! each side's proof of the other (D-51).

use serde::{Deserialize, Serialize};

use crate::frame::{self, FrameError};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Pairing {
    /// Guest to host: the offer's secret (64 hex characters), who asks, and
    /// the guest's commitment to its value for the words (64 hex characters:
    /// BLAKE3 of `hide-pair/v1/commit`, the host's key, the guest's key and
    /// the value). A `pair` without one is read, so the host can end the
    /// offer it showed the secret of.
    /// The guest's key comes from the connection, never from this message.
    Pair {
        secret: String,
        device: String,
        name: String,
        #[serde(default)]
        commit: String,
    },
    /// Host to guest: the secret is the offer's. It names the host, the
    /// word list both screens show the confirmation words from, and the
    /// host's value for the words (64 hex characters), drawn after `pair`
    /// arrived.
    Asking {
        device: String,
        name: String,
        words: String,
        nonce: String,
    },
    /// Guest to host: the value its `pair` committed to, once and within
    /// the time a machine answers in. The host's operator is asked only
    /// once it opens that commitment.
    Reveal {
        nonce: String,
    },
    /// A machine that hosts this one asks to be let in the other way.
    Request {
        device: String,
        name: String,
    },
    /// Host to guest: the operator allowed it and a pending grant is saved.
    Granted {
        grant: String,
        device: String,
        name: String,
    },
    Refused {
        reason: PairRefusal,
    },
    Confirm {
        grant: String,
    },
    /// The grant's state after a confirm, one answer per state.
    Confirmed {
        grant: String,
        state: ConfirmState,
    },
}

impl Pairing {
    pub const TYPES: [&'static str; 8] = [
        "pair",
        "asking",
        "reveal",
        "request",
        "granted",
        "refused",
        "confirm",
        "confirmed",
    ];

    /// `None` for a type this build does not know.
    pub fn parse(body: &[u8]) -> Result<Option<Self>, FrameError> {
        match frame::message_type(body)? {
            Some(name) if Self::TYPES.contains(&name.as_str()) => frame::decode(body).map(Some),
            _ => Ok(None),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairRefusal {
    /// No offer is open, or the secret is not its secret.
    NoOffer,
    Declined,
    /// The host already holds its limit of connected machines (B6).
    Full,
    /// Nobody answered within the time an operator is given.
    Unanswered,
    #[serde(other)]
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmState {
    /// Active now, whether this confirm made it so or an earlier one did.
    Active,
    Paused,
    /// Expired, removed, or never there: the guest pairs again.
    Unknown,
}

/// What the revoke stream carries. The answer is the same whether the grant
/// was just removed, was removed before, or never existed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Revoke {
    Revoke { grant: String },
    Revoked { grant: String },
}

impl Revoke {
    pub const TYPES: [&'static str; 2] = ["revoke", "revoked"];

    pub fn parse(body: &[u8]) -> Result<Option<Self>, FrameError> {
        match frame::message_type(body)? {
            Some(name) if Self::TYPES.contains(&name.as_str()) => frame::decode(body).map(Some),
            _ => Ok(None),
        }
    }
}
