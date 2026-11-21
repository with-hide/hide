//! Google's Firebase Cloud Messaging (HTTP v1), for the relay: an access token
//! from the VM's own service account through the GCE metadata server, one
//! reusable HTTP/2 connection to FCM, and a fixed data-only body built around
//! the ciphertext hided sent. Nothing here is persisted and no key file
//! exists: no token, no payload, no access token (`docs/RELAY_PROTOCOL.md`,
//! Push relay).
use crate::{
    apns::{Clock, Sent, endpoint_is_loopback},
    limits::{FCM_ACCESS_TOKEN_MARGIN, FCM_ACCESS_TOKEN_MAX_LIFETIME, PushLimits},
};
use hide_relay_protocol::admission::{FcmRequest, PushOutcome};
use http_body_util::{BodyExt, Empty, Full, Limited};
use hyper::{Method, Request, StatusCode, body::Bytes, header::HeaderValue};
use hyper_rustls::HttpsConnector;
use hyper_util::{
    client::legacy::{Client, connect::HttpConnector},
    rt::{TokioExecutor, TokioIo},
};
use std::time::Duration;

/// FCM's own host. Google's names are never taken from a request.
const SEND_HOST: &str = "https://fcm.googleapis.com";
/// The GCE metadata server, by its fixed link-local address so no name lookup
/// stands between the relay and its token; the `Host` header names it as
/// Google documents.
const METADATA_ADDRESS: &str = "169.254.169.254:80";
const METADATA_HOST: &str = "metadata.google.internal";
const TOKEN_PATH: &str = "/computeMetadata/v1/instance/service-accounts/default/token";
/// What Google reads besides the token and the ciphertext: a high-priority
/// data message kept an hour, with no notification block and no collapse key.
const PRIORITY: &str = "high";
const TTL: &str = "3600s";
/// FCM's limit on a message's data, keys and values together.
const MAX_DATA: usize = 4096;
/// The most of FCM's answer body the relay reads, and of the metadata
/// server's.
const MAX_ANSWER: usize = 8192;
/// An access token longer than this is not one the metadata server issues.
const MAX_ACCESS_TOKEN: usize = 4096;

/// Where the relay reaches Google. Production uses Google's own hosts; a test
/// names loopback stand-ins, refused for any other host.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FcmEndpoints {
    /// A stand-in for `fcm.googleapis.com`: `http://127.0.0.1:<port>` (h2c)
    /// or `https://127.0.0.1:<port>`.
    pub send: Option<String>,
    /// A stand-in for the metadata server: `http://127.0.0.1:<port>`.
    pub metadata: Option<String>,
}

/// The metadata stand-in is plain HTTP on this machine, as the metadata
/// server is plain HTTP on the VM's link-local address. The one rule the
/// configuration and `Fcm::new` both apply.
pub fn metadata_endpoint_is_loopback(endpoint: &str) -> bool {
    endpoint.starts_with("http://") && endpoint_is_loopback(endpoint)
}

