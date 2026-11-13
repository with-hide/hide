//! Web Push to paired phones (PRD D-09, D-19, D-21, D-23).
//!
//! The daemon signs with its own VAPID key (RFC 8292, ES256) and encrypts
//! each message for the phone's subscription (RFC 8291, aes128gcm), with
//! `ring` for the curve, HMAC and AES-GCM and `ureq` over rustls for the
//! POST: both are already in the build, and no other TLS stack is added.
//!
//! What is sent: one notification per root that comes to wait on the
//! operator's answer (a question, an approval, a block, a raised ask or a
//! person's turn; a raised descendant reaches the phone as its lineage root),
//! never a finished turn or a View alone, and only once the root has stayed
//! waiting for `SETTLE` (`PhoneNotices`). It goes as data: the task as the
//! title, the state (`needs_you`), the row's status code and chosen
//! second-line sentence, and the project; never terminal content.
//! The sentence stays untranslated and is never logged. Status words belong to
//! the phone's language, so the phone page hands them to its service worker
//! and the worker composes the body (`web/public/m/sw.js`). A later
//! notice of the same agent replaces it (same tag). Agents the desktop
//! has since made Seen ride along as tags to close.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Value, json};

use hide_relay_protocol::admission::{self, FcmRequest, PushRequest};

use super::projection::{AgentKey, Projection};
use super::store::{ApnsRegistration, FcmRegistration, PushMode, PushSubscription};
#[cfg(test)]
use hide_core::agent_state::push::VANISHED_TTL;
pub use hide_core::agent_state::push::{Notice, NoticeState, Transitions};

const RECORD_SIZE: u32 = 4096;
// aes128gcm adds a 16-byte salt, 4-byte record size, 1-byte key length,
// 65-byte P-256 public key, 1-byte delimiter and 16-byte authentication tag.
const MAX_PLAINTEXT_SIZE: usize = 4096 - 86 - 1 - 16;
/// The same body carried by the relay to Apple's push service or to Google's
/// FCM: the ciphertext is at most 3,000 bytes there (`admission::MAX_CIPHERTEXT`),
/// which holds a plaintext of this size (`docs/RELAY_PROTOCOL.md`, The
/// notification body).
pub const MAX_APP_PLAINTEXT: usize = admission::MAX_CIPHERTEXT - 86 - 1 - 16;
const TTL_SECS: u64 = 60 * 60;
const JWT_LIFETIME_SECS: u64 = 12 * 60 * 60;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// The push services a subscription may point at. A paired phone chooses
/// the endpoint, so the daemon only ever posts to a known push service.
const PUSH_HOSTS: [&str; 4] = [
    "push.apple.com",
    "fcm.googleapis.com",
    "push.services.mozilla.com",
    "notify.windows.com",
];

fn host_allowed(host: &str) -> bool {
    PUSH_HOSTS
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

/// Whether the daemon may post to this endpoint: https to a known push
/// service; a debug build also accepts `http://127.0.0.1:<port>/...` for the
/// e2e fake push service.
pub fn endpoint_allowed(endpoint: &str) -> bool {
    if endpoint.len() > 2048 || endpoint.bytes().any(|b| b.is_ascii_control()) {
        return false;
    }
    // Parsed as the HTTP client will read it; any userinfo is refused
    // outright, because a client splits `host:port@other` differently from a
    // naive check and would connect to `other`.
    let Ok(uri) = endpoint.parse::<axum::http::Uri>() else {
        return false;
    };
    let Some(authority) = uri.authority() else {
        return false;
    };
    if authority.as_str().contains('@') {
        return false;
    }
    match uri.scheme_str() {
        Some("https") => {
            matches!(authority.port_u16(), None | Some(443)) && host_allowed(authority.host())
        }
        Some("http") if cfg!(debug_assertions) => {
            authority.host() == "127.0.0.1" && authority.port_u16().is_some()
        }
        _ => false,
    }
}

/// A subscription the phone sent, checked for shape before it is stored.
pub fn subscription_valid(push: &PushSubscription) -> bool {
    let key = URL_SAFE_NO_PAD.decode(push.p256dh.trim_end_matches('='));
    let auth = URL_SAFE_NO_PAD.decode(push.auth.trim_end_matches('='));
    endpoint_allowed(&push.endpoint)
        && key.is_ok_and(|key| key.len() == 65 && key[0] == 4)
        && auth.is_ok_and(|auth| auth.len() == 16)
}

/// The daemon's VAPID key pair.
pub struct Vapid {
    key: EcdsaKeyPair,
}

impl Vapid {
    /// A new key pair and its PKCS#8 bytes to store.
    pub fn generate() -> Result<(Self, Vec<u8>), String> {
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| "VAPID key generation failed".to_owned())?;
        let bytes = pkcs8.as_ref().to_vec();
        Ok((Self::from_pkcs8(&bytes)?, bytes))
    }

    pub fn from_pkcs8(bytes: &[u8]) -> Result<Self, String> {
        EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            bytes,
            &SystemRandom::new(),
        )
        .map(|key| Self { key })
        .map_err(|_| "the stored VAPID key is unreadable".to_owned())
    }

    /// The uncompressed public key, base64url: the phone's `applicationServerKey`.
    pub fn public_key(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.key.public_key().as_ref())
    }

    /// The `Authorization` header value for one push service origin.
    pub fn authorization(
        &self,
        audience: &str,
        subject: &str,
        now_secs: u64,
    ) -> Result<String, String> {
        let header = URL_SAFE_NO_PAD.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = URL_SAFE_NO_PAD.encode(
            json!({"aud": audience, "exp": now_secs + JWT_LIFETIME_SECS, "sub": subject})
                .to_string(),
        );
        let input = format!("{header}.{claims}");
        let signature = self
            .key
            .sign(&SystemRandom::new(), input.as_bytes())
            .map_err(|_| "VAPID signing failed".to_owned())?;
        Ok(format!(
            "vapid t={input}.{}, k={}",
            URL_SAFE_NO_PAD.encode(signature.as_ref()),
            self.public_key()
        ))
    }
}

fn hmac(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    let mut context = ring::hmac::Context::with_key(&key);
    for part in parts {
        context.update(part);
    }
    let mut out = [0_u8; 32];
    out.copy_from_slice(context.sign().as_ref());
    out
}

/// RFC 8291 section 3.4 once the ECDH secret is known: derives the key and
/// nonce and returns the whole aes128gcm body (header, then one record).
pub fn encrypt_with_secret(
    ecdh_secret: &[u8],
    auth_secret: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8; 16],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    if plaintext.len() > MAX_PLAINTEXT_SIZE {
        return Err("the message exceeds the 4 KiB Web Push body limit".to_owned());
    }
    let prk_key = hmac(auth_secret, &[ecdh_secret]);
    let ikm = hmac(&prk_key, &[b"WebPush: info\0", ua_public, as_public, &[1]]);
    let prk = hmac(salt, &[&ikm]);
    let cek = hmac(&prk, &[b"Content-Encoding: aes128gcm\0", &[1]]);
    let nonce = hmac(&prk, &[b"Content-Encoding: nonce\0", &[1]]);
    let key = ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &cek[..16])
        .map_err(|_| "content key rejected".to_owned())?;
    let key = ring::aead::LessSafeKey::new(key);
    let nonce = ring::aead::Nonce::try_assume_unique_for_key(&nonce[..12])
        .map_err(|_| "nonce rejected".to_owned())?;
    let mut record = plaintext.to_vec();
    // The last (and only) record's padding delimiter.
    record.push(2);
    if record.len() + 16 > RECORD_SIZE as usize {
        return Err("the message is longer than one record".to_owned());
    }
    key.seal_in_place_append_tag(nonce, ring::aead::Aad::empty(), &mut record)
        .map_err(|_| "encryption failed".to_owned())?;
    let mut body = Vec::with_capacity(16 + 4 + 1 + as_public.len() + record.len());
    body.extend_from_slice(salt);
    body.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    body.push(u8::try_from(as_public.len()).map_err(|_| "key id too long".to_owned())?);
    body.extend_from_slice(as_public);
    body.extend_from_slice(&record);
    Ok(body)
}

/// Encrypts one message for a phone's P-256 key and authentication secret
/// with a fresh sender key and salt: RFC 8291, whether the phone carries the
/// body over Web Push or through the relay to Apple.
pub fn encrypt_for(p256dh: &str, auth: &str, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let ua_public = URL_SAFE_NO_PAD
        .decode(p256dh.trim_end_matches('='))
        .map_err(|_| "the subscription key is not base64url".to_owned())?;
    if ua_public.len() != 65 || ua_public[0] != 4 {
        return Err("the subscription key is not a P-256 point".to_owned());
    }
    let auth = URL_SAFE_NO_PAD
        .decode(auth.trim_end_matches('='))
        .map_err(|_| "the subscription secret is not base64url".to_owned())?;
    if auth.len() != 16 {
        return Err("the subscription secret is not 16 bytes".to_owned());
    }
    let rng = SystemRandom::new();
    let private = ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
        .map_err(|_| "sender key generation failed".to_owned())?;
    let as_public = private
        .compute_public_key()
        .map_err(|_| "sender key generation failed".to_owned())?;
    let mut salt = [0_u8; 16];
    getrandom::getrandom(&mut salt).map_err(|error| error.to_string())?;
    let peer =
        ring::agreement::UnparsedPublicKey::new(&ring::agreement::ECDH_P256, ua_public.clone());
    let secret = ring::agreement::agree_ephemeral(private, &peer, |secret| secret.to_vec())
        .map_err(|_| "the subscription key is not a P-256 point".to_owned())?;
    encrypt_with_secret(
        &secret,
        &auth,
        &ua_public,
        as_public.as_ref(),
        &salt,
        plaintext,
    )
}

