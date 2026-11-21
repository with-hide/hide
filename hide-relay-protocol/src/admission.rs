//! Plaintext routing metadata, never a pairing code or phone credential.
use crate::{Error, decode, encode, random};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub const WIRE_VERSION: u8 = 1;
pub const MAX_CONTROL: usize = 2048;
/// The control messages that carry ciphertext (`push_request` and
/// `fcm_request`), bounded so that the body the relay builds around it stays
/// inside APNs' 4,096 bytes and FCM's 4,096 bytes of data
/// (`docs/RELAY_PROTOCOL.md`, Push relay).
pub const MAX_PUSH_CONTROL: usize = 6144;
pub const ADMISSION_TIMEOUT_MS: u64 = 10_000;
/// A device token is hex, in either case; 32 characters covers the shortest a
/// simulator issues and 200 the longest a device has been seen with.
pub const MIN_TOKEN: usize = 32;
pub const MAX_TOKEN: usize = 200;
pub const MAX_COLLAPSE: usize = 64;
/// An FCM registration token is opaque to everyone but Google: its observed
/// form is base64url, a colon and more base64url, about 160 characters. The
/// bounds leave room for that form to grow and keep the whole `fcm_request`
/// inside `MAX_PUSH_CONTROL`.
pub const MIN_FCM_TOKEN: usize = 32;
pub const MAX_FCM_TOKEN: usize = 1024;
/// The upgrade request header in which a device names the `registered` fields
/// past `push` that it reads, comma-separated. A device released before a
/// field existed refuses a `registered` that carries it (the message denies
/// unknown fields), so the relay adds a field only for a device that named it
/// here, and a relay that does not know the header ignores it
/// (`docs/RELAY_PROTOCOL.md`, Compatibility and deploy order).
pub const REGISTERED_FIELDS_HEADER: &str = "hide-registered-fields";
/// The value a device that reads `registered.fcm` sends in that header.
pub const REGISTERED_FIELD_FCM: &str = "fcm";
/// The RFC 8291 body the relay forwards, base64url.
pub const MAX_CIPHERTEXT: usize = 3000;
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
    Device {
        hello: DeviceHello,
    },
    Proof {
        signature: String,
    },
    Phone {
        version: u8,
        room: String,
    },
    Close {
        connection: u32,
    },
    /// A notification for Apple's push service, carried over the admitted
    /// device socket only. The relay fixes the topic, the push type and the
    /// key ids itself and keeps nothing (`docs/RELAY_PROTOCOL.md`, Push relay).
    #[serde(rename = "push_request")]
    Push(PushRequest),
    /// A notification for Google's Firebase Cloud Messaging, carried over the
    /// admitted device socket only, sent after a `registered` whose `fcm` is
    /// true. The relay fixes everything Google reads besides the token and the
    /// ciphertext (`docs/RELAY_PROTOCOL.md`, Push relay).
    #[serde(rename = "fcm_request")]
    Fcm(FcmRequest),
}

/// Which Apple host carries the notification. There is no default: the phone
/// names the environment it was issued in, and the relay only accepts one.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PushEnvironment {
    Sandbox,
    Production,
}

#[derive(Clone, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PushRequest {
    /// The device's own request id, carried back in the result. Never 0.
    pub id: u32,
    pub environment: PushEnvironment,
    /// The APNs device token, hex (`token_valid`).
    pub token: String,
    /// Apple's plaintext collapse header. The sender makes it opaque with a
    /// per-phone key; it never carries an agent key or a tag.
    pub collapse: String,
    /// The RFC 8291 body, base64url: the only part that holds content.
    pub payload: String,
}

/// Printed without the token, the collapse value or the ciphertext, so a
/// `Debug` of a request names no install and carries no content.
impl std::fmt::Debug for PushRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushRequest")
            .field("id", &self.id)
            .field("environment", &self.environment)
            .finish_non_exhaustive()
    }
}