/// Whether `project` is a Google Cloud project id, which goes into the path
/// Google reads: 6 to 30 lowercase letters, digits and hyphens, starting with
/// a letter and not ending with a hyphen. The one rule the configuration and
/// `Fcm::new` both apply.
pub fn project_valid(project: &str) -> bool {
    (6..=30).contains(&project.len())
        && project.starts_with(|first: char| first.is_ascii_lowercase())
        && !project.ends_with('-')
        && project
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

pub struct Fcm {
    client: Client<HttpsConnector<HttpConnector>, Full<Bytes>>,
    send_url: String,
    /// Where the token request connects, and the `Host` it names.
    metadata_address: String,
    metadata_host: String,
    /// One access token for every request, fetched again shortly before it
    /// expires; the lock also makes one fetch serve every send waiting on it.
    token: tokio::sync::Mutex<Option<AccessToken>>,
    timeout: Duration,
    clock: Clock,
}

struct AccessToken {
    value: String,
    /// Unix time, on the relay's clock.
    expires: Duration,
}

impl Fcm {
    /// `project` is the Firebase project the Android app belongs to. Both
    /// legs go through the HTTP stack Apple's client uses: the same
    /// hyper-rustls HTTP/2 client for FCM, and hyper's own HTTP/1 connection
    /// for the metadata server, which speaks nothing else.
    pub fn new(
        project: impl Into<String>,
        endpoints: FcmEndpoints,
        limits: PushLimits,
        clock: Clock,
    ) -> Result<Self, &'static str> {
        let project = project.into();
        if !project_valid(&project) {
            return Err("fcm_project_invalid");
        }
        let send_host = match endpoints.send {
            None => SEND_HOST.to_owned(),
            Some(endpoint) if endpoint_is_loopback(&endpoint) => {
                endpoint.trim_end_matches('/').to_owned()
            }
            Some(_) => return Err("fcm_endpoint_invalid"),
        };
        let (metadata_address, metadata_host) = match endpoints.metadata {
            None => (METADATA_ADDRESS.to_owned(), METADATA_HOST.to_owned()),
            Some(endpoint) if metadata_endpoint_is_loopback(&endpoint) => {
                let authority = endpoint
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                (authority.clone(), authority)
            }
            Some(_) => return Err("fcm_metadata_endpoint_invalid"),
        };
        let connector = hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_or_http()
            .enable_http2()
            .build();
        let mut builder = Client::builder(TokioExecutor::new());
        // One connection, kept between sends, as for Apple. The client never
        // follows a redirect: a 3xx is an answer like any other.
        builder
            .http2_only(true)
            .pool_idle_timeout(Duration::from_secs(300))
            .http2_keep_alive_interval(Duration::from_secs(30))
            .http2_keep_alive_timeout(Duration::from_secs(20))
            .timer(hyper_util::rt::TokioTimer::new());
        Ok(Self {
            client: builder.build(connector),
            send_url: format!("{send_host}/v1/projects/{project}/messages:send"),
            metadata_address,
            metadata_host,
            token: tokio::sync::Mutex::new(None),
            timeout: limits.request_timeout,
            clock,
        })
    }

    /// The JSON body: a fixed data-only message around the token and the
    /// base64url ciphertext. Both were checked by `FcmRequest::check`, so
    /// neither needs a JSON escape.
    pub fn body(request: &FcmRequest) -> Result<Vec<u8>, &'static str> {
        // The data block's keys and values, which FCM limits.
        if "hide".len() + request.payload.len() > MAX_DATA {
            return Err("fcm_body_too_large");
        }
        Ok(format!(
            r#"{{"message":{{"token":"{}","data":{{"hide":"{}"}},"android":{{"priority":"{PRIORITY}","ttl":"{TTL}"}}}}}}"#,
            request.token, request.payload
        )
        .into_bytes())
    }

    /// One send to FCM, with the one access token refresh a 401 earns, all
    /// inside the one request deadline.
    pub async fn send(&self, request: &FcmRequest) -> Sent {
        tokio::time::timeout(self.timeout, self.attempt(request))
            .await
            .unwrap_or(Sent {
                outcome: PushOutcome::Unavailable,
                reason: "fcm_unreachable",
            })
    }

    async fn attempt(&self, request: &FcmRequest) -> Sent {
        let failed = |outcome, reason| Sent { outcome, reason };
        let body = match Self::body(request) {
            Ok(body) => body,
            Err(reason) => return failed(PushOutcome::Failed, reason),
        };
        let token = match self.access_token(None).await {
            Ok(token) => token,
            Err(reason) => return failed(PushOutcome::Unavailable, reason),
        };
        match self.post(&body, &token).await {
            Ok(Answer::Done(sent)) => sent,
            Err(reason) => failed(PushOutcome::Unavailable, reason),
            // The only retry: a token Google refused is fetched again once
            // and the same request sent once more.
            Ok(Answer::Unauthorized) => {
                let renewed = match self.access_token(Some(&token)).await {
                    Ok(renewed) => renewed,
                    Err(reason) => return failed(PushOutcome::Unavailable, reason),
                };
                match self.post(&body, &renewed).await {
                    Ok(Answer::Done(sent)) => sent,
                    Ok(Answer::Unauthorized) => {
                        failed(PushOutcome::Unavailable, "fcm_unauthorized_again")
                    }
                    Err(reason) => failed(PushOutcome::Unavailable, reason),
                }
            }
        }
    }

    /// The access token now: the shared one while it has more than
    /// `FCM_ACCESS_TOKEN_MARGIN` left, otherwise a fresh one from the
    /// metadata server. With `refused`, Google turned that token down: a
    /// different one another send already fetched is used as it is, and the
    /// refused one is never used again.
    async fn access_token(&self, refused: Option<&str>) -> Result<String, &'static str> {
        let mut shared = self.token.lock().await;
        let now = (self.clock)();
        if let Some(token) = shared.as_ref()
            && refused != Some(token.value.as_str())
            && now + FCM_ACCESS_TOKEN_MARGIN < token.expires
        {
            return Ok(token.value.clone());
        }
        *shared = None;
        let token = self.fetch_token().await?;
        let value = token.value.clone();
        *shared = Some(token);
        Ok(value)
    }

    /// One request to the metadata server over a connection of its own,
    /// closed when the answer is read: a fetch happens about once an hour.
    async fn fetch_token(&self) -> Result<AccessToken, &'static str> {
        let stream = tokio::net::TcpStream::connect(&self.metadata_address)
            .await
            .map_err(|_| "fcm_metadata_unreachable")?;
        let (mut sender, connection) =
            hyper::client::conn::http1::handshake::<_, Empty<Bytes>>(TokioIo::new(stream))
                .await
                .map_err(|_| "fcm_metadata_unreachable")?;
        let request = Request::builder()
            .method(Method::GET)
            .uri(TOKEN_PATH)
            .header("host", self.metadata_host.as_str())
            .header("metadata-flavor", "Google")
            .header("connection", "close")
            .body(Empty::new())
            .map_err(|_| "fcm_metadata_request_invalid")?;
        let exchange = async move {
            let response = sender
                .send_request(request)
                .await
                .map_err(|_| "fcm_metadata_unreachable")?;
            // The metadata server marks its own answers; anything else on that
            // address is not it.
            let flavored = response
                .headers()
                .get("metadata-flavor")
                .is_some_and(|value| value == "Google");
            if response.status() != StatusCode::OK || !flavored {
                return Err("fcm_metadata_refused");
            }
            Limited::new(response.into_body(), MAX_ANSWER)
                .collect()
                .await
                .map(|body| body.to_bytes())
                .map_err(|_| "fcm_metadata_unreadable")
        };
        // The connection lives only as long as this one exchange. It may end
        // as soon as the answer is in, before the exchange has read it; the
        // exchange then finishes with what arrived, or fails.
        let mut exchange = std::pin::pin!(exchange);
        let body = tokio::select! {
            body = &mut exchange => body?,
            _ = connection => exchange.await?,
        };
        #[derive(serde::Deserialize)]
        struct Answer {
            access_token: String,
            expires_in: u64,
        }
        let answer: Answer =
            serde_json::from_slice(&body).map_err(|_| "fcm_metadata_unreadable")?;
        let valid = !answer.access_token.is_empty()
            && answer.access_token.len() <= MAX_ACCESS_TOKEN
            && answer
                .access_token
                .bytes()
                .all(|byte| byte.is_ascii_graphic());
        if !valid {
            return Err("fcm_metadata_unreadable");
        }
        let lifetime = Duration::from_secs(answer.expires_in).min(FCM_ACCESS_TOKEN_MAX_LIFETIME);
        Ok(AccessToken {
            value: answer.access_token,
            expires: (self.clock)() + lifetime,
        })
    }

    async fn post(&self, body: &[u8], token: &str) -> Result<Answer, &'static str> {
        let Ok(authorization) = HeaderValue::from_str(&format!("Bearer {token}")) else {
            return Err("fcm_access_token_invalid");
        };
        let request = Request::builder()
            .method(Method::POST)
            .uri(self.send_url.as_str())
            .header("authorization", authorization)
            .header("content-type", "application/json")
            .body(Full::new(Bytes::copy_from_slice(body)))
            .map_err(|_| "fcm_request_invalid")?;
        let response = self
            .client
            .request(request)
            .await
            .map_err(|_| "fcm_unreachable")?;
        let status = response.status();
        // Read to let the connection reuse its stream, and only up to
        // `MAX_ANSWER`; only a few fixed codes are kept from it.
        let body = Limited::new(response.into_body(), MAX_ANSWER)
            .collect()
            .await
            .map(|body| body.to_bytes())
            .unwrap_or_default();
        Ok(classify(status, &google_error(&body)))
    }
}