/// Encrypts one message for a Web Push subscription with a fresh sender key
/// and salt.
pub fn encrypt(push: &PushSubscription, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    encrypt_for(&push.p256dh, &push.auth, plaintext)
}

/// Apple's collapse header, which its network reads in plain text. It is an
/// HMAC prefix under the phone's own authentication secret over the agent
/// tag, so it is stable for one agent on one phone and says nothing: never an
/// agent key, a pane id or a title (`docs/RELAY_PROTOCOL.md`, The collapse
/// value). A later notice for the same agent replaces the earlier one.
pub fn collapse(auth: &[u8], tag: &str) -> String {
    URL_SAFE_NO_PAD.encode(&hmac(auth, &[b"hide-collapse\0", tag.as_bytes()])[..24])
}

/// The push request for an iOS app phone: the notice JSON encrypted for the
/// phone's key exactly as Web Push carries it, and the collapse value. The
/// request id is the link's to assign (`docs/RELAY_PROTOCOL.md`, The
/// notification body).
pub fn app_request(
    registration: &ApnsRegistration,
    payload: &Value,
) -> Result<PushRequest, &'static str> {
    let plaintext = app_plaintext(payload)?;
    let body = encrypt_for(&registration.p256dh, &registration.auth, &plaintext)
        .map_err(|_| "encrypt_failed")?;
    let auth = URL_SAFE_NO_PAD
        .decode(registration.auth.trim_end_matches('='))
        .map_err(|_| "encrypt_failed")?;
    let tag = payload.get("tag").and_then(Value::as_str).unwrap_or("");
    Ok(PushRequest {
        id: 0,
        environment: registration.environment,
        token: registration.token.clone(),
        collapse: collapse(&auth, tag),
        payload: URL_SAFE_NO_PAD.encode(body),
    })
}

/// The push request for an Android app phone: the same body `app_request`
/// builds, for the phone's key, and no collapse value: FCM has none, and the
/// app replaces an agent's earlier notification by the tag inside the body.
/// The request id is the link's to assign.
pub fn fcm_request(
    registration: &FcmRegistration,
    payload: &Value,
) -> Result<FcmRequest, &'static str> {
    let plaintext = app_plaintext(payload)?;
    let body = encrypt_for(&registration.p256dh, &registration.auth, &plaintext)
        .map_err(|_| "encrypt_failed")?;
    Ok(FcmRequest {
        id: 0,
        token: registration.token.clone(),
        payload: URL_SAFE_NO_PAD.encode(body),
    })
}

/// The notice JSON inside the relay's ciphertext bound. Tags to close are the
/// one part that can shrink, from the end: a notification the phone does not
/// close is better than none. Past that the notice does not fit.
fn app_plaintext(payload: &Value) -> Result<Vec<u8>, &'static str> {
    let mut payload = payload.clone();
    loop {
        let text = payload.to_string();
        if text.len() <= MAX_APP_PLAINTEXT {
            return Ok(text.into_bytes());
        }
        let Some(clear) = payload
            .get_mut("clear")
            .and_then(Value::as_array_mut)
            .filter(|clear| !clear.is_empty())
        else {
            return Err("payload_too_large");
        };
        clear.pop();
    }
}

/// `https://host[:port]` of an endpoint: the JWT audience.
fn origin_of(endpoint: &str) -> String {
    let (scheme, rest) = endpoint.split_once("://").unwrap_or(("https", endpoint));
    let host = rest.split('/').next().unwrap_or("");
    format!("{scheme}://{host}")
}

/// How a send ended.
#[derive(Debug, Eq, PartialEq)]
pub enum SendOutcome {
    Delivered,
    /// The push service says the subscription no longer exists (404, 410).
    Gone(u16),
    Failed(String),
}

/// A fully encrypted and signed Web Push, ready for its one HTTP attempt.
/// Separating preparation lets Mobile keep closes when no request can be made.
pub struct Prepared {
    endpoint: String,
    body: Vec<u8>,
    authorization: String,
    topic: String,
}

pub fn prepare(
    vapid: &Vapid,
    subject: &str,
    push: &PushSubscription,
    payload: &Value,
    now_secs: u64,
) -> Result<Prepared, String> {
    if !endpoint_allowed(&push.endpoint) {
        return Err("the endpoint is not a known push service".to_owned());
    }
    let body = encrypt(push, payload.to_string().as_bytes())?;
    let authorization = vapid.authorization(&origin_of(&push.endpoint), subject, now_secs)?;
    let topic = URL_SAFE_NO_PAD.encode(
        &ring::digest::digest(
            &ring::digest::SHA256,
            payload
                .get("tag")
                .and_then(Value::as_str)
                .unwrap_or("")
                .as_bytes(),
        )
        .as_ref()[..24],
    );
    Ok(Prepared {
        endpoint: push.endpoint.clone(),
        body,
        authorization,
        topic,
    })
}

impl Prepared {
    pub fn send(self) -> SendOutcome {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(HTTP_TIMEOUT))
            .http_status_as_error(false)
            // The allowlisted service answers itself; a redirect could send the
            // signed request anywhere.
            .max_redirects(0)
            .build()
            .into();
        let result = agent
            .post(&self.endpoint)
            .header("Authorization", &self.authorization)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .header("TTL", &TTL_SECS.to_string())
            .header("Urgency", "high")
            .header("Topic", &self.topic)
            .send(&self.body[..]);
        match result {
            Ok(response) => match response.status().as_u16() {
                200..=299 => SendOutcome::Delivered,
                status @ (404 | 410) => SendOutcome::Gone(status),
                status => SendOutcome::Failed(format!("the push service answered {status}")),
            },
            Err(error) => SendOutcome::Failed(error.to_string()),
        }
    }
}

/// The saved `app_closed` spelling is retained for compatibility; its policy
/// rests only during actual computer use with a desktop notice route enabled.
pub fn mode_allows(mode: PushMode, computer_in_use: bool) -> bool {
    mode == PushMode::Always || !computer_in_use
}

/// The first reason an operator notice cannot reach a phone, before sending.
pub fn notice_route(
    mode: PushMode,
    mobile_on: bool,
    computer_in_use: bool,
    subscriptions: usize,
) -> Result<(), &'static str> {
    if !mode_allows(mode, computer_in_use) {
        return Err("computer_in_use");
    }
    if !mobile_on {
        return Err("mobile_off");
    }
    if subscriptions == 0 {
        return Err("no_subscription");
    }
    Ok(())
}

/// How long a root must keep waiting on the operator before its phone notice
/// goes out. An agent that passes through a state on its way to a question
/// (a stop, then the question its label or hook reports; an approval, then
/// a question) settles within the core's notification bursts, well under a
/// second, so one notice of where it came to rest goes out instead of one per
/// step, and three seconds is still at once for a phone in a pocket.
pub const SETTLE: Duration = Duration::from_secs(3);

/// How many roots may be waiting out `SETTLE` at once. Past it the one due
/// first goes out at once, unsettled, so nothing is dropped.
pub const SETTLE_CAP: usize = 64;

/// The phones' notice state: the ledger of transitions every phone shares,
/// the roots that entered the operator's turn and are waiting out `SETTLE`
/// (`pending`), and the roots whose notice may stand on a phone (`standing`).
/// A root may be pending, standing or both. A root that leaves the operator's
/// turn while pending is dropped and sends nothing; one that leaves while
/// standing is closed once. Its SETTLE deadline never moves; an absent root
/// waits within the ledger's existing disappearance grace. Nothing pending
/// survives a restart: the
/// ledger seeds silently and every root then waiting is standing, so a
/// restart never sends a notice twice and still closes the ones it finds.
/// Computer use records new resting waits as standing without replacing an
/// existing pending root; ending rest never rebuilds a pending backlog.
#[derive(Default)]
pub struct PhoneNotices {
    ledger: Transitions,
    pending: BTreeMap<AgentKey, Pending>,
    standing: BTreeSet<AgentKey>,
    /// Whether the cap was crossed and not yet left, so a crossing is
    /// recorded once.
    capped: bool,
}

struct Pending {
    due: Instant,
    notice: Notice,
}

