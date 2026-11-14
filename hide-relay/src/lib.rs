//! Memory-only opaque relay. The server owns sockets, never Noise sessions.
pub mod apns;
pub mod config;
pub mod event;
pub mod fcm;
pub mod limits;
mod server;
mod state;

pub use server::Relay;

fn protocol_error(error: hide_relay_protocol::Error) -> state::Rejection {
    use hide_relay_protocol::{Error, admission::Refusal};
    match error {
        Error::SizeLimit => state::Rejection::cap(Refusal::FrameLimit, "frame_size"),
        Error::UnsupportedVersion => {
            state::Rejection::new(Refusal::UnsupportedVersion, "unsupported_version")
        }
        Error::InvalidSignature | Error::InvalidKey => {
            state::Rejection::new(Refusal::InvalidProof, "invalid_proof")
        }
        Error::Timeout => state::Rejection::new(Refusal::Timeout, "admission_timeout"),
        _ => state::Rejection::new(Refusal::InvalidMessage, "invalid_message"),
    }
}