/// What FCM's answer means for the relay: an outcome, or the one answer that
/// earns a retry.
enum Answer {
    Done(Sent),
    Unauthorized,
}

/// The parts of Google's error body the relay classifies: the code of the
/// `google.firebase.fcm.v1.FcmError` detail, and which field a
/// `google.rpc.BadRequest` detail faults. Only allowlisted codes are kept and
/// only whether the field is the token; nothing else of the body leaves this
/// function.
#[derive(Debug, Default, Eq, PartialEq)]
struct GoogleError {
    code: &'static str,
    field: Fault,
}

/// What a `BadRequest` detail says about the request's fields.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum Fault {
    /// No field violation.
    #[default]
    None,
    /// `message.token`.
    Token,
    /// Some other field: the relay's own body, not the token.
    Other,
}

const FCM_ERROR: &str = "type.googleapis.com/google.firebase.fcm.v1.FcmError";
const BAD_REQUEST: &str = "type.googleapis.com/google.rpc.BadRequest";

fn google_error(body: &[u8]) -> GoogleError {
    const CODES: [&str; 3] = ["UNREGISTERED", "INVALID_ARGUMENT", "SENDER_ID_MISMATCH"];
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return GoogleError::default();
    };
    let details = value
        .pointer("/error/details")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let of_type = |kind: &'static str| {
        details.iter().filter(move |detail| {
            detail.get("@type").and_then(serde_json::Value::as_str) == Some(kind)
        })
    };
    let code = of_type(FCM_ERROR)
        .filter_map(|detail| detail.get("errorCode").and_then(serde_json::Value::as_str))
        .find_map(|code| CODES.into_iter().find(|allowed| *allowed == code))
        .unwrap_or("");
    let fields: Vec<Option<&str>> = of_type(BAD_REQUEST)
        .filter_map(|detail| detail.get("fieldViolations"))
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .map(|violation| violation.get("field").and_then(serde_json::Value::as_str))
        .collect();
    let field = if fields.is_empty() {
        Fault::None
    } else if fields.iter().all(|field| *field == Some("message.token")) {
        Fault::Token
    } else {
        Fault::Other
    };
    GoogleError { code, field }
}

