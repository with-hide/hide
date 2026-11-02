//! Phone initiator and device responder, one ordered cipher state per direction.
use crate::{
    Error,
    frame::{Frame, Kind, MAX_PAYLOAD, MAX_PLAINTEXT},
};
use zeroize::{Zeroize, Zeroizing};

pub const PROTOCOL: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2b";
pub const PROLOGUE: &[u8] = b"hide-mobile-relay/v1\0";
/// Maximum serialized phone wire message, before encryption.
pub const MAX_MESSAGE: usize = 256 * 1024;
pub const REASSEMBLY_TIMEOUT_MS: u64 = 10_000;
const CHUNK_HEADER: usize = 8;
const CHUNK_DATA: usize = MAX_PLAINTEXT - CHUNK_HEADER;

struct Incoming {
    total: usize,
    started_ms: u64,
    bytes: Zeroizing<Vec<u8>>,
}

pub const NOISE_MAX_MESSAGE: usize = 65_535;
pub const NOISE_MAX_PLAINTEXT: usize = 65_519;

/// A raw X25519 private scalar and public key. This is not an Ed25519 seed.
pub struct NoiseKeypair {
    secret: Zeroizing<[u8; 32]>,
    public: [u8; 32],
}
impl NoiseKeypair {
    /// Restore a persisted raw X25519 scalar, never an Ed25519 signing seed.
    pub fn from_secret(raw_secret: &[u8; 32]) -> Result<Self, Error> {
        use snow::resolvers::{CryptoResolver, DefaultResolver};
        let mut dh = DefaultResolver
            .resolve_dh(&snow::params::DHChoice::Curve25519)
            .ok_or(Error::InvalidKey)?;
        dh.set(raw_secret);
        let public = dh.pubkey().try_into().map_err(|_| Error::InvalidKey)?;
        Ok(Self {
            secret: Zeroizing::new(*raw_secret),
            public,
        })
    }
    pub fn secret(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(*self.secret)
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.public
    }

    pub fn generate() -> Result<Self, Error> {
        let mut pair = builder()?
            .generate_keypair()
            .map_err(|_| Error::RandomUnavailable)?;
        let secret = Zeroizing::new(
            pair.private
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidKey)?,
        );
        pair.private.zeroize();
        let public = pair
            .public
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidKey)?;
        Ok(Self { secret, public })
    }
}

fn builder<'a>() -> Result<snow::Builder<'a>, Error> {
    Ok(snow::Builder::new(
        PROTOCOL.parse().map_err(|_| Error::InvalidState)?,
    ))
}

enum State {
    Handshake(Box<snow::HandshakeState>),
    Transport(Box<snow::TransportState>),
    Closed,
}

/// Own for exactly one phone socket. Never clone, reset nonces or reuse after
/// error. All failed operations destroy the state before returning their code.
pub struct Session {
    connection: u32,
    state: State,
    incoming: Option<Incoming>,
    last_now_ms: Option<u64>,
}
impl Session {
    pub fn initiator(remote_public: &[u8; 32], connection: u32) -> Result<Self, Error> {
        let prologue = prologue(connection)?;
        let state = builder()?
            .prologue(&prologue)
            .map_err(|_| Error::InvalidState)?
            .remote_public_key(remote_public)
            .map_err(|_| Error::InvalidKey)?
            .build_initiator()
            .map_err(|_| Error::InvalidKey)?;
        Ok(Self {
            connection,
            state: State::Handshake(Box::new(state)),
            incoming: None,
            last_now_ms: None,
        })
    }
    /// The responder accepts only the Noise key type, not a signing seed.
    /// ```compile_fail
    /// use hide_relay_protocol::noise::Session;
    /// let signing_seed = [1_u8; 32];
    /// Session::responder(&signing_seed, 7);
    /// ```
    pub fn responder(key: &NoiseKeypair, connection: u32) -> Result<Self, Error> {
        let prologue = prologue(connection)?;
        let state = builder()?
            .prologue(&prologue)
            .map_err(|_| Error::InvalidState)?
            .local_private_key(&*key.secret)
            .map_err(|_| Error::InvalidKey)?
            .build_responder()
            .map_err(|_| Error::InvalidKey)?;
        Ok(Self {
            connection,
            state: State::Handshake(Box::new(state)),
            incoming: None,
            last_now_ms: None,
        })
    }
    pub fn is_transport(&self) -> bool {
        matches!(self.state, State::Transport(_))
    }
    pub fn close(&mut self) {
        self.state = State::Closed;
        self.incoming = None;
    }

