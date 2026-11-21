//! A machine's public key: its connection id (PRD D-29, D-52).

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The Ed25519 public key a machine connects as. It is written as 64 lower
/// hex characters everywhere Hide stores or sends it, never in a transport's
/// own spelling.
///
/// The order is the byte order: two machines that dial each other at once
/// keep the connection the smaller key dialed (D-62).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PeerKey([u8; 32]);

/// How many hex characters of a key name a guest's folder (D-77).
pub const SHORT_KEY_CHARS: usize = 16;

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidKey;

impl fmt::Display for InvalidKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a peer key is 64 lower hex characters")
    }
}

impl std::error::Error for InvalidKey {}

impl PeerKey {
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Reads the spelling [`Self::to_hex`] writes and no other: a folder name
    /// made from a key must have one spelling.
    pub fn from_hex(text: &str) -> Result<Self, InvalidKey> {
        let canonical = text.len() == 64
            && text
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        if !canonical {
            return Err(InvalidKey);
        }
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(text, &mut bytes).map_err(|_| InvalidKey)?;
        Ok(Self(bytes))
    }

    /// The first [`SHORT_KEY_CHARS`] hex characters: the name of this
    /// machine's folder on a host. The full key is in a file inside it.
    pub fn short(&self) -> String {
        let mut text = self.to_hex();
        text.truncate(SHORT_KEY_CHARS);
        text
    }
}

impl fmt::Display for PeerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for PeerKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PeerKey({})", self.short())
    }
}

impl Serialize for PeerKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for PeerKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_hex(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_has_one_spelling() {
        let key = PeerKey::from_bytes([0xab; 32]);
        assert_eq!(PeerKey::from_hex(&key.to_hex()), Ok(key));
        assert_eq!(key.short(), "abababababababab");
        assert_eq!(
            PeerKey::from_hex(&key.to_hex().to_uppercase()),
            Err(InvalidKey)
        );
        assert_eq!(PeerKey::from_hex("ab"), Err(InvalidKey));
    }
}