impl PhoneNotices {
    pub fn reset(&mut self) {
        self.ledger.reset();
        self.pending.clear();
        self.standing.clear();
        self.capped = false;
    }

    pub fn seeded(&self) -> bool {
        self.ledger.seeded()
    }

    /// One ledger pass. The pass that seeds it announces nothing and makes
    /// every root then waiting standing, whichever path seeds it.
    fn judge(&mut self, projection: &Projection, at: Instant) -> (Vec<Notice>, BTreeSet<AgentKey>) {
        let seeding = !self.ledger.seeded();
        let judged = self.ledger.observe_at(projection, at);
        if seeding {
            self.standing = self.ledger.waiting().cloned().collect();
        }
        judged
    }

    /// Judges a changed projection: the roots that entered the operator's
    /// turn start waiting out `SETTLE`, the pending ones that left it stop,
    /// and the standing ones that left it are closed. Returns what goes out
    /// now (only past the cap) and the agents whose notices to close.
    #[cfg(test)]
    pub fn observe(
        &mut self,
        projection: &Projection,
        at: Instant,
    ) -> (Vec<Notice>, BTreeSet<AgentKey>) {
        self.observe_with_rest(projection, at, false)
    }

    pub(super) fn observe_with_rest(
        &mut self,
        projection: &Projection,
        at: Instant,
        resting: bool,
    ) -> (Vec<Notice>, BTreeSet<AgentKey>) {
        let (announced, mut close) = self.judge(projection, at);
        let ledger = &self.ledger;
        // A root absent from the projection keeps its notice through the
        // ledger's disappearance grace, whose end the ledger reports in
        // `close` itself.
        close.extend(
            self.standing
                .iter()
                .filter(|key| ledger.present(key) && !ledger.waits_on_operator(key))
                .cloned(),
        );
        self.standing.retain(|key| !close.contains(key));
        self.pending.retain(|key, _| {
            !close.contains(key) && (!ledger.present(key) || ledger.waits_on_operator(key))
        });
        for notice in announced {
            // A finished turn, a View to look at or anything else the ledger
            // announces for the desktop is no phone notice.
            if !self.ledger.waits_on_operator(&notice.key) {
                continue;
            }
            // Resting creates no candidate and leaves any existing pending
            // root alone. Standing still owns its eventual clear.
            if resting {
                self.standing.insert(notice.key);
                continue;
            }
            match self.pending.get_mut(&notice.key) {
                Some(pending) => {
                    pending.notice = notice;
                }
                None => {
                    self.pending.insert(
                        notice.key.clone(),
                        Pending {
                            due: at + SETTLE,
                            notice,
                        },
                    );
                }
            }
        }
        let mut now = Vec::new();
        if self.pending.len() > SETTLE_CAP {
            if !self.capped {
                hide_core::diagnostic!(json!({
                    "component": "mobile_push", "kind": "push.settle_capped",
                    "pending": self.pending.len(), "cap": SETTLE_CAP,
                }));
            }
            self.capped = true;
            while self.pending.len() > SETTLE_CAP {
                let Some(first) = self
                    .pending
                    .iter()
                    .min_by_key(|(_, pending)| pending.due)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                now.extend(self.send(&first, projection));
            }
        } else if self.pending.len() < SETTLE_CAP {
            self.capped = false;
        }
        (now, close)
    }

    /// Seeds the ledger with what already waits, which is never announced.
    pub fn seed(&mut self, projection: &Projection, at: Instant) {
        self.judge(projection, at);
    }

    /// The notices whose moment is up at `at`, each worded from its root's
    /// row as it is now.
    pub fn due(&mut self, projection: &Projection, at: Instant) -> Vec<Notice> {
        let due: Vec<AgentKey> = self
            .pending
            .iter()
            .filter(|(key, pending)| self.ledger.present(key) && pending.due <= at)
            .map(|(key, _)| key.clone())
            .collect();
        due.iter()
            .filter_map(|key| self.send(key, projection))
            .collect()
    }

    /// When the next pending notice is due.
    pub fn next_due(&self) -> Option<Instant> {
        self.pending
            .iter()
            .filter(|(key, _)| self.ledger.present(key))
            .map(|(_, pending)| pending.due)
            .min()
    }

    fn send(&mut self, key: &AgentKey, projection: &Projection) -> Option<Notice> {
        let pending = self.pending.remove(key)?;
        let agent = projection.find(key)?;
        self.standing.insert(key.clone());
        self.ledger.waits_on_operator(key).then(|| {
            hide_core::agent_state::push::waiting_notice(
                key,
                agent,
                pending.notice.subject.as_ref(),
            )
        })
    }
}

/// Whether an iOS app phone's registration can be stored: a device token the
/// relay accepts, and the key and secret in the same shape a Web Push
/// subscription carries.
pub fn registration_valid(registration: &ApnsRegistration) -> bool {
    admission::token_valid(&registration.token)
        && encrypt_for(&registration.p256dh, &registration.auth, &[]).is_ok()
}

/// Whether an Android app phone's registration can be stored: a token the
/// relay accepts and the key in the shape `registration_valid` checks.
pub fn fcm_registration_valid(registration: &FcmRegistration) -> bool {
    admission::fcm_token_valid(&registration.token)
        && encrypt_for(&registration.p256dh, &registration.auth, &[]).is_ok()
}

/// The JSON a phone's service worker receives.
pub fn payload(notice: &Notice, clear: &BTreeSet<AgentKey>) -> Value {
    json!({
        "title": notice.title,
        "state": notice.state.as_str(),
        "status_code": notice.status_code,
        "detail": notice.detail,
        "subject": notice.subject,
        "place": notice.place,
        "tag": notice.key.tag(),
        "device_id": notice.key.device_id,
        "pane_id": notice.key.pane_id,
        "clear": clear.iter().map(AgentKey::tag).collect::<Vec<_>>(),
    })
}