    /// Empty handshake payloads only. Credentials can only use `encrypt`
    /// after both handshake messages have been processed.
    pub fn write_handshake(&mut self) -> Result<Vec<u8>, Error> {
        let state = std::mem::replace(&mut self.state, State::Closed);
        let State::Handshake(mut handshake) = state else {
            return Err(state_error(&state));
        };
        let mut payload = [0; 48];
        let len = handshake
            .write_message(&[], &mut payload)
            .map_err(|_| Error::InvalidState)?;
        let frame = Frame {
            kind: Kind::Handshake,
            connection: self.connection,
            payload: &payload[..len],
        }
        .encode()?;
        self.finish_handshake(handshake)?;
        Ok(frame)
    }
    pub fn read_handshake(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let state = std::mem::replace(&mut self.state, State::Closed);
        let State::Handshake(mut handshake) = state else {
            return Err(state_error(&state));
        };
        let frame = self.read_frame(bytes, Kind::Handshake)?;
        // Both NK messages are exactly e(32) + empty authenticated payload(16).
        if frame.payload.len() != 48 {
            return Err(Error::InvalidMessage);
        }
        let mut empty = [];
        let len = handshake
            .read_message(frame.payload, &mut empty)
            .map_err(|_| Error::AuthenticationFailed)?;
        if len != 0 {
            return Err(Error::InvalidMessage);
        }
        self.finish_handshake(handshake)
    }
    fn finish_handshake(&mut self, handshake: Box<snow::HandshakeState>) -> Result<(), Error> {
        self.state = if handshake.is_handshake_finished() {
            State::Transport(Box::new(
                handshake
                    .into_transport_mode()
                    .map_err(|_| Error::InvalidState)?,
            ))
        } else {
            State::Handshake(handshake)
        };
        Ok(())
    }
    /// Internal framing primitive; external callers always use application messages.
    pub(crate) fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let result = self.encrypt_frame(plaintext);
        if result.is_err() {
            self.close();
        }
        result
    }
    fn encrypt_frame(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let state = std::mem::replace(&mut self.state, State::Closed);
        let State::Transport(mut transport) = state else {
            return Err(state_error(&state));
        };
        if plaintext.len() > MAX_PLAINTEXT {
            return Err(Error::SizeLimit);
        }
        let mut payload = vec![0; plaintext.len() + 16];
        let len = transport
            .write_message(plaintext, &mut payload)
            .map_err(|_| Error::AuthenticationFailed)?;
        let frame = Frame {
            kind: Kind::Transport,
            connection: self.connection,
            payload: &payload[..len],
        }
        .encode()?;
        self.state = State::Transport(transport);
        Ok(frame)
    }
    pub(crate) fn decrypt(&mut self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let result = self.decrypt_frame(bytes);
        if result.is_err() {
            self.close();
        }
        result
    }
    fn decrypt_frame(&mut self, bytes: &[u8]) -> Result<Vec<u8>, Error> {
        let state = std::mem::replace(&mut self.state, State::Closed);
        let State::Transport(mut transport) = state else {
            return Err(state_error(&state));
        };
        let frame = self.read_frame(bytes, Kind::Transport)?;
        let mut plaintext = Zeroizing::new(vec![0; frame.payload.len()]);
        let len = transport
            .read_message(frame.payload, &mut plaintext)
            .map_err(|_| Error::AuthenticationFailed)?;
        let result = plaintext[..len].to_vec();
        self.state = State::Transport(transport);
        Ok(result)
    }
    /// Ordered frames of one application message. Send every frame in order;
    /// never interleave messages on a direction.
    pub fn send_message(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
        let result = self.split_message(bytes);
        if result.is_err() {
            self.close();
        }
        result
    }
    fn split_message(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
        if bytes.len() > MAX_MESSAGE {
            return Err(Error::SizeLimit);
        }
        let mut frames = Vec::new();
        let mut offset = 0;
        loop {
            let end = (offset + CHUNK_DATA).min(bytes.len());
            let mut chunk = Zeroizing::new(Vec::with_capacity(CHUNK_HEADER + end - offset));
            chunk.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            chunk.extend_from_slice(&(offset as u32).to_be_bytes());
            chunk.extend_from_slice(&bytes[offset..end]);
            frames.push(self.encrypt(&chunk)?);
            if end == bytes.len() {
                return Ok(frames);
            }
            offset = end;
        }
    }
    /// None means one bounded message is incomplete. Use one monotonic clock
    /// for now_ms and check_timeout, and poll check_timeout even with no input.
    pub fn recv_message(&mut self, frame: &[u8], now_ms: u64) -> Result<Option<Vec<u8>>, Error> {
        let result = self.join_message(frame, now_ms);
        if result.is_err() {
            self.close();
        }
        result
    }
    fn join_message(&mut self, frame: &[u8], now_ms: u64) -> Result<Option<Vec<u8>>, Error> {
        self.check_timeout(now_ms)?;
        let chunk = Zeroizing::new(self.decrypt(frame)?);
        if chunk.len() < CHUNK_HEADER {
            return Err(Error::InvalidMessage);
        }
        let total =
            u32::from_be_bytes(chunk[..4].try_into().map_err(|_| Error::InvalidMessage)?) as usize;
        let offset =
            u32::from_be_bytes(chunk[4..8].try_into().map_err(|_| Error::InvalidMessage)?) as usize;
        if total > MAX_MESSAGE {
            return Err(Error::SizeLimit);
        }
        let incoming = self.incoming.get_or_insert_with(|| Incoming {
            total,
            started_ms: now_ms,
            bytes: Zeroizing::new(Vec::with_capacity(total)),
        });
        // Canonical chunks: all but the last fill the available frame. Empty
        // messages consist of exactly one header, never a stream of empty parts.
        if total != incoming.total
            || offset != incoming.bytes.len()
            || offset > total
            || chunk.len() - CHUNK_HEADER != (total - offset).min(CHUNK_DATA)
        {
            return Err(Error::InvalidMessage);
        }
        incoming.bytes.extend_from_slice(&chunk[CHUNK_HEADER..]);
        if incoming.bytes.len() == total {
            let complete = self.incoming.take().ok_or(Error::InvalidState)?;
            return Ok(Some(complete.bytes.to_vec()));
        }
        Ok(None)
    }
    /// The socket owner must call this on its timer as well as on each frame.
    /// A backwards clock is an error; a deadline is never extended by progress.
    pub fn check_timeout(&mut self, now_ms: u64) -> Result<(), Error> {
        let error = if matches!(self.state, State::Closed) {
            Some(Error::SessionClosed)
        } else if self.last_now_ms.is_some_and(|last| now_ms < last) {
            Some(Error::InvalidState)
        } else if self
            .incoming
            .as_ref()
            .is_some_and(|incoming| now_ms - incoming.started_ms >= REASSEMBLY_TIMEOUT_MS)
        {
            Some(Error::Timeout)
        } else {
            None
        };
        if let Some(error) = error {
            self.close();
            return Err(error);
        }
        self.last_now_ms = Some(now_ms);
        Ok(())
    }
    fn read_frame<'a>(&self, bytes: &'a [u8], kind: Kind) -> Result<Frame<'a>, Error> {
        let frame = Frame::decode(bytes)?;
        if frame.kind != kind || frame.connection != self.connection {
            return Err(Error::InvalidMessage);
        }
        Ok(frame)
    }
}
fn state_error(state: &State) -> Error {
    if matches!(state, State::Closed) {
        Error::SessionClosed
    } else {
        Error::InvalidState
    }
}
fn prologue(connection: u32) -> Result<Vec<u8>, Error> {
    if connection == 0 {
        return Err(Error::InvalidMessage);
    }
    let mut bytes = PROLOGUE.to_vec();
    bytes.extend_from_slice(&connection.to_be_bytes());
    Ok(bytes)
}