impl PushRequest {
    /// Which field is wrong, for the relay's content-free diagnostic; `Ok`
    /// when the whole request may go to Apple.
    pub fn check(&self) -> Result<(), PushField> {
        if self.id == 0 {
            return Err(PushField::Shape);
        }
        if !token_valid(&self.token) {
            return Err(PushField::Token);
        }
        if self.collapse.is_empty()
            || self.collapse.len() > MAX_COLLAPSE
            || !self.collapse.bytes().all(collapse_byte)
        {
            return Err(PushField::Collapse);
        }
        payload_valid(&self.payload)
    }

    pub fn validate(&self) -> Result<(), Error> {
        self.check().map_err(|_| Error::InvalidMessage)
    }
}

/// Whether `token` has the shape of an APNs device token: hex, an even
/// length between `MIN_TOKEN` and `MAX_TOKEN`. The one rule the relay and the
/// device that stores a token both apply.
pub fn token_valid(token: &str) -> bool {
    (MIN_TOKEN..=MAX_TOKEN).contains(&token.len())
        && token.len().is_multiple_of(2)
        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A notification for an Android install, to Google's FCM. There is no
/// environment and no collapse value: FCM has one host, and the relay sends
/// the data-only message with no collapse key.
#[derive(Clone, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FcmRequest {
    /// The device's own request id, carried back in the result. Never 0.
    pub id: u32,
    /// The FCM registration token (`fcm_token_valid`).
    pub token: String,
    /// The RFC 8291 body, base64url: the same body hided builds for APNs.
    pub payload: String,
}

/// Printed without the token or the ciphertext, as `PushRequest` is.
impl std::fmt::Debug for FcmRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FcmRequest")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl FcmRequest {
    /// Which field is wrong, for the relay's content-free diagnostic; `Ok`
    /// when the whole request may go to Google.
    pub fn check(&self) -> Result<(), PushField> {
        if self.id == 0 {
            return Err(PushField::Shape);
        }
        if !fcm_token_valid(&self.token) {
            return Err(PushField::Token);
        }
        payload_valid(&self.payload)
    }

    pub fn validate(&self) -> Result<(), Error> {
        self.check().map_err(|_| Error::InvalidMessage)
    }
}

/// Whether `token` has the shape of an FCM registration token: base64url
/// characters and colons, `MIN_FCM_TOKEN` to `MAX_FCM_TOKEN` long. The one
/// rule the relay and the device that stores a token both apply; it also
/// keeps the token from needing any JSON escape in the body Google reads.
pub fn fcm_token_valid(token: &str) -> bool {
    (MIN_FCM_TOKEN..=MAX_FCM_TOKEN).contains(&token.len())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':'))
}

/// The ciphertext both push messages carry: unpadded base64url of 1 to
/// `MAX_CIPHERTEXT` bytes.
fn payload_valid(payload: &str) -> Result<(), PushField> {
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| PushField::Payload)?;
    if payload.is_empty() || payload.len() > MAX_CIPHERTEXT {
        return Err(PushField::Payload);
    }
    Ok(())
}

/// One push request the relay forwards, to Apple or to Google.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Push {
    Apns(PushRequest),
    Fcm(FcmRequest),
}

/// The first thing the relay finds wrong with a push request. It names a
/// field, never a value (`docs/RELAY_PROTOCOL.md`, Push relay).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PushField {
    Shape,
    Environment,
    Token,
    Collapse,
    Payload,
}

impl PushField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shape => "shape",
            Self::Environment => "environment",
            Self::Token => "token",
            Self::Collapse => "collapse",
            Self::Payload => "payload",
        }
    }
}

/// A push message read as far as it can be, so one whose fields are wrong is
/// still answered under its own id instead of closing the device's socket.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PushAttempt {
    /// Not a push message; read it as the rest of the vocabulary.
    NotPush,
    /// A push message whose id cannot be read, so no answer could name it.
    Unaddressable(Error),
    Push {
        id: u32,
        request: Result<Push, PushField>,
    },
}

