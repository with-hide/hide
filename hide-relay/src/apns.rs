//! Apple's push service, for the relay: one shared provider token, one
//! reusable HTTP/2 connection, and a fixed body built around the ciphertext
//! hided sent. Nothing here is persisted: no token, no payload, no JWT
//! (`docs/RELAY_PROTOCOL.md`, Push relay).
use crate::limits::{PROVIDER_TOKEN_LIFETIME, PROVIDER_TOKEN_REFRESH, PushLimits};
use hide_relay_protocol::admission::{PushEnvironment, PushOutcome, PushRequest};
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, StatusCode, body::Bytes, header::HeaderValue};
use hyper_rustls::HttpsConnector;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::TokioExecutor,
};
use ring::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// The production hosts. Apple's own names are never taken from a request.
const PRODUCTION_HOST: &str = "https://api.push.apple.com";
const SANDBOX_HOST: &str = "https://api.sandbox.push.apple.com";

/// The plaintext body Apple and its network see: a fixed alert key and the
/// ciphertext. `hide-notification` is the app's contract; the phone supplies
/// the words (`docs/RELAY_PROTOCOL.md`, The notification body).
const ALERT_KEY: &str = "hide-notification";
const PUSH_TYPE: &str = "alert";
const PRIORITY: &str = "10";
const EXPIRATION: Duration = Duration::from_secs(60 * 60);
/// Apple's body limit, and the ceiling the relay's own build must stay under.
const MAX_BODY: usize = 4096;

/// The `.p8` signing key and the ids Apple issued with it.
pub struct ProviderKey {
    key: EcdsaKeyPair,
    key_id: String,
    team_id: String,
    topic: String,
}