const _: () = assert!(MAX_PAYLOAD <= NOISE_MAX_MESSAGE);
const _: () = assert!(MAX_PLAINTEXT <= NOISE_MAX_PLAINTEXT);

#[cfg(test)]
mod tests {
    use super::*;
    fn pair() -> (Session, Session) {
        let key = NoiseKeypair::generate().unwrap();
        let mut phone = Session::initiator(&key.public_key(), 7).unwrap();
        let mut device = Session::responder(&key, 7).unwrap();
        device
            .read_handshake(&phone.write_handshake().unwrap())
            .unwrap();
        phone
            .read_handshake(&device.write_handshake().unwrap())
            .unwrap();
        (phone, device)
    }
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn authenticated_malformed_chunks_are_fatal() {
        for (total, offset, data) in [
            (MAX_MESSAGE + 1, 0, vec![]),
            (3, 1, vec![1, 2]),
            (3, 0, vec![1]),
            (0, 0, vec![1]),
        ] {
            let (mut phone, mut device) = pair();
            let mut chunk = (total as u32).to_be_bytes().to_vec();
            chunk.extend_from_slice(&(offset as u32).to_be_bytes());
            chunk.extend_from_slice(&data);
            let frame = phone.encrypt(&chunk).unwrap();
            assert!(device.recv_message(&frame, 0).is_err());
            assert_eq!(device.send_message(b"retry"), Err(Error::SessionClosed));
        }
    }
}
