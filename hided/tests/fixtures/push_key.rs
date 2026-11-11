use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};

/// A phone's push key pair, as the iOS app makes one per computer: the
/// private half stays here to decrypt what Apple would carry.
pub struct AppKey {
    private: ring::agreement::EphemeralPrivateKey,
    public: Vec<u8>,
    auth: [u8; 16],
}
impl AppKey {
    pub fn new() -> Self {
        let rng = ring::rand::SystemRandom::new();
        let private =
            ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
                .unwrap();
        let public = private.compute_public_key().unwrap().as_ref().to_vec();
        Self {
            private,
            public,
            auth: [7; 16],
        }
    }
    pub fn registration(&self, token: &str) -> Value {
        json!({
            "type": "apns_registration",
            "token": token,
            "environment": "sandbox",
            "key": {
                "p256dh": URL_SAFE_NO_PAD.encode(&self.public),
                "auth": URL_SAFE_NO_PAD.encode(self.auth),
            },
        })
    }
    /// RFC 8291 decryption of one aes128gcm body, as the phone's
    /// notification extension does it (`docs/RELAY_PROTOCOL.md`, The
    /// notification body).
    pub fn decrypt(self, body: &[u8]) -> Vec<u8> {
        let hmac = |key: &[u8], parts: &[&[u8]]| {
            let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
            let mut context = ring::hmac::Context::with_key(&key);
            for part in parts {
                context.update(part);
            }
            context.sign().as_ref().to_vec()
        };
        let (salt, rest) = body.split_at(16);
        assert_eq!(&rest[..4], 4096_u32.to_be_bytes(), "record size");
        assert_eq!(rest[4], 65, "the sender key is an uncompressed P-256 point");
        let sender = &rest[5..70];
        let ciphertext = &rest[70..];
        let peer = ring::agreement::UnparsedPublicKey::new(&ring::agreement::ECDH_P256, sender);
        let shared =
            ring::agreement::agree_ephemeral(self.private, &peer, |secret| secret.to_vec())
                .unwrap();
        let prk_key = hmac(&self.auth, &[&shared]);
        let ikm = hmac(&prk_key, &[b"WebPush: info\0", &self.public, sender, &[1]]);
        let prk = hmac(salt, &[&ikm]);
        let cek = hmac(&prk, &[b"Content-Encoding: aes128gcm\0", &[1]]);
        let nonce = hmac(&prk, &[b"Content-Encoding: nonce\0", &[1]]);
        let key = ring::aead::LessSafeKey::new(
            ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &cek[..16]).unwrap(),
        );
        let mut buffer = ciphertext.to_vec();
        let plain = key
            .open_in_place(
                ring::aead::Nonce::try_assume_unique_for_key(&nonce[..12]).unwrap(),
                ring::aead::Aad::empty(),
                &mut buffer,
            )
            .unwrap();
        // One record: the plaintext, then the last-record delimiter.
        assert_eq!(plain.last(), Some(&2));
        plain[..plain.len() - 1].to_vec()
    }
}