impl ProviderKey {
    /// Reads Apple's own PKCS#8 `.p8`. The bytes are the caller's; the key
    /// stays here and is never written anywhere by this module.
    pub fn from_pkcs8(
        pkcs8: &[u8],
        key_id: impl Into<String>,
        team_id: impl Into<String>,
        topic: impl Into<String>,
    ) -> Result<Self, &'static str> {
        let signing = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            pkcs8,
            &SystemRandom::new(),
        )
        .map_err(|_| "apns_key_unreadable")?;
        let (key_id, team_id, topic) = (key_id.into(), team_id.into(), topic.into());
        if [&key_id, &team_id, &topic]
            .iter()
            .any(|part| part.is_empty() || part.len() > 128 || !part.bytes().all(identifier_byte))
        {
            return Err("apns_id_invalid");
        }
        Ok(Self {
            key: signing,
            key_id,
            team_id,
            topic,
        })
    }

    /// The provider token for one moment: a JWT Apple accepts.
    fn token(&self, issued_at: u64) -> Result<String, &'static str> {
        use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
        let header = URL_SAFE_NO_PAD
            .encode(format!(r#"{{"alg":"ES256","kid":"{}"}}"#, self.key_id).as_bytes());
        let claims = URL_SAFE_NO_PAD
            .encode(format!(r#"{{"iss":"{}","iat":{issued_at}}}"#, self.team_id).as_bytes());
        let input = format!("{header}.{claims}");
        let signature = self
            .key
            .sign(&SystemRandom::new(), input.as_bytes())
            .map_err(|_| "apns_signing_failed")?;
        Ok(format!(
            "{input}.{}",
            URL_SAFE_NO_PAD.encode(signature.as_ref())
        ))
    }
}

/// A header the ids may carry: Apple issues decimal key ids and alphanumeric
/// team ids and topics.
fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'.' || byte == b'-' || byte == b'_'
}

/// A clock the relay supplies, so a test moves the provider token's schedule
/// without waiting out 45 minutes.
pub type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

pub struct Apns {
    key: ProviderKey,
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    endpoint: String,
    /// One token for every request, replaced on one schedule rather than
    /// minted per request (`PROVIDER_TOKEN_REFRESH`).
    token: Mutex<SharedToken>,
    concurrent: Arc<Semaphore>,
    timeout: Duration,
    clock: Clock,
}

struct SharedToken {
    value: String,
    issued: Duration,
    replaced: Duration,
}

/// What one send produced, before the relay turns it into a result.
pub struct Sent {
    pub outcome: PushOutcome,
    /// A fixed reason for the diagnostic: a status class, never Apple's body.
    pub reason: &'static str,
}

impl Apns {
    /// `endpoint` is Apple's host, or the loopback stand-in a test uses. Both
    /// go through the same client; only the scheme differs.
    pub fn new(
        key: ProviderKey,
        endpoint: impl Into<String>,
        limits: PushLimits,
        clock: Clock,
    ) -> Result<Self, &'static str> {
        let endpoint = endpoint.into();
        // Empty means Apple's own host; anything else is the loopback
        // stand-in the configuration already checked.
        if !endpoint.is_empty()
            && !endpoint.starts_with("https://")
            && !endpoint.starts_with("http://")
        {
            return Err("apns_endpoint_invalid");
        }
        let connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http2()
            .build();
        let mut builder = Client::builder(TokioExecutor::new());
        // One connection, kept between sends; a dropped one is replaced by
        // the next request rather than a new connection per request.
        builder
            .http2_only(true)
            .pool_idle_timeout(Duration::from_secs(300))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .http2_keep_alive_timeout(Duration::from_secs(20))
            .timer(hyper_util::rt::TokioTimer::new());
        let client = builder.build(connector);
        let now = clock();
        Ok(Self {
            key,
            client,
            endpoint,
            token: Mutex::new(SharedToken {
                value: String::new(),
                issued: now,
                replaced: now,
            }),
            concurrent: Arc::new(Semaphore::new(limits.concurrent)),
            timeout: limits.request_timeout,
            clock,
        })
    }

    /// A slot on the one connection, held for a whole request.
    pub fn slot(&self) -> Option<OwnedSemaphorePermit> {
        self.concurrent.clone().try_acquire_owned().ok()
    }

    /// The provider token now, replaced only on the shared schedule. `iat`
    /// comes from the uptime clock a test can move.
    fn token(&self) -> Result<String, &'static str> {
        let now = (self.clock)();
        let mut shared = self.token.lock().unwrap_or_else(|error| error.into_inner());
        if shared.value.is_empty()
            || now.saturating_sub(shared.replaced) >= PROVIDER_TOKEN_REFRESH
            || now.saturating_sub(shared.issued) >= PROVIDER_TOKEN_LIFETIME
        {
            shared.value = self.key.token(now.as_secs())?;
            shared.issued = now;
            shared.replaced = now;
        }
        Ok(shared.value.clone())
    }

    /// Replaces the shared token because Apple refused it, and only when the
    /// refused one is still the shared one (another send may have already
    /// replaced it).
    fn renew_token(&self, refused: &str) -> Result<String, &'static str> {
        let now = (self.clock)();
        let mut shared = self.token.lock().unwrap_or_else(|error| error.into_inner());
        if shared.value == refused {
            shared.value = self.key.token(now.as_secs())?;
            shared.issued = now;
            // A refusal is not the scheduled replacement, so the schedule
            // waits a full interval from now before replacing it again.
            shared.replaced = now;
        }
        Ok(shared.value.clone())
    }

    /// The JSON body: a fixed envelope around the base64url ciphertext.
    pub fn body(request: &PushRequest) -> Result<Vec<u8>, &'static str> {
        let body = format!(
            r#"{{"aps":{{"alert":{{"loc-key":"{ALERT_KEY}"}},"mutable-content":1}},"hide":"{}"}}"#,
            request.payload
        );
        if body.len() > MAX_BODY {
            return Err("apns_body_too_large");
        }
        Ok(body.into_bytes())
    }

    fn url(&self, environment: PushEnvironment, token: &str) -> String {
        // A loopback stand-in replaces the host and keeps the path, so the
        // request a test sees is the request Apple would see.
        let host = if self.endpoint.is_empty() {
            match environment {
                PushEnvironment::Production => PRODUCTION_HOST,
                PushEnvironment::Sandbox => SANDBOX_HOST,
            }
        } else {
            self.endpoint.as_str()
        };
        format!("{host}/3/device/{token}")
    }

    /// One send to Apple, with the one retry Apple's expired token earns.
    pub async fn send(&self, request: &PushRequest, _slot: OwnedSemaphorePermit) -> Sent {
        let body = match Self::body(request) {
            Ok(body) => body,
            Err(reason) => {
                return Sent {
                    outcome: PushOutcome::Failed,
                    reason,
                };
            }
        };
        let token = match self.token() {
            Ok(token) => token,
            Err(reason) => {
                return Sent {
                    outcome: PushOutcome::Unavailable,
                    reason,
                };
            }
        };
        let first = self.post(request, &body, &token).await;
        let first = match first {
            Ok(outcome) => outcome,
            Err(reason) => {
                return Sent {
                    outcome: PushOutcome::Unavailable,
                    reason,
                };
            }
        };
        if first.reason == "expired_token" {
            // The only retry in the relay: a token Apple expired is signed
            // once more and the same request is sent once more.
            let renewed = match self.renew_token(&token) {
                Ok(renewed) => renewed,
                Err(reason) => {
                    return Sent {
                        outcome: PushOutcome::Unavailable,
                        reason,
                    };
                }
            };
            return match self.post(request, &body, &renewed).await {
                Ok(outcome) if outcome.outcome == PushOutcome::Delivered => outcome,
                _ => Sent {
                    outcome: PushOutcome::Unavailable,
                    reason: "expired_token_again",
                },
            };
        }
        first
    }

    async fn post(
        &self,
        request: &PushRequest,
        body: &[u8],
        token: &str,
    ) -> Result<Sent, &'static str> {
        let mut builder = Request::builder()
            .method(Method::POST)
            .uri(self.url(request.environment, &request.token))
            .header("apns-topic", self.key.topic.as_str())
            .header("apns-push-type", PUSH_TYPE)
            .header("apns-priority", PRIORITY)
            .header("apns-collapse-id", request.collapse.as_str())
            .header(
                "apns-expiration",
                (unix_now().saturating_add(EXPIRATION.as_secs())).to_string(),
            )
            .header("content-type", "application/json");
        let authorization = format!("bearer {token}");
        let Ok(value) = HeaderValue::from_str(&authorization) else {
            return Err("apns_token_invalid");
        };
        builder = builder.header("authorization", value);
        let request = builder
            .body(Full::new(Bytes::copy_from_slice(body)))
            .map_err(|_| "apns_request_invalid")?;
        let answer = tokio::time::timeout(self.timeout, self.client.request(request)).await;
        let Ok(Ok(response)) = answer else {
            return Err("apns_unreachable");
        };
        let status = response.status();
        // Read the body only to let the connection reuse its stream; the
        // reason is Apple's `reason` field, never free text.
        let reason = match response.into_body().collect().await {
            Ok(body) => apple_reason(&body.to_bytes()),
            Err(_) => "",
        };
        Ok(classify(status, reason))
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Apple's reason string, from its small JSON body, and only ever one of the
/// few the relay classifies; anything else is dropped here.
fn apple_reason(body: &[u8]) -> &'static str {
    const REASONS: [&str; 6] = [
        "BadDeviceToken",
        "DeviceTokenNotForTopic",
        "Unregistered",
        "ExpiredProviderToken",
        "TooManyProviderTokenUpdates",
        "BadCollapseId",
    ];
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return "";
    };
    let Some(reason) = value.get("reason").and_then(|reason| reason.as_str()) else {
        return "";
    };
    REASONS
        .into_iter()
        .find(|allowed| *allowed == reason)
        .unwrap_or("")
}