/// Reads the push message in `text` (`push_request` or `fcm_request`): its id
/// first, so a request whose fields are wrong is still answered under it, then
/// the whole message through the one reader `ClientMessage::decode` uses
/// (serde, then the request's `check`).
pub fn decode_push(text: &str) -> PushAttempt {
    if text.len() > MAX_PUSH_CONTROL {
        return PushAttempt::Unaddressable(Error::SizeLimit);
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
        return PushAttempt::NotPush;
    };
    let fcm = match value.get("type").and_then(serde_json::Value::as_str) {
        Some("push_request") => false,
        Some("fcm_request") => true,
        _ => return PushAttempt::NotPush,
    };
    let id = value
        .get("id")
        .and_then(serde_json::Value::as_u64)
        .and_then(|id| u32::try_from(id).ok())
        .filter(|id| *id != 0);
    let Some(id) = id else {
        return PushAttempt::Unaddressable(Error::InvalidMessage);
    };
    // Only Apple's request names an environment.
    let environment_known = fcm
        || matches!(
            value.get("environment").and_then(serde_json::Value::as_str),
            Some("sandbox" | "production")
        );
    let request = match serde_json::from_value::<ClientMessage>(value) {
        Ok(ClientMessage::Push(request)) => request.check().map(|()| Push::Apns(request)),
        Ok(ClientMessage::Fcm(request)) => request.check().map(|()| Push::Fcm(request)),
        // A missing, extra or mistyped field; the environment is named
        // because the relay records it under its own event.
        _ if !environment_known => Err(PushField::Environment),
        _ => Err(PushField::Shape),
    };
    PushAttempt::Push { id, request }
}

/// base64url without padding, as APNs' collapse ids are carried here.
fn collapse_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PushOutcome {
    Delivered,
    /// The push service says the token no longer names an install; the sender drops it.
    TokenGone,
    /// The push service refuses the token itself (Apple's 400 `BadDeviceToken`, `DeviceTokenNotForTopic`; Google's 400
    /// `INVALID_ARGUMENT` on the token, 403 `SENDER_ID_MISMATCH`): malformed, made for the other environment or for
    /// another app. It does not say the install is gone, so the sender keeps the registration and reports it as a
    /// configuration problem.
    TokenRejected,
    /// A limit refused the request; the socket stays open.
    RateLimited,
    /// The push service could not be reached or its answer means "try later".
    Unavailable,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PushResult {
    pub id: u32,
    pub result: PushOutcome,
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
    /// Admission is confirmed. `push` says whether this relay forwards
    /// `push_request` to Apple and `fcm` whether it forwards `fcm_request` to
    /// Google; a relay that does not know a field leaves it out, which reads as
    /// `false`, so a device never sends a request an older relay would refuse.
    /// `fcm` is written only when true, and only to a device that named it in
    /// `REGISTERED_FIELDS_HEADER` (`docs/RELAY_PROTOCOL.md`).
    Registered {
        #[serde(default)]
        push: bool,
        #[serde(default, skip_serializing_if = "is_false")]
        fcm: bool,
    },
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
    PushResult(PushResult),
}

fn is_false(value: &bool) -> bool {
    !value
}

/// Whether a `REGISTERED_FIELDS_HEADER` value names `field`: comma-separated
/// names, spaces around each ignored.
pub fn registered_fields_name(value: &str, field: &str) -> bool {
    value.split(',').any(|name| name.trim() == field)
}

fn parse<T: serde::de::DeserializeOwned>(text: &str, limit: usize) -> Result<T, Error> {
    if text.len() > limit {
        return Err(Error::SizeLimit);
    }
    serde_json::from_str(text).map_err(|_| {
        // Past the control budget, only a push request (Apple's or Google's)
        // is a message at all; anything else that size, well-formed or not,
        // is over the bound.
        if text.len() > MAX_CONTROL {
            Error::SizeLimit
        } else {
            Error::InvalidMessage
        }
    })
}
impl ClientMessage {
    pub fn decode(text: &str) -> Result<Self, Error> {
        // Only a push or FCM request may carry a body past the control budget; a
        // message of another kind that large is still refused at it.
        let value: Self = parse(text, MAX_PUSH_CONTROL)?;
        if text.len() > MAX_CONTROL && !matches!(value, Self::Push(_) | Self::Fcm(_)) {
            return Err(Error::SizeLimit);
        }
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
            Self::Push(request) => request.validate()?,
            Self::Fcm(request) => request.validate()?,
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
        let value: Self = parse(text, MAX_CONTROL)?;
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
            Self::PushResult(result) if result.id == 0 => return Err(Error::InvalidMessage),
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
