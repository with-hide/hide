//! Why a connection ended, as the number both sides read (PRD D-79).

/// The code a connection is closed with. A transport carries the number;
/// a number this build does not know reads as [`CloseCode::Other`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseCode {
    /// The side that closed is shutting down or was asked to disconnect.
    Shutdown,
    /// The same side dialed again: this older connection gave way (D-62).
    Replaced,
    /// Both sides dialed at once and this connection lost: the one the
    /// smaller key dialed stays (D-62).
    Duplicate,
    /// No grant is left in either direction (D-62).
    NoGrant,
    /// The two builds share no device connection number, or their contract
    /// files differ (D-40).
    UpdateNeeded,
    /// The peer broke the contract: a bad first exchange, an oversized frame.
    Protocol,
    /// The connection had no relay path left while its machine is attached to
    /// the relay, so it is dialed again (D-64).
    NoRelayPath,
    /// The transport lost the peer without a close from it.
    Lost,
    /// The side that closed holds as many connections in their first
    /// exchange, or without a grant, as it takes: the peer may dial again.
    /// It says nothing about a grant, which [`CloseCode::NoGrant`] does.
    Busy,
    /// A connection dialed to try a path and not chosen: closed before its
    /// first exchange, or because it did not beat the path held
    /// (mission-107). Nothing was admitted, so nothing is recorded.
    Probe,
    Other(u32),
}

impl CloseCode {
    pub const ALL: [Self; 10] = [
        Self::Shutdown,
        Self::Replaced,
        Self::Duplicate,
        Self::NoGrant,
        Self::UpdateNeeded,
        Self::Protocol,
        Self::NoRelayPath,
        Self::Lost,
        Self::Busy,
        Self::Probe,
    ];

    pub const fn number(self) -> u32 {
        match self {
            Self::Shutdown => 0,
            Self::Replaced => 1,
            Self::Duplicate => 2,
            Self::NoGrant => 3,
            Self::UpdateNeeded => 4,
            Self::Protocol => 5,
            Self::NoRelayPath => 6,
            Self::Lost => 7,
            Self::Busy => 8,
            Self::Probe => 9,
            Self::Other(number) => number,
        }
    }

    pub fn from_number(number: u32) -> Self {
        Self::ALL
            .into_iter()
            .find(|code| code.number() == number)
            .unwrap_or(Self::Other(number))
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::Shutdown => "shutdown",
            Self::Replaced => "replaced",
            Self::Duplicate => "duplicate",
            Self::NoGrant => "no_grant",
            Self::UpdateNeeded => "update_needed",
            Self::Protocol => "protocol",
            Self::NoRelayPath => "no_relay_path",
            Self::Lost => "lost",
            Self::Busy => "busy",
            Self::Probe => "probe",
            Self::Other(_) => "other",
        }
    }
}
