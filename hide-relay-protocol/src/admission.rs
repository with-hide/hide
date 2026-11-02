//! Plaintext routing metadata, never a pairing code or phone credential.
use crate::{Error, decode, encode, random};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const WIRE_VERSION: u8 = 1;
pub const MAX_CONTROL: usize = 2048;
pub const ADMISSION_TIMEOUT_MS: u64 = 10_000;
const CHALLENGE_DOMAIN: &[u8] = b"hide-relay/device-admission/v1\0";

/// Device signing seed, not a raw X25519 Noise secret. Persist privately.
pub struct DeviceKey(SigningKey);
impl DeviceKey {
    pub fn generate() -> Result<Self, Error> {
        let seed = Zeroizing::new(random::<32>()?);
        Ok(Self::from_seed(&seed))
    }
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self(SigningKey::from_bytes(seed))
    }
    pub fn seed(&self) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(self.0.to_bytes())
    }
    pub fn public_key(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub fn sign(&self, hello: &DeviceHello, challenge: &Challenge) -> Result<String, Error> {
        if decode::<32>(&hello.device_key)? != self.public_key() {
            return Err(Error::InvalidKey);
        }
        Ok(encode(
            &self.0.sign(&signed_bytes(hello, challenge)?).to_bytes(),
        ))
    }
}

/// Room = base64url(SHA-256(raw Ed25519 public key)), never a secret.
pub fn room_id(device_key: &[u8; 32]) -> String {
    encode(&Sha256::digest(device_key))
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DeviceHello {
    pub version: u8,
    pub room: String,
    pub device_key: String,
    pub phone_protocol: u32,
}
impl DeviceHello {
    pub fn new(device_key: &[u8; 32], phone_protocol: u32) -> Result<Self, Error> {
        let value = Self {
            version: WIRE_VERSION,
            room: room_id(device_key),
            device_key: encode(device_key),
            phone_protocol,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.version != WIRE_VERSION {
            return Err(Error::UnsupportedVersion);
        }
        let key = decode::<32>(&self.device_key)?;
        VerifyingKey::from_bytes(&key).map_err(|_| Error::InvalidKey)?;
        if self.room != room_id(&key) || self.phone_protocol == 0 {
            return Err(Error::InvalidMessage);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Challenge {
    pub nonce: String,
}
impl Challenge {
    pub fn generate() -> Result<Self, Error> {
        Ok(Self {
            nonce: encode(&random::<32>()?),
        })
    }
}

fn signed_bytes(hello: &DeviceHello, challenge: &Challenge) -> Result<Vec<u8>, Error> {
    hello.validate()?;
    let mut bytes = CHALLENGE_DOMAIN.to_vec();
    bytes.extend_from_slice(&decode::<32>(&challenge.nonce)?);
    bytes.extend_from_slice(&decode::<32>(&hello.room)?);
    bytes.extend_from_slice(&decode::<32>(&hello.device_key)?);
    bytes.extend_from_slice(&hello.phone_protocol.to_be_bytes());
    Ok(bytes)
}

/// One challenge belongs to one socket and is consumed even on failure.
/// The relay supplies a monotonic clock, drops this value on socket close or
/// timeout, and closes on any verification error.
pub struct PendingAdmission {
    hello: DeviceHello,
    challenge: Challenge,
    started_ms: u64,
}
impl PendingAdmission {
    pub fn new(hello: DeviceHello, now_ms: u64) -> Result<Self, Error> {
        hello.validate()?;
        Ok(Self {
            hello,
            challenge: Challenge::generate()?,
            started_ms: now_ms,
        })
    }
    pub fn challenge(&self) -> &Challenge {
        &self.challenge
    }
    pub fn verify(self, signature: &str, now_ms: u64) -> Result<DeviceHello, Error> {
        if now_ms < self.started_ms || now_ms - self.started_ms >= ADMISSION_TIMEOUT_MS {
            return Err(Error::Timeout);
        }
        verify_proof(&self.hello, &self.challenge, signature)?;
        Ok(self.hello)
    }
}

pub fn verify_proof(
    hello: &DeviceHello,
    challenge: &Challenge,
    signature: &str,
) -> Result<(), Error> {
    let bytes = signed_bytes(hello, challenge)?;
    let key = VerifyingKey::from_bytes(&decode::<32>(&hello.device_key)?)
        .map_err(|_| Error::InvalidKey)?;
    let signature = Signature::from_bytes(&decode::<64>(signature)?);
    key.verify_strict(&bytes, &signature)
        .map_err(|_| Error::InvalidSignature)
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    Device { hello: DeviceHello },
    Proof { signature: String },
    Phone { version: u8, room: String },
    Close { connection: u32 },
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    InvalidMessage,
    InvalidProof,
    UnsupportedVersion,
    DeviceOffline,
    RoomOccupied,
    ConnectionLimit,
    RoomLimit,
    RateLimit,
    FrameLimit,
    DailyLimit,
    Timeout,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerMessage {
    Challenge {
        challenge: Challenge,
    },
    Registered,
    Joined {
        connection: u32,
        phone_protocol: u32,
    },
    PeerJoined {
        connection: u32,
    },
    PeerLeft {
        connection: u32,
    },
    Refused {
        reason: Refusal,
    },
}

fn parse<T: serde::de::DeserializeOwned>(text: &str) -> Result<T, Error> {
    if text.len() > MAX_CONTROL {
        return Err(Error::SizeLimit);
    }
    serde_json::from_str(text).map_err(|_| Error::InvalidMessage)
}
impl ClientMessage {
    pub fn decode(text: &str) -> Result<Self, Error> {
        let value: Self = parse(text)?;
        match &value {
            Self::Device { hello } => hello.validate()?,
            Self::Proof { signature } => {
                decode::<64>(signature)?;
            }
            Self::Phone { version, room } => {
                if *version != WIRE_VERSION {
                    return Err(Error::UnsupportedVersion);
                }
                decode::<32>(room)?;
            }
            Self::Close { connection } if *connection == 0 => return Err(Error::InvalidMessage),
            Self::Close { .. } => {}
        }
        Ok(value)
    }
    pub fn encode(&self) -> Result<String, Error> {
        let text = serde_json::to_string(self).map_err(|_| Error::InvalidMessage)?;
        Self::decode(&text)?;
        Ok(text)
    }
}
impl ServerMessage {
    pub fn decode(text: &str) -> Result<Self, Error> {
        let value: Self = parse(text)?;
        match &value {
            Self::Challenge { challenge } => {
                decode::<32>(&challenge.nonce)?;
            }
            Self::Joined {
                connection,
                phone_protocol,
            } if *connection == 0 || *phone_protocol == 0 => return Err(Error::InvalidMessage),
            Self::PeerJoined { connection } | Self::PeerLeft { connection } if *connection == 0 => {
                return Err(Error::InvalidMessage);
            }
            _ => {}
        }
        Ok(value)
    }
    pub fn encode(&self) -> Result<String, Error> {
        let text = serde_json::to_string(self).map_err(|_| Error::InvalidMessage)?;
        Self::decode(&text)?;
        Ok(text)
    }
}