/// What a notice asks of the person, for the app's notification to offer the action that fits (a Reply to a
/// question, an Approve to an approval) and for that action to be checked later against the same ask: `turn` is
/// the agent's last change of state, which `input` compares with the agent's now. Only a notice that sends the
/// person to an agent waiting on a question or an approval carries one.
pub fn with_ask(payload: &mut Value, agent: &super::projection::PhoneAgent) {
    if payload.get("state").and_then(Value::as_str) != Some("needs_you") {
        return;
    }
    let (Some(at), "question" | "approval") = (agent.changed_at_unix_ms, agent.demand.as_str())
    else {
        return;
    };
    payload["ask"] = json!({"kind": agent.demand, "turn": at.to_string()});
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobile::projection::project;

    fn b64(text: &str) -> Vec<u8> {
        URL_SAFE_NO_PAD.decode(text).unwrap()
    }

    /// RFC 8291 Appendix A: the example's inputs produce its header and
    /// ciphertext byte for byte.
    #[test]
    fn rfc8291_appendix_a_vector() {
        let as_public = b64(
            "BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
        );
        let ua_public = b64(
            "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4",
        );
        let secret = b64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs");
        let auth = b64("BTBZMqHH6r4Tts7J_aSIgg");
        let salt: [u8; 16] = b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap();
        let plaintext = b64("V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24");
        let body =
            encrypt_with_secret(&secret, &auth, &ua_public, &as_public, &salt, &plaintext).unwrap();
        let expected_header = b64(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8",
        );
        let expected_cipher =
            b64("8pfeW0KbunFT06SuDKoJH9Ql87S1QUrdirN6GcG7sFz1y1sqLgVi1VhjVkHsUoEsbI_0LpXMuGvnzQ");
        assert_eq!(&body[..86], &expected_header[..]);
        assert_eq!(&body[86..], &expected_cipher[..]);
        // The intermediate keys the appendix lists.
        let prk_key = hmac(&auth, &[&secret]);
        assert_eq!(
            URL_SAFE_NO_PAD.encode(prk_key),
            "Snr3JMxaHVDXHWJn5wdC52WjpCtd2EIEGBykDcZW32k"
        );
        let ikm = hmac(
            &prk_key,
            &[b"WebPush: info\0", &ua_public, &as_public, &[1]],
        );
        assert_eq!(
            URL_SAFE_NO_PAD.encode(ikm),
            "S4lYMb_L0FxCeq0WhDx813KgSYqU26kOyzWUdsXYyrg"
        );
    }

    /// The iOS app's body is the same RFC 8291 bytes, base64url: Appendix A's
    /// inputs give the `hide` value `docs/RELAY_PROTOCOL.md` lists, which the
    /// relay's own test wraps in Apple's body.
    #[test]
    fn the_app_body_vector_is_appendix_a_in_base64url() {
        let body = encrypt_with_secret(
            &b64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs"),
            &b64("BTBZMqHH6r4Tts7J_aSIgg"),
            &b64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"),
            &b64("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"),
            &b64("DGv6ra1nlYgDCS1FRnbzlw").try_into().unwrap(),
            b"When I grow up, I want to be a watermelon",
        )
        .unwrap();
        assert_eq!(URL_SAFE_NO_PAD.encode(body), APP_BODY_VECTOR);
    }

    /// The `hide` value of the documented vector (`docs/RELAY_PROTOCOL.md`).
    const APP_BODY_VECTOR: &str = "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIgDll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A_yl95bQpu6cVPTpK4Mqgkf1CXztLVBSt2Ks3oZwbuwXPXLWyouBWLVWGNWQexSgSxsj_Qulcy4a-fN";

    /// Apple reads the collapse value in plain text: stable for one agent on
    /// one phone, different across agents and phones, and never the tag.
    #[test]
    fn the_collapse_value_is_opaque_and_stable_per_phone_and_agent() {
        let tag = "device-1:w1:p1";
        let one = collapse(&[1; 16], tag);
        assert_eq!(one, collapse(&[1; 16], tag));
        assert_ne!(one, collapse(&[2; 16], tag));
        assert_ne!(one, collapse(&[1; 16], "device-1:w1:p2"));
        assert_eq!(one.len(), 32);
        assert!(one.len() <= admission::MAX_COLLAPSE);
        assert!(!one.contains("w1") && !one.contains("device"));
    }

    /// Past the bound only the tags to close shrink, from the end; a notice
    /// that still does not fit is refused rather than cut.
    #[test]
    fn an_app_plaintext_sheds_tags_to_close_and_never_its_notice() {
        let tags: Vec<String> = (0..400).map(|n| format!("device-1:w1:p{n}")).collect();
        let payload = json!({"title": "task", "tag": "device-1:w1:p1", "clear": tags});
        let plain: Value = serde_json::from_slice(&app_plaintext(&payload).unwrap()).unwrap();
        assert!(plain.to_string().len() <= MAX_APP_PLAINTEXT);
        assert_eq!(plain["title"], "task");
        let kept = plain["clear"].as_array().unwrap();
        assert!(!kept.is_empty() && kept.len() < 400);
        assert_eq!(kept[0], "device-1:w1:p0");
        let huge = json!({"title": "x".repeat(MAX_APP_PLAINTEXT), "clear": []});
        assert_eq!(app_plaintext(&huge), Err("payload_too_large"));
    }

    /// A message encrypted for a subscription decrypts with the phone's key.
    #[test]
    fn a_message_round_trips_through_ecdh() {
        let rng = SystemRandom::new();
        let phone =
            ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
                .unwrap();
        let phone_public = phone.compute_public_key().unwrap();
        let auth = [7_u8; 16];
        let push = PushSubscription {
            endpoint: "https://web.push.apple.com/abc".into(),
            p256dh: URL_SAFE_NO_PAD.encode(phone_public.as_ref()),
            auth: URL_SAFE_NO_PAD.encode(auth),
        };
        assert!(subscription_valid(&push));
        let body = encrypt(&push, b"{\"title\":\"t\"}").unwrap();
        let salt: [u8; 16] = body[..16].try_into().unwrap();
        let as_public = body[21..86].to_vec();
        let peer =
            ring::agreement::UnparsedPublicKey::new(&ring::agreement::ECDH_P256, as_public.clone());
        let secret =
            ring::agreement::agree_ephemeral(phone, &peer, |secret| secret.to_vec()).unwrap();
        let prk_key = hmac(&auth, &[&secret]);
        let ikm = hmac(
            &prk_key,
            &[b"WebPush: info\0", phone_public.as_ref(), &as_public, &[1]],
        );
        let prk = hmac(&salt, &[&ikm]);
        let cek = hmac(&prk, &[b"Content-Encoding: aes128gcm\0", &[1]]);
        let nonce = hmac(&prk, &[b"Content-Encoding: nonce\0", &[1]]);
        let key = ring::aead::LessSafeKey::new(
            ring::aead::UnboundKey::new(&ring::aead::AES_128_GCM, &cek[..16]).unwrap(),
        );
        let mut record = body[86..].to_vec();
        let plain = key
            .open_in_place(
                ring::aead::Nonce::try_assume_unique_for_key(&nonce[..12]).unwrap(),
                ring::aead::Aad::empty(),
                &mut record,
            )
            .unwrap();
        assert_eq!(plain, b"{\"title\":\"t\"}\x02");
    }

    #[test]
    fn the_vapid_token_verifies_with_the_public_key() {
        let (vapid, pkcs8) = Vapid::generate().unwrap();
        let reloaded = Vapid::from_pkcs8(&pkcs8).unwrap();
        assert_eq!(vapid.public_key(), reloaded.public_key());
        let header = reloaded
            .authorization("https://web.push.apple.com", "mailto:hide@localhost", 1_000)
            .unwrap();
        let token = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split(", k=")
            .next()
            .unwrap();
        let (input, signature) = token.rsplit_once('.').unwrap();
        let claims: Value = serde_json::from_slice(&b64(input.split('.').nth(1).unwrap())).unwrap();
        assert_eq!(claims["aud"], "https://web.push.apple.com");
        assert_eq!(claims["exp"], 1_000 + JWT_LIFETIME_SECS);
        let public = b64(&reloaded.public_key());
        ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, public)
            .verify(input.as_bytes(), &b64(signature))
            .expect("signature verifies");
    }

    #[test]
    fn only_known_push_services_are_reachable() {
        assert!(endpoint_allowed("https://web.push.apple.com/QGuW"));
        assert!(endpoint_allowed("https://fcm.googleapis.com/fcm/send/x"));
        assert!(endpoint_allowed(
            "https://updates.push.services.mozilla.com/wpush/v2/x"
        ));
        assert!(!endpoint_allowed("https://example.com/push"));
        assert!(!endpoint_allowed("https://push.apple.com.evil.test/x"));
        assert!(!endpoint_allowed("https://user@web.push.apple.com/x"));
        assert!(!endpoint_allowed(
            "https://web.push.apple.com:1@evil.test/x"
        ));
        assert!(!endpoint_allowed("https://fcm.googleapis.com:8443/x"));
        assert!(!endpoint_allowed("http://web.push.apple.com/x"));
        assert!(!endpoint_allowed("http://10.0.0.1:80/x"));
        // The e2e fake service, in this debug test build only.
        assert_eq!(
            endpoint_allowed("http://127.0.0.1:4000/push/1"),
            cfg!(debug_assertions)
        );
    }

    fn rest(agents: Value) -> Value {
        json!({"navigator": {"agents": agents, "workspaces": [
            {"label": "hide", "is_git": true, "checkouts": [{"label": "main", "branch": "main", "tabs": [{"panes": [{"id": "w1:p1"}, {"id": "w1:p2"}]}]}]}
        ]}})
    }

    fn row(pane: &str, group: &str, demand: &str, parent: Option<&str>) -> Value {
        json!({"pane_id": pane, "group": group, "demand": demand, "identity_label": format!("task {pane}"),
               "agent_kind": "claude", "lineage_parent_pane_id": parent})
    }

    #[test]
    fn roots_announce_needs_you_and_done_once_each() {
        let mut transitions = Transitions::default();
        let seed = project(
            &rest(json!([row("w1:p1", "needs_you", "question", None)])),
            "local",
        );
        assert_eq!(
            transitions.observe(&seed),
            (Vec::new(), BTreeSet::new()),
            "the first look only seeds"
        );
        let working = project(
            &rest(json!([row("w1:p1", "working", "none", None)])),
            "local",
        );
        assert!(transitions.observe(&working).0.is_empty());
        let asking = project(
            &rest(json!([row("w1:p1", "needs_you", "approval", None)])),
            "local",
        );
        let (notices, _) = transitions.observe(&asking);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].title, "task w1:p1");
        assert_eq!(notices[0].state, NoticeState::NeedsYou);
        assert_eq!(notices[0].place, "hide");
        assert!(
            transitions.observe(&asking).0.is_empty(),
            "no repeat without a transition"
        );
        let done = project(&rest(json!([row("w1:p1", "done", "none", None)])), "local");
        let finished = transitions.observe(&done).0;
        assert_eq!(finished[0].state, NoticeState::Done);
        assert_eq!(finished[0].place, "hide");
        let seen = project(&rest(json!([row("w1:p1", "seen", "none", None)])), "local");
        let (notices, cleared) = transitions.observe(&seen);
        assert!(notices.is_empty());
        assert_eq!(
            cleared
                .into_iter()
                .map(|key| key.pane_id)
                .collect::<Vec<_>>(),
            ["w1:p1"]
        );
    }

    /// PRD ask-human B5, B13, B17, B23: a person's turn announces once per
    /// declaration, a new one again while the row already stands in Needs
    /// You in its own words, a raised one in the root's notice with its
    /// subject, and the first look after a restart only seeds.
    #[test]
    fn each_persons_turn_is_announced_once_with_what_and_where() {
        let turn = |id: &str| json!({"cause": "human_turn", "id": id, "kind": "auth", "place": "github.com/login", "device": "mini"});
        let root = |raised: &[&str]| {
            let mut row = row("w1:p1", "needs_you", "none", None);
            row["raised"] = json!(
                raised
                    .iter()
                    .map(|id| json!({"subject": turn(id)}))
                    .collect::<Vec<_>>()
            );
            row["human_turn"] = json!({"id": raised[0], "own": false});
            project(&rest(json!([row])), "local")
        };
        let mut transitions = Transitions::default();
        assert!(
            transitions.observe(&root(&["agent-3.1"])).0.is_empty(),
            "a restart only seeds"
        );
        assert!(transitions.observe(&root(&["agent-3.1"])).0.is_empty());
        let (notices, _) = transitions.observe(&root(&["agent-3.1", "agent-4.2"]));
        assert_eq!(notices.len(), 1, "a second declaration in the lineage");
        let notice = &notices[0];
        assert_eq!(notice.state, NoticeState::NeedsYou);
        assert_eq!(
            serde_json::to_value(&notice.subject).unwrap(),
            turn("agent-4.2"),
            "the new declaration says itself, not the lead"
        );
        assert!(
            transitions.observe(&root(&["agent-4.2"])).0.is_empty(),
            "one settled"
        );
        let (notices, _) = transitions.observe(&root(&["agent-5.3"]));
        assert_eq!(notices.len(), 1, "a new declaration after a settle");
    }

    /// A View to look at notifies like any Needs You entry
    /// (status-model.md, A View to look at): once per news, as `볼 것` with the newest
    /// View, and taken back when it is seen. A finished turn that has one is
    /// that one notice, and the row's own question keeps its own.
    #[test]
    fn a_view_to_look_at_is_one_notice_per_news_and_yields_to_the_rows_own_ask() {
        let at = |group: &str, demand: &str, own: Option<&str>, views: &[&str]| {
            let mut agent = row("w1:p1", group, demand, None);
            agent["status_code"] = json!(if group == "done" { "done" } else { "idle" });
            if let Some(own) = own {
                agent["look"] = json!({"group": own, "count": views.len(), "view_id": "d1",
                                       "label": views[0]});
                agent["mark"] = json!("look");
            }
            project(&rest(json!([agent])), "local")
        };
        let mut transitions = Transitions::default();
        transitions.observe(&at("working", "none", None, &[]));

        // The View opens while the agent works: one notice, the eye's word
        // and the View's name; the row's sentence and subject stay out.
        let (notices, _) =
            transitions.observe(&at("needs_you", "none", Some("working"), &["report.html"]));
        assert_eq!(
            notices
                .iter()
                .map(|n| (n.state, n.status_code.as_deref(), n.detail.as_deref()))
                .collect::<Vec<_>>(),
            [(NoticeState::NeedsYou, Some("look"), Some("report.html"))]
        );
        assert_eq!(notices[0].place, "hide");
        // A second open while it is still unseen, and the turn finishing
        // under it, say nothing new.
        let (notices, cleared) = transitions.observe(&at(
            "needs_you",
            "none",
            Some("working"),
            &["b.html", "report.html"],
        ));
        assert!(notices.is_empty() && cleared.is_empty());
        let (notices, _) = transitions.observe(&at("needs_you", "none", Some("done"), &["b.html"]));
        assert!(notices.is_empty(), "the finished turn is no Done beside it");
        // Seen: the row goes back to its own Done with no notice, and the
        // View's notice is taken back.
        let (notices, cleared) = transitions.observe(&at("done", "none", None, &[]));
        assert!(notices.is_empty());
        assert_eq!(cleared.len(), 1);
        // An open again of a View already seen is news again.
        let (notices, _) =
            transitions.observe(&at("needs_you", "none", Some("done"), &["report.html"]));
        assert_eq!(notices[0].status_code.as_deref(), Some("look"));

        // A turn that finishes with a View open is one notice, the View's.
        let mut transitions = Transitions::default();
        transitions.observe(&at("working", "none", None, &[]));
        let (notices, _) =
            transitions.observe(&at("needs_you", "none", Some("done"), &["report.html"]));
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].status_code.as_deref(), Some("look"));

        // A question on the row keeps its own notice, with the View or
        // without, and its answer leaves the row in Needs You for the View
        // with nothing new to say.
        let mut transitions = Transitions::default();
        transitions.observe(&at("working", "none", None, &[]));
        let (notices, _) = transitions.observe(&at(
            "needs_you",
            "question",
            Some("needs_you"),
            &["report.html"],
        ));
        assert_eq!(notices[0].status_code.as_deref(), Some("idle"));
        assert_eq!(notices[0].state, NoticeState::NeedsYou);
        let (notices, cleared) =
            transitions.observe(&at("needs_you", "none", Some("working"), &["report.html"]));
        assert!(notices.is_empty() && cleared.is_empty());
        // A question asked while the View waits is announced as a question.
        let (notices, _) = transitions.observe(&at(
            "needs_you",
            "question",
            Some("needs_you"),
            &["report.html"],
        ));
        assert_eq!(notices.len(), 1);
        assert_ne!(notices[0].status_code.as_deref(), Some("look"));
    }

    /// The worker receives the row's sentence and status unchanged, not a
    /// translated body. An old worker ignores the additive fields.
    #[test]
    fn the_payload_carries_the_rows_sentence_and_status_unchanged() {
        let notice = Notice {
            key: AgentKey {
                device_id: "mini".into(),
                pane_id: "w2:p1".into(),
            },
            title: "fix the build".into(),
            state: NoticeState::Done,
            status_code: Some("done".into()),
            detail: Some("빌드 수정 완료".into()),
            subject: None,
            place: "contong".into(),
        };
        let clear = BTreeSet::from([AgentKey {
            device_id: "local".into(),
            pane_id: "w1:p1".into(),
        }]);
        assert_eq!(
            payload(&notice, &clear),
            json!({
                "title": "fix the build",
                "state": "done",
                "status_code": "done",
                "detail": "빌드 수정 완료",
                "subject": null,
                "place": "contong",
                "tag": "mini|w2:p1",
                "device_id": "mini",
                "pane_id": "w2:p1",
                "clear": ["local|w1:p1"],
            })
        );
        // A question the core asks for the agent goes as what it is; the
        // phone words it (hide-plan B14).
        let review = Notice {
            detail: None,
            subject: serde_json::from_value(json!({"cause": "plan_review", "steps": 5})).unwrap(),
            ..notice.clone()
        };
        assert_eq!(
            payload(&review, &clear)["subject"],
            json!({"cause": "plan_review", "steps": 5})
        );
        let asking = Notice {
            state: NoticeState::NeedsYou,
            place: String::new(),
            ..notice
        };
        let payload = payload(&asking, &BTreeSet::new());
        assert_eq!(payload["state"], "needs_you");
        assert_eq!(payload["place"], "");
        assert!(payload.get("body").is_none());
    }

    /// A notice asks an action of the person only while its agent waits on a question or an approval, and carries the
    /// state change the action is later checked against.
    #[test]
    fn a_notice_carries_the_ask_its_action_is_checked_against_only_for_a_question_or_an_approval() {
        let agent = |demand: &str, changed_at: Option<u64>| {
            let row = json!({
                "pane_id": "w1:p1", "group": "needs_you", "mark": "turn", "identity_label": "t",
                "agent_kind": "claude", "status_code": demand, "demand": demand,
                "changed_at_unix_ms": changed_at,
            });
            project(&json!({"navigator": {"agents": [row]}}), "local")
                .agents()
                .next()
                .cloned()
                .unwrap()
        };
        let ask = |state: &str, demand: &str, at: Option<u64>| {
            let mut payload = json!({"state": state});
            with_ask(&mut payload, &agent(demand, at));
            payload.get("ask").cloned()
        };
        assert_eq!(
            ask("needs_you", "question", Some(5)),
            Some(json!({"kind": "question", "turn": "5"}))
        );
        assert_eq!(
            ask("needs_you", "approval", Some(6)),
            Some(json!({"kind": "approval", "turn": "6"}))
        );
        assert_eq!(
            ask("needs_you", "error", Some(5)),
            None,
            "no action for any other demand"
        );
        assert_eq!(
            ask("needs_you", "question", None),
            None,
            "no turn to check against, no action"
        );
        assert_eq!(
            ask("done", "question", Some(5)),
            None,
            "a finished task offers none"
        );
    }

    /// The same public transition boundary used by phone and desktop carries
    /// the root's exact detail even when the compact phone row hides it.
    #[test]
    fn a_raised_roots_notice_keeps_its_detail_without_a_compact_line() {
        let mut transitions = Transitions::default();
        transitions.observe(&project(
            &rest(json!([row("w1:p1", "working", "none", None)])),
            "local",
        ));
        let mut root = row("w1:p1", "needs_you", "none", None);
        root["detail"] = json!("자식의 질문에 답해 주세요");
        root["status_code"] = json!("idle");
        root["raised"] = json!([{"human_notice": false}]);
        let projection = project(&rest(json!([root])), "local");
        assert!(projection.agents().next().unwrap().line.is_none());
        let notices = transitions.observe(&projection).0;
        let data = payload(&notices[0], &BTreeSet::new());
        assert_eq!(data["detail"], "자식의 질문에 답해 주세요");
        assert_eq!(data["status_code"], "idle");
        assert_eq!(data["pane_id"], "w1:p1");
    }

    /// RFC 8291 overhead belongs inside the push service's 4096-byte cap,
    /// not just inside its record. A maximal payload fits; one more byte is
    /// a reported failure, without logging or truncating its sentence.
    #[test]
    fn the_encrypted_push_body_is_at_most_four_kibibytes() {
        let notice = Notice {
            key: AgentKey {
                device_id: "mini".into(),
                pane_id: "w2:p1".into(),
            },
            title: "🦀".repeat(80),
            state: NoticeState::Done,
            status_code: Some("done".into()),
            detail: Some("🦀".repeat(80)),
            subject: None,
            place: String::new(),
        };
        let mut data = payload(&notice, &BTreeSet::new());
        let remaining = 3993 - data.to_string().len();
        data["place"] = json!("p".repeat(remaining));
        let mut plaintext = data.to_string().into_bytes();
        assert_eq!(plaintext.len(), 3993);
        let encrypted =
            encrypt_with_secret(&[0; 32], &[0; 16], &[0; 65], &[0; 65], &[0; 16], &plaintext)
                .unwrap();
        assert_eq!(encrypted.len(), 4096);
        plaintext.push(b' ');
        assert!(
            encrypt_with_secret(&[0; 32], &[0; 16], &[0; 65], &[0; 65], &[0; 16], &plaintext)
                .is_err()
        );
    }

    #[test]
    fn an_ordinary_descendant_question_never_announces_its_root() {
        let mut transitions = Transitions::default();
        let quiet = project(
            &rest(json!([
                row("w1:p1", "working", "none", None),
                row("w1:p2", "working", "none", Some("w1:p1")),
            ])),
            "local",
        );
        transitions.observe(&quiet);
        let asking = project(
            &rest(json!([
                row("w1:p1", "working", "none", None),
                row("w1:p2", "working", "question", Some("w1:p1")),
            ])),
            "local",
        );
        let (notices, _) = transitions.observe(&asking);
        assert!(notices.is_empty());
    }

    /// needs-you-cleanup B12, B14: a question announces once; read, it stays
    /// listed in 내 차례 and its notice closes, and neither ☆ nor taking the
    /// ★ off announces it again.
    #[test]
    fn a_read_question_listed_in_needs_you_never_announces_again() {
        let mut transitions = Transitions::default();
        transitions.observe(&project(
            &rest(json!([row("w1:p1", "working", "none", None)])),
            "local",
        ));
        let (notices, _) = transitions.observe(&project(
            &rest(json!([row("w1:p1", "needs_you", "question", None)])),
            "local",
        ));
        assert_eq!(notices.len(), 1);
        let read = |starred: bool| {
            let mut row = row("w1:p1", "seen", "question", None);
            row["revisit"] = json!({"starred": starred, "read_ask": true, "since_unix_ms": 5});
            project(&rest(json!([row])), "local")
        };
        let listed = read(false);
        let agent = listed.agents().next().unwrap();
        assert_eq!((agent.group.as_str(), agent.read), ("needs_you", true));
        let (notices, cleared) = transitions.observe(&listed);
        assert!(notices.is_empty());
        assert_eq!(cleared.len(), 1, "reading it closes its notice");
        assert!(transitions.observe(&read(true)).0.is_empty(), "☆");
        assert!(
            transitions.observe(&read(false)).0.is_empty(),
            "★ taken off"
        );
    }

    /// B5: a block enters Needs You like a question, notifies once, and its
    /// notification closes once the operator has read it.
    #[test]
    fn a_blocked_root_notifies_once_and_closes_when_read() {
        let mut transitions = Transitions::default();
        transitions.observe(&project(
            &rest(json!([row("w1:p1", "working", "none", None)])),
            "local",
        ));
        let blocked = project(
            &rest(json!([row("w1:p1", "needs_you", "error", None)])),
            "local",
        );
        let (notices, _) = transitions.observe(&blocked);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].state, NoticeState::NeedsYou);
        assert!(transitions.observe(&blocked).0.is_empty(), "not twice");
        let read = project(&rest(json!([row("w1:p1", "seen", "error", None)])), "local");
        let (notices, cleared) = transitions.observe(&read);
        assert!(notices.is_empty());
        assert_eq!(cleared.len(), 1);
    }

    /// PRD hide-ai-work-home B4, B10: a row its home owns is on the phone only
    /// while the operator is called out, notifies once when it enters Needs
    /// You, and a change of the home's turn inside Needs You is no new notice.
    #[test]
    fn a_home_owned_row_notifies_once_on_entering_needs_you_and_not_on_a_turn_change() {
        let owned = |group: &str, turn: Value| {
            let mut row = row("w1:p1", group, "none", None);
            row["hide_ai"] = json!({"home": "memory", "turn": turn});
            row
        };
        let observe = |transitions: &mut Transitions, row: Value| {
            transitions
                .observe(&project(&rest(json!([row])), "local"))
                .0
        };
        let phone = |row: Value| project(&rest(json!([row])), "local").agents().count();
        assert_eq!(phone(owned("seen", Value::Null)), 0, "not on the phone");
        assert_eq!(
            phone(owned(
                "needs_you",
                json!({"kind": "pr_review", "pr_number": 924})
            )),
            1,
            "on the phone once called out"
        );
        let mut transitions = Transitions::default();
        let review = json!({"kind": "pr_review", "pr_number": 924});
        let failed = json!({"kind": "ci_failed", "pr_number": 924});
        assert!(
            observe(&mut transitions, owned("seen", Value::Null)).is_empty(),
            "the first look only seeds"
        );
        assert!(
            observe(&mut transitions, owned("seen", Value::Null)).is_empty(),
            "progress alone calls no one"
        );
        let entered = observe(&mut transitions, owned("needs_you", review.clone()));
        assert_eq!(entered.len(), 1);
        assert_eq!(entered[0].state, NoticeState::NeedsYou);
        assert!(
            observe(&mut transitions, owned("needs_you", review)).is_empty(),
            "the same turn again"
        );
        assert!(
            observe(&mut transitions, owned("needs_you", failed)).is_empty(),
            "the turn word changing inside Needs You"
        );
    }

    /// B8, B5: an ordinary delegated child's block is the parent's to handle,
    /// so the root is not announced; only a core escalation cause raises it.
    #[test]
    fn a_delegated_childs_block_never_announces_its_root() {
        let mut transitions = Transitions::default();
        transitions.observe(&project(
            &rest(json!([
                row("w1:p1", "working", "none", None),
                row("w1:p2", "working", "none", Some("w1:p1")),
            ])),
            "local",
        ));
        let blocked = project(
            &rest(json!([
                row("w1:p1", "working", "none", None),
                row("w1:p2", "working", "error", Some("w1:p1")),
            ])),
            "local",
        );
        assert!(transitions.observe(&blocked).0.is_empty());
    }

    #[test]
    fn read_root_and_child_questions_never_reannounce_the_root() {
        let mut transitions = Transitions::default();
        transitions.observe(&project(
            &rest(json!([row("w1:p1", "needs_you", "question", None),])),
            "local",
        ));
        let read = project(
            &rest(json!([row("w1:p1", "seen", "question", None),])),
            "local",
        );
        let (notices, cleared) = transitions.observe(&read);
        assert!(notices.is_empty());
        assert_eq!(
            cleared
                .iter()
                .map(|key| key.pane_id.as_str())
                .collect::<Vec<_>>(),
            ["w1:p1"]
        );

        let child = project(
            &rest(json!([
                row("w1:p1", "seen", "question", None),
                row("w1:p2", "seen", "question", Some("w1:p1")),
            ])),
            "local",
        );
        let (notices, cleared) = transitions.observe(&child);
        assert!(cleared.is_empty());
        assert!(notices.is_empty());
        assert!(transitions.observe(&child).0.is_empty());
    }

    // agent-hierarchy-screens B9: a raised descendant notifies through its
    // lineage root, once, and never on its own row.
    #[test]
    fn a_raised_descendant_notifies_its_root_once_and_clears_with_the_raise() {
        for (cause, human_notice) in [
            ("draft", false),
            ("bell_exhausted", false),
            ("child_blocked", false),
            ("input_unconfirmed", false),
            ("undelivered", true),
            ("observer_unconfirmed", true),
        ] {
            let mut transitions = Transitions::default();
            // A quiet root, so clearing the raise returns it to Seen.
            let parent = row("w1:p1", "seen", "none", None);
            let child = row("w1:p2", "seen", "question", Some("w1:p1"));
            let quiet = project(&rest(json!([parent.clone(), child.clone()])), "local");
            transitions.observe(&quiet);
            let mut root = parent.clone();
            root["group"] = json!("needs_you");
            root["raised"] = json!([{"verb": "answer", "human_notice": human_notice}]);
            let mut escalated = child.clone();
            escalated["escalation"] = json!({"cause": cause, "human_notice": human_notice});
            let raised = project(&rest(json!([root, escalated])), "local");
            let (notices, _) = transitions.observe(&raised);
            if human_notice {
                assert!(notices.is_empty(), "{cause} already has a delivery notice");
            } else {
                assert_eq!(notices.len(), 1, "{cause}");
                assert_eq!(notices[0].key.pane_id, "w1:p1", "the root, not the child");
                assert_eq!(notices[0].state, NoticeState::NeedsYou);
            }
            assert!(transitions.observe(&raised).0.is_empty());
            let (notices, cleared) = transitions.observe(&quiet);
            assert!(notices.is_empty());
            assert_eq!(cleared.len(), usize::from(!human_notice), "{cause}");
            if !human_notice {
                assert_eq!(cleared.first().unwrap().pane_id, "w1:p1");
                assert_eq!(
                    transitions.observe(&raised).0.len(),
                    1,
                    "a new raise is a new transition"
                );
            }
        }
    }

    #[test]
    fn an_agent_that_comes_back_in_the_same_state_is_not_announced_again() {
        let mut transitions = Transitions::default();
        let start = std::time::Instant::now();
        let asking = project(
            &rest(json!([row("w1:p1", "needs_you", "question", None)])),
            "local",
        );
        let empty = project(&rest(json!([])), "local");
        transitions.observe_at(&asking, start);
        assert_eq!(
            transitions.observe_at(&empty, start),
            (Vec::new(), BTreeSet::new()),
            "a brief absence closes nothing"
        );
        assert!(
            transitions.observe_at(&asking, start).0.is_empty(),
            "the same request after a reconnect is no new notice"
        );
        transitions.observe_at(&empty, start);
        let (_, cleared) = transitions.observe_at(&empty, start + VANISHED_TTL);
        assert_eq!(
            cleared
                .into_iter()
                .map(|key| key.pane_id)
                .collect::<Vec<_>>(),
            ["w1:p1"],
            "gone for good, its notification closes"
        );
    }

    /// One root row in `group` with `demand`, its status code and sentence.
    fn rested(group: &str, demand: &str, status: &str, detail: &str) -> Projection {
        let mut agent = row("w1:p1", group, demand, None);
        agent["status_code"] = json!(status);
        agent["detail"] = json!(detail);
        project(&rest(json!([agent])), "local")
    }

    fn said(notices: &[Notice]) -> Vec<(NoticeState, Option<String>, Option<String>)> {
        notices
            .iter()
            .map(|notice| {
                (
                    notice.state,
                    notice.status_code.clone(),
                    notice.detail.clone(),
                )
            })
            .collect()
    }

    fn question() -> (NoticeState, Option<String>, Option<String>) {
        (
            NoticeState::NeedsYou,
            Some("question".to_owned()),
            Some("Ship it?".to_owned()),
        )
    }

    /// The operator's report: a stop and then the question it ended on, a
    /// moment apart, were two notifications. The phone sends one, the
    /// question, once the row has waited out its moment.
    #[test]
    fn a_stop_then_a_question_is_one_notice_of_the_question() {
        let t0 = Instant::now();
        let mut phone = PhoneNotices::default();
        phone.seed(&rested("working", "none", "working", ""), t0);
        let stopped = rested("done", "none", "stopped", "");
        assert_eq!(phone.observe(&stopped, t0).0, Vec::new());
        assert_eq!(phone.next_due(), None, "a stop never waits to be sent");
        let asking = rested("needs_you", "question", "question", "Ship it?");
        let t1 = t0 + Duration::from_millis(500);
        assert_eq!(phone.observe(&asking, t1).0, Vec::new(), "not at once");
        assert_eq!(phone.next_due(), Some(t1 + SETTLE));
        assert!(
            phone
                .due(&asking, t1 + SETTLE - Duration::from_millis(1))
                .is_empty()
        );
        assert_eq!(said(&phone.due(&asking, t1 + SETTLE)), [question()]);
        assert_eq!(phone.next_due(), None);
        assert!(phone.due(&asking, t1 + SETTLE * 10).is_empty(), "sent once");
    }

    #[test]
    fn an_ask_answered_inside_its_moment_sends_nothing() {
        let t0 = Instant::now();
        let mut phone = PhoneNotices::default();
        phone.seed(&rested("working", "none", "working", ""), t0);
        let asking = rested("needs_you", "approval", "approval", "Run tests?");
        phone.observe(&asking, t0);
        let working = rested("working", "none", "working", "");
        phone.observe(&working, t0 + Duration::from_secs(1));
        assert_eq!(phone.next_due(), None);
        assert!(phone.due(&working, t0 + SETTLE).is_empty());
    }

    /// The moment starts when the row enters the operator's turn and never
    /// moves; the notice says the row as it is when the moment is up.
    #[test]
    fn a_notice_goes_out_on_time_with_the_rows_latest_ask() {
        let t0 = Instant::now();
        let mut phone = PhoneNotices::default();
        phone.seed(&rested("working", "none", "working", ""), t0);
        phone.observe(
            &rested("needs_you", "approval", "approval", "Run tests?"),
            t0,
        );
        let asking = rested("needs_you", "question", "question", "Ship it?");
        phone.observe(&asking, t0 + Duration::from_millis(2900));
        assert_eq!(phone.next_due(), Some(t0 + SETTLE), "the deadline holds");
        assert_eq!(said(&phone.due(&asking, t0 + SETTLE)), [question()]);
    }

    /// Operator decision 2026-11-11: a phone hears only what waits on the
    /// operator's answer. The desktop's ledger still announces a finished
    /// turn and a View, under its own switches.
    #[test]
    fn only_a_wait_on_the_operators_answer_reaches_a_phone() {
        let looking = {
            let mut agent = row("w1:p1", "needs_you", "none", None);
            agent["status_code"] = json!("idle");
            agent["look"] =
                json!({"group": "working", "count": 1, "view_id": "d1", "label": "report.html"});
            agent["mark"] = json!("look");
            project(&rest(json!([agent])), "local")
        };
        let steps = [
            rested("done", "none", "done", "Build fixed"),
            rested("seen", "none", "idle", ""),
            rested("done", "none", "unfinished", "Three tests remain"),
            rested("working", "none", "working", ""),
            rested("done", "none", "stopped", ""),
            rested("working", "none", "working", ""),
            looking,
            rested("seen", "none", "idle", ""),
            rested("needs_you", "error", "blocked", "Login required"),
        ];
        let t0 = Instant::now();
        let seed = rested("working", "none", "working", "");
        let mut phone = PhoneNotices::default();
        let mut desktop = Transitions::default();
        phone.seed(&seed, t0);
        desktop.observe_at(&seed, t0);
        let (mut sent, mut announced) = (Vec::new(), Vec::new());
        for (n, step) in steps.iter().enumerate() {
            let at = t0 + SETTLE * (n as u32 + 1);
            sent.extend(phone.observe(step, at).0);
            sent.extend(phone.due(step, at + SETTLE));
            announced.extend(desktop.observe_at(step, at).0);
        }
        assert_eq!(
            said(&sent),
            [(
                NoticeState::NeedsYou,
                Some("blocked".to_owned()),
                Some("Login required".to_owned())
            )],
            "only the block"
        );
        let codes: Vec<_> = announced
            .iter()
            .map(|notice| notice.status_code.clone().unwrap_or_default())
            .collect();
        assert_eq!(
            codes,
            ["done", "unfinished", "stopped", "look", "blocked"],
            "the desktop's ledger is unchanged"
        );
    }

    /// PRD ask-human B17 on the phone: a declaration raised under a row
    /// already in the operator's turn is one notice in its own words; a
    /// second inside the same moment makes it say the newest, on the first
    /// one's deadline.
    #[test]
    fn a_new_declaration_is_one_notice_in_its_own_words() {
        let turn = |id: &str| json!({"cause": "human_turn", "id": id, "kind": "auth", "place": "github.com/login", "device": "mini"});
        let root = |raised: &[&str]| {
            let mut row = row("w1:p1", "needs_you", "none", None);
            row["raised"] = json!(
                raised
                    .iter()
                    .map(|id| json!({"subject": turn(id)}))
                    .collect::<Vec<_>>()
            );
            row["human_turn"] = json!({"id": raised[0], "own": false});
            project(&rest(json!([row])), "local")
        };
        let t0 = Instant::now();
        let mut phone = PhoneNotices::default();
        phone.seed(&root(&["agent-3.1"]), t0);
        let both = root(&["agent-3.1", "agent-4.2"]);
        assert!(phone.observe(&both, t0).0.is_empty());
        let notices = phone.due(&both, t0 + SETTLE);
        assert_eq!(notices.len(), 1);
        assert_eq!(
            serde_json::to_value(&notices[0].subject).unwrap(),
            turn("agent-4.2")
        );
        let t1 = t0 + SETTLE * 2;
        phone.observe(&root(&["agent-3.1", "agent-4.2", "agent-5.3"]), t1);
        let all = root(&["agent-3.1", "agent-4.2", "agent-5.3", "agent-6.4"]);
        phone.observe(&all, t1 + Duration::from_secs(2));
        assert_eq!(phone.next_due(), Some(t1 + SETTLE), "the deadline holds");
        let notices = phone.due(&all, t1 + SETTLE);
        assert_eq!(notices.len(), 1);
        assert_eq!(
            serde_json::to_value(&notices[0].subject).unwrap(),
            turn("agent-6.4"),
            "the newest declaration"
        );
    }

    #[test]
    fn past_the_cap_the_first_due_goes_out_at_once_and_none_is_dropped() {
        let rows = |group: &str, demand: &str| {
            let agents: Vec<Value> = (0..=SETTLE_CAP)
                .map(|n| row(&format!("w1:p{n}"), group, demand, None))
                .collect();
            project(&rest(json!(agents)), "local")
        };
        let t0 = Instant::now();
        let mut phone = PhoneNotices::default();
        phone.seed(&rows("working", "none"), t0);
        let asking = rows("needs_you", "question");
        let (now, _) = phone.observe(&asking, t0);
        assert_eq!(now.len(), 1, "one over the cap goes out at once");
        assert_eq!(phone.due(&asking, t0 + SETTLE).len(), SETTLE_CAP);
    }

    /// A restart begins with an empty phone state: a fresh ledger seeds on
    /// what already waits, so nothing is sent a second time.
    #[test]
    fn a_restart_sends_nothing_for_what_already_waits() {
        let t0 = Instant::now();
        let asking = rested("needs_you", "question", "question", "Ship it?");
        let mut phone = PhoneNotices::default();
        phone.seed(&asking, t0);
        assert!(phone.observe(&asking, t0).0.is_empty());
        assert_eq!(phone.next_due(), None);
        assert!(phone.due(&asking, t0 + SETTLE).is_empty());
    }

    /// A root already waiting when the ledger seeds may have a notice on a
    /// phone from before: it is closed once when it stops waiting, whether it
    /// finishes unread, leaves only a View, or goes back to work, by either
    /// path that seeds the ledger.
    #[test]
    fn a_wait_found_at_the_seed_is_closed_once_when_it_stops_waiting() {
        let looking = {
            let mut agent = row("w1:p1", "needs_you", "none", None);
            agent["look"] =
                json!({"group": "working", "count": 1, "view_id": "d1", "label": "report.html"});
            project(&rest(json!([agent])), "local")
        };
        let asking = rested("needs_you", "question", "question", "Ship it?");
        for (label, after) in [
            ("finished unread", rested("done", "none", "done", "")),
            ("a View alone", looking),
            ("back at work", rested("working", "none", "working", "")),
        ] {
            for seed_by_observe in [false, true] {
                let t0 = Instant::now();
                let mut phone = PhoneNotices::default();
                if seed_by_observe {
                    assert_eq!(phone.observe(&asking, t0), (Vec::new(), BTreeSet::new()));
                } else {
                    phone.seed(&asking, t0);
                }
                let (now, close) = phone.observe(&after, t0 + SETTLE);
                assert!(now.is_empty(), "{label}");
                assert_eq!(
                    close.into_iter().map(|key| key.pane_id).collect::<Vec<_>>(),
                    ["w1:p1"],
                    "{label}"
                );
                assert!(
                    phone.observe(&after, t0 + SETTLE * 2).1.is_empty(),
                    "{label}: once"
                );
                assert!(phone.due(&after, t0 + SETTLE * 3).is_empty(), "{label}");
            }
        }
    }

    /// A notice sent while running is closed the same way, and a root that
    /// leaves before its moment had nothing sent and nothing to close.
    #[test]
    fn a_sent_notice_is_closed_when_its_root_finishes_and_an_unsent_one_is_not() {
        let t0 = Instant::now();
        let working = rested("working", "none", "working", "");
        let asking = rested("needs_you", "question", "question", "Ship it?");
        let done = rested("done", "none", "done", "");
        let mut phone = PhoneNotices::default();
        phone.seed(&working, t0);
        phone.observe(&asking, t0);
        assert!(
            phone
                .observe(&done, t0 + Duration::from_secs(1))
                .1
                .is_empty()
        );
        phone.observe(&asking, t0 + SETTLE);
        assert_eq!(phone.due(&asking, t0 + SETTLE * 2).len(), 1);
        assert_eq!(phone.observe(&done, t0 + SETTLE * 3).1.len(), 1);
    }

    /// A root absent from the list keeps its notice through the ledger's
    /// disappearance grace, and back still waiting it sends nothing.
    #[test]
    fn a_standing_root_that_drops_out_of_the_list_keeps_its_notice() {
        let t0 = Instant::now();
        let asking = rested("needs_you", "question", "question", "Ship it?");
        let mut phone = PhoneNotices::default();
        phone.seed(&asking, t0);
        let gone = project(&rest(json!([])), "local");
        assert_eq!(
            phone.observe(&gone, t0 + SETTLE),
            (Vec::new(), BTreeSet::new())
        );
        assert_eq!(
            phone.observe(&asking, t0 + SETTLE * 2),
            (Vec::new(), BTreeSet::new())
        );
        assert_eq!(phone.next_due(), None);
    }

    fn declared(raised: &[&str]) -> Projection {
        let turn = |id: &str| json!({"cause": "human_turn", "id": id, "kind": "auth", "place": "github.com/login", "device": "mini"});
        let mut root = row("w1:p1", "needs_you", "none", None);
        root["raised"] = json!(
            raised
                .iter()
                .map(|id| json!({"subject": turn(id)}))
                .collect::<Vec<_>>()
        );
        root["human_turn"] = json!({"id": raised[0], "own": false});
        project(&rest(json!([root])), "local")
    }

    fn turn_id(notice: &Notice) -> Value {
        serde_json::to_value(&notice.subject).unwrap()["id"].clone()
    }

    /// SETTLE belongs to a root: a withdrawn new declaration can still
    /// announce its older wait if the root remains in Needs You (lead's
    /// root-level settle policy).
    #[test]
    fn a_withdrawn_declaration_still_sends_once_if_its_root_keeps_waiting() {
        let t0 = Instant::now();
        let mut phone = PhoneNotices::default();
        phone.seed(&declared(&["agent-1.1"]), t0);
        phone.observe(&declared(&["agent-1.1", "agent-2.2"]), t0);
        let back = declared(&["agent-1.1"]);
        phone.observe(&back, t0 + Duration::from_secs(2));
        assert_eq!(phone.due(&back, t0 + SETTLE).len(), 1);
        let t1 = t0 + SETTLE * 2;
        let kept = declared(&["agent-1.1", "agent-3.3"]);
        phone.observe(&kept, t1);
        let notices = phone.due(&kept, t1 + SETTLE);
        assert_eq!(
            notices.iter().map(turn_id).collect::<Vec<_>>(),
            [json!("agent-3.3")]
        );
    }

    /// A root that newly waits says its row's latest ask, even when a turn
    /// added inside its moment is taken back.
    #[test]
    fn a_new_wait_whose_added_turn_is_taken_back_says_its_rows_ask() {
        let t0 = Instant::now();
        let mut phone = PhoneNotices::default();
        phone.seed(&rested("working", "none", "working", ""), t0);
        phone.observe(&declared(&["agent-1.1"]), t0);
        phone.observe(
            &declared(&["agent-1.1", "agent-2.2"]),
            t0 + Duration::from_secs(1),
        );
        let back = declared(&["agent-1.1"]);
        phone.observe(&back, t0 + Duration::from_secs(2));
        let notices = phone.due(&back, t0 + SETTLE);
        assert_eq!(
            notices.iter().map(turn_id).collect::<Vec<_>>(),
            [json!("agent-1.1")]
        );
    }

    #[test]
    fn the_mode_decides_by_computer_use() {
        assert!(mode_allows(PushMode::AppClosed, false));
        assert!(!mode_allows(PushMode::AppClosed, true));
        assert!(mode_allows(PushMode::Always, true));
    }

    #[test]
    fn a_notice_goes_to_a_phone_only_when_every_condition_holds() {
        let route = notice_route;
        assert_eq!(route(PushMode::Always, true, true, 1), Ok(()));
        assert_eq!(route(PushMode::AppClosed, true, false, 1), Ok(()));
        assert_eq!(
            route(PushMode::AppClosed, true, true, 1),
            Err("computer_in_use")
        );
        assert_eq!(route(PushMode::Always, false, false, 1), Err("mobile_off"));
        assert_eq!(
            route(PushMode::Always, true, false, 0),
            Err("no_subscription")
        );
        // Several causes at once report the first in the documented order.
        assert_eq!(
            route(PushMode::AppClosed, false, true, 0),
            Err("computer_in_use")
        );
        assert_eq!(route(PushMode::Always, false, false, 0), Err("mobile_off"));
    }
}
