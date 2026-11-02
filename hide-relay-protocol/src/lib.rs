//! Shared mobile relay wire contract. No sockets, persistence or phone admission.
//! A Noise handshake authenticates the device only; hided must check the phone's
//! existing credential in the encrypted transport before accepting any request.
#![forbid(unsafe_code)]

pub mod admission;
pub mod frame;
pub mod noise;
pub mod qr;
#[cfg(target_arch = "wasm32")]
mod wasm;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};

/// Fixed, content-free failure codes safe for a diagnostic's reason field.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidMessage,
    UnsupportedVersion,
    SizeLimit,
    InvalidKey,
    InvalidSignature,
    InvalidQr,
    RandomUnavailable,
    SessionClosed,
    InvalidState,
    AuthenticationFailed,
    Timeout,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidMessage => "invalid_message",
            Self::UnsupportedVersion => "unsupported_version",
            Self::SizeLimit => "size_limit",
            Self::InvalidKey => "invalid_key",
            Self::InvalidSignature => "invalid_signature",
            Self::InvalidQr => "invalid_qr",
            Self::RandomUnavailable => "random_unavailable",
            Self::SessionClosed => "session_closed",
            Self::InvalidState => "invalid_state",
            Self::AuthenticationFailed => "authentication_failed",
            Self::Timeout => "timeout",
        })
    }
}
impl std::error::Error for Error {}

pub(crate) fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}
pub(crate) fn decode<const N: usize>(text: &str) -> Result<[u8; N], Error> {
    if text.len() != (N * 8).div_ceil(6) {
        return Err(Error::InvalidMessage);
    }
    URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| Error::InvalidMessage)?
        .try_into()
        .map_err(|_| Error::InvalidMessage)
}
pub(crate) fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| Error::RandomUnavailable)?;
    Ok(bytes)
}