/// The status-to-outcome table `docs/RELAY_PROTOCOL.md` documents.
fn classify(status: StatusCode, error: &GoogleError) -> Answer {
    let sent = |outcome, reason| Answer::Done(Sent { outcome, reason });
    match status.as_u16() {
        200..=299 => sent(PushOutcome::Delivered, "fcm_delivered"),
        // Only Google's own code says the install is gone: a bare 404 is a
        // wrong project or path, which must not make every computer drop its
        // phones' registrations.
        _ if error.code == "UNREGISTERED" => sent(PushOutcome::TokenGone, "fcm_unregistered"),
        404 => sent(PushOutcome::Failed, "fcm_not_found"),
        // Google's documented answer to a token it cannot read is
        // `INVALID_ARGUMENT` with no field named; one that names the token is
        // the same. One that names another field is the relay's own body.
        400 if error.field == Fault::Token
            || (error.code == "INVALID_ARGUMENT" && error.field == Fault::None) =>
        {
            sent(PushOutcome::TokenRejected, "fcm_token_rejected")
        }
        400 => sent(PushOutcome::Failed, "fcm_bad_request"),
        401 => Answer::Unauthorized,
        403 if error.code == "SENDER_ID_MISMATCH" => {
            sent(PushOutcome::TokenRejected, "fcm_sender_id_mismatch")
        }
        403 => sent(PushOutcome::Failed, "fcm_forbidden"),
        429 => sent(PushOutcome::Unavailable, "fcm_too_many_requests"),
        500..=599 => sent(PushOutcome::Unavailable, "fcm_server_error"),
        _ => sent(PushOutcome::Failed, "fcm_refused"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(body: &str) -> GoogleError {
        google_error(body.as_bytes())
    }

    /// Google's documented answer to an invalid token
    /// (firebase.google.com/docs/cloud-messaging/error-codes, Example 2).
    const DOCUMENTED_INVALID_TOKEN: &str = r#"{"error":{"code":400,"message":"The registration token is not a valid FCM registration token","status":"INVALID_ARGUMENT","details":[{"@type":"type.googleapis.com/google.firebase.fcm.v1.FcmError","errorCode":"INVALID_ARGUMENT"}]}}"#;

    #[test]
    fn google_s_error_body_yields_only_the_typed_code_and_the_faulted_field() {
        let fcm = |code: &str| format!(r#"{{"@type":"{FCM_ERROR}","errorCode":"{code}"}}"#);
        let bad = |field: &str| {
            format!(r#"{{"@type":"{BAD_REQUEST}","fieldViolations":[{{"field":"{field}"}}]}}"#)
        };
        let body =
            |details: &[String]| format!(r#"{{"error":{{"details":[{}]}}}}"#, details.join(","));
        for (body, code, field) in [
            (
                DOCUMENTED_INVALID_TOKEN.to_owned(),
                "INVALID_ARGUMENT",
                Fault::None,
            ),
            (body(&[fcm("UNREGISTERED")]), "UNREGISTERED", Fault::None),
            (
                body(&[fcm("INVALID_ARGUMENT"), bad("message.token")]),
                "INVALID_ARGUMENT",
                Fault::Token,
            ),
            (
                body(&[fcm("INVALID_ARGUMENT"), bad("message.data")]),
                "INVALID_ARGUMENT",
                Fault::Other,
            ),
            // A code outside the allowlist, or under another detail type, is
            // dropped, and so are free text and junk.
            (body(&[fcm("SOMETHING_NEW")]), "", Fault::None),
            (
                r#"{"error":{"details":[{"errorCode":"UNREGISTERED"}]}}"#.to_owned(),
                "",
                Fault::None,
            ),
            (
                r#"{"error":{"message":"UNREGISTERED"}}"#.to_owned(),
                "",
                Fault::None,
            ),
            ("<html>".to_owned(), "", Fault::None),
            (String::new(), "", Fault::None),
        ] {
            assert_eq!(error(&body), GoogleError { code, field }, "{body}");
        }
    }

    #[test]
    fn each_status_and_code_lands_on_one_outcome() {
        use StatusCode as S;
        let with = |code, field| GoogleError { code, field };
        let none = GoogleError::default();
        for (status, error, outcome) in [
            (S::OK, &none, PushOutcome::Delivered),
            (
                S::NOT_FOUND,
                &with("UNREGISTERED", Fault::None),
                PushOutcome::TokenGone,
            ),
            (
                S::BAD_REQUEST,
                &with("UNREGISTERED", Fault::None),
                PushOutcome::TokenGone,
            ),
            // A 404 without Google's code is a wrong project or path.
            (S::NOT_FOUND, &none, PushOutcome::Failed),
            (
                S::BAD_REQUEST,
                &with("INVALID_ARGUMENT", Fault::None),
                PushOutcome::TokenRejected,
            ),
            (
                S::BAD_REQUEST,
                &with("INVALID_ARGUMENT", Fault::Token),
                PushOutcome::TokenRejected,
            ),
            (
                S::BAD_REQUEST,
                &with("", Fault::Token),
                PushOutcome::TokenRejected,
            ),
            (
                S::BAD_REQUEST,
                &with("INVALID_ARGUMENT", Fault::Other),
                PushOutcome::Failed,
            ),
            (S::BAD_REQUEST, &none, PushOutcome::Failed),
            (
                S::FORBIDDEN,
                &with("SENDER_ID_MISMATCH", Fault::None),
                PushOutcome::TokenRejected,
            ),
            (S::FORBIDDEN, &none, PushOutcome::Failed),
            (S::TOO_MANY_REQUESTS, &none, PushOutcome::Unavailable),
            (S::INTERNAL_SERVER_ERROR, &none, PushOutcome::Unavailable),
            (S::SERVICE_UNAVAILABLE, &none, PushOutcome::Unavailable),
            (S::MOVED_PERMANENTLY, &none, PushOutcome::Failed),
            (S::CONFLICT, &none, PushOutcome::Failed),
        ] {
            let Answer::Done(sent) = classify(status, error) else {
                panic!("{status} {error:?} is not a retry");
            };
            assert_eq!(sent.outcome, outcome, "{status} {error:?}");
        }
        assert!(matches!(
            classify(S::UNAUTHORIZED, &none),
            Answer::Unauthorized
        ));
    }

    #[test]
    fn a_project_id_is_google_s_shape() {
        for allowed in ["hide-android", "abcdef", "a12345-b6"] {
            assert!(project_valid(allowed), "{allowed}");
        }
        for refused in [
            "short",
            "Hide-android",
            "1hide-android",
            "hide-android-",
            "hide_android",
            "hide/android",
            "hide-android-0123456789-0123456789",
        ] {
            assert!(!project_valid(refused), "{refused}");
        }
    }
}