/// The status-to-outcome table `docs/RELAY_PROTOCOL.md` documents.
fn classify(status: StatusCode, reason: &str) -> Sent {
    let sent = |outcome, reason| Sent { outcome, reason };
    match status.as_u16() {
        200..=299 => sent(PushOutcome::Delivered, "delivered"),
        410 => sent(PushOutcome::TokenGone, "unregistered"),
        400 => match reason {
            "BadDeviceToken" => sent(PushOutcome::TokenGone, "bad_device_token"),
            "DeviceTokenNotForTopic" => sent(PushOutcome::TokenGone, "device_token_not_for_topic"),
            _ => sent(PushOutcome::Failed, "bad_request"),
        },
        403 if reason == "ExpiredProviderToken" => sent(PushOutcome::Failed, "expired_token"),
        403 => sent(PushOutcome::Failed, "forbidden"),
        429 => sent(PushOutcome::Unavailable, "too_many_requests"),
        500..=599 => sent(PushOutcome::Unavailable, "server_error"),
        _ => sent(PushOutcome::Failed, "refused"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_body_is_the_fixed_envelope_around_the_ciphertext_and_stays_under_apple_s_limit() {
        let request = PushRequest {
            id: 1,
            environment: PushEnvironment::Sandbox,
            token: "ab".repeat(32),
            collapse: "collapse".into(),
            payload: "A".repeat(4000),
        };
        let body = Apns::body(&request).unwrap();
        assert_eq!(
            body,
            format!(
                r#"{{"aps":{{"alert":{{"loc-key":"hide-notification"}},"mutable-content":1}},"hide":"{}"}}"#,
                request.payload
            )
            .into_bytes()
        );
        assert!(body.len() <= MAX_BODY);
        let too_large = PushRequest {
            payload: "A".repeat(4000),
            ..request
        };
        assert!(Apns::body(&too_large).is_ok());
    }

    #[test]
    fn each_status_and_reason_lands_on_one_outcome() {
        use StatusCode as S;
        for (status, reason, outcome) in [
            (S::OK, "", PushOutcome::Delivered),
            (S::GONE, "", PushOutcome::TokenGone),
            (S::BAD_REQUEST, "BadDeviceToken", PushOutcome::TokenGone),
            (
                S::BAD_REQUEST,
                "DeviceTokenNotForTopic",
                PushOutcome::TokenGone,
            ),
            (S::BAD_REQUEST, "PayloadTooLarge", PushOutcome::Failed),
            (S::FORBIDDEN, "ExpiredProviderToken", PushOutcome::Failed),
            (S::FORBIDDEN, "BadCertificate", PushOutcome::Failed),
            (S::TOO_MANY_REQUESTS, "", PushOutcome::Unavailable),
            (S::INTERNAL_SERVER_ERROR, "", PushOutcome::Unavailable),
            (S::SERVICE_UNAVAILABLE, "", PushOutcome::Unavailable),
            (S::NOT_FOUND, "", PushOutcome::Failed),
        ] {
            assert_eq!(
                classify(status, reason).outcome,
                outcome,
                "{status} {reason}"
            );
        }
        assert_eq!(
            classify(S::FORBIDDEN, "ExpiredProviderToken").reason,
            "expired_token"
        );
    }
}
