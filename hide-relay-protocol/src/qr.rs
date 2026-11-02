//! Only the fragment carries pairing material. The request URL is constant.
use crate::{Error, admission::room_id, decode, encode};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};

pub const APP_URL: &str = "https://app.withhide.dev/";
pub const MAX_QR_URL: usize = 1024;

/// Keys are raw public bytes encoded base64url without padding. `device_key`
/// is Ed25519; `noise_key` is X25519. `code` is the existing random 16-byte code.
#[derive(Clone, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PairingQr {
    pub v: u8,
    pub device_key: String,
    pub noise_key: String,
    pub room: String,
    pub code: String,
}
impl PairingQr {
    pub fn new(
        device_key: &[u8; 32],
        noise_key: &[u8; 32],
        code: &[u8; 16],
    ) -> Result<Self, Error> {
        let value = Self {
            v: 2,
            device_key: encode(device_key),
            noise_key: encode(noise_key),
            room: room_id(device_key),
            code: encode(code),
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), Error> {
        if self.v != 2 {
            return Err(Error::UnsupportedVersion);
        }
        let key = decode::<32>(&self.device_key).map_err(|_| Error::InvalidQr)?;
        ed25519_dalek::VerifyingKey::from_bytes(&key).map_err(|_| Error::InvalidQr)?;
        let noise_key = decode::<32>(&self.noise_key).map_err(|_| Error::InvalidQr)?;
        decode::<16>(&self.code).map_err(|_| Error::InvalidQr)?;
        if self.room != room_id(&key) || noise_key == [0; 32] {
            return Err(Error::InvalidQr);
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<String, Error> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|_| Error::InvalidQr)?;
        Ok(format!("{APP_URL}#pair={}", encode(&bytes)))
    }
    pub fn decode(url: &str) -> Result<Self, Error> {
        if url.len() > MAX_QR_URL {
            return Err(Error::SizeLimit);
        }
        let payload = url
            .strip_prefix("https://app.withhide.dev/#pair=")
            .ok_or(Error::InvalidQr)?;
        let bytes = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| Error::InvalidQr)?;
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidQr)?;
        value.validate()?;
        Ok(value)
    }
}
