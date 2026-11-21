//! A JSON message on a stream: a big-endian `u32` length, then that many
//! bytes of one JSON object. Hello frames, pairing and management messages
//! use it.

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub const LENGTH_BYTES: usize = 4;

/// The most bytes one message's JSON takes. An event carries a letter's
/// body, so this is far above a hello.
pub const MAX_MESSAGE: usize = 1024 * 1024;

#[derive(Debug)]
pub enum FrameError {
    /// The message is longer than the reader accepts.
    TooLong { length: usize, max: usize },
    /// The bytes are not the JSON the reader expects.
    Json(serde_json::Error),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { length, max } => {
                write!(
                    f,
                    "a message of {length} bytes is over the {max} byte limit"
                )
            }
            Self::Json(error) => write!(f, "a message is not the expected JSON: {error}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// The length prefix and the JSON of `message`, ready to write.
pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(message).map_err(FrameError::Json)?;
    if body.len() > MAX_MESSAGE {
        return Err(FrameError::TooLong {
            length: body.len(),
            max: MAX_MESSAGE,
        });
    }
    let mut bytes = Vec::with_capacity(LENGTH_BYTES + body.len());
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&body);
    Ok(bytes)
}

/// How many bytes of JSON follow a length prefix, refused above `max`
/// before any of them is read.
pub fn body_length(prefix: [u8; LENGTH_BYTES], max: usize) -> Result<usize, FrameError> {
    let length = u32::from_be_bytes(prefix) as usize;
    if length > max {
        return Err(FrameError::TooLong { length, max });
    }
    Ok(length)
}

pub fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, FrameError> {
    serde_json::from_slice(body).map_err(FrameError::Json)
}

/// The `t` field that names a tagged message's type, when it has one.
/// A reader skips a type it does not know instead of failing (D-65).
pub fn message_type(body: &[u8]) -> Result<Option<String>, FrameError> {
    #[derive(serde::Deserialize)]
    struct Tagged {
        t: Option<String>,
    }
    Ok(decode::<Tagged>(body)?.t)
}
