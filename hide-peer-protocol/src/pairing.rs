//! What the pairing and revoke streams carry (PRD D-30).
//!
//! Pairing settles in two steps: the host saves a `pending` grant that
//! opens nothing and answers [`Pairing::Granted`]; the guest saves its side
//! and sends [`Pairing::Confirm`], which makes the grant `active`.
//! A confirm is accepted with no offer open, answers the same however often
//! it is sent, and never revives a pause or a removal.

use serde::{Deserialize, Serialize};

use crate::frame::{self, FrameError};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Pairing {
    /// Guest to host: the offer's secret (64 hex characters) and who asks.
    /// The guest's key comes from the connection, never from this message.
    Pair {
        secret: String,
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
    pub const TYPES: [&'static str; 5] = ["pair", "granted", "refused", "confirm", "confirmed"];

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
