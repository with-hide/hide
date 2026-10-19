//! Typed transport for Herdr's newline-delimited JSON socket API.

pub mod plugin;

use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use hide_platform::ipc::{LocalStream, ShutdownHandle};
use serde::Deserialize;
use serde_json::{Value, json};

/// The canonical schema copied from the pinned Herdr binary.
pub const HERDR_API_SCHEMA_JSON: &str = include_str!("../../contracts/herdr-api.schema.json");

include!(concat!(env!("OUT_DIR"), "/herdr_contract.rs"));

/// Herdr API protocol revision this client speaks. A mismatch is a hard,
/// explicit failure instead of a partially working sidebar.
pub const HERDR_PROTOCOL_REVISION: u64 = HERDR_PROTOCOL_REVISION_SCHEMA as u64;

const API_TIMEOUT: Duration = Duration::from_secs(5);

pub trait ConnectionShutdown: Send {
    fn shutdown(&self);
}

pub trait ApiStream: Read + Write + Send {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError>;

    fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError>;

    fn read_line_with_timeout(&mut self, timeout: Duration) -> Result<String, ApiError>;

    fn shutdown_handle(&self) -> Result<Box<dyn ConnectionShutdown>, ApiError>;
}

pub trait ApiConnector: Send + Sync {
    fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError>;

    /// Establishes a stream within this absolute deadline. The default
    /// refuses instead of falling back to a potentially unbounded connect.
    fn connect_until(&self, _deadline: Instant) -> Result<Box<dyn ApiStream>, ApiError> {
        Err(ApiError::Transport(
            "this transport cannot enforce a connection deadline".to_owned(),
        ))
    }
}

/// Reaches Herdr's socket API over the local stream the platform has: a Unix
/// socket on macOS and Linux, a named pipe on Windows (`hide_platform::ipc`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalSocketConnector {
    socket_path: PathBuf,
}

impl LocalSocketConnector {
    pub fn new(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }
}

impl ApiConnector for LocalSocketConnector {
    fn connect_until(&self, deadline: Instant) -> Result<Box<dyn ApiStream>, ApiError> {
        LocalStream::connect_until(&self.socket_path, deadline)
            .map(|stream| Box::new(stream) as Box<dyn ApiStream>)
            .map_err(|error| ApiError::Transport(format!("deadline-bound connect failed: {error}")))
    }

    fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
        let stream = LocalStream::connect(&self.socket_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ApiError::NotRunning(format!(
                    "Herdr socket file does not exist at {}; the herdr server is not running",
                    self.socket_path.display()
                ))
            } else {
                ApiError::Transport(format!(
                    "connect failed for {}: {error}",
                    self.socket_path.display()
                ))
            }
        })?;
        Ok(Box::new(stream))
    }
}

struct LocalShutdown(ShutdownHandle);

impl ConnectionShutdown for LocalShutdown {
    fn shutdown(&self) {
        self.0.shutdown();
    }
}

impl ApiStream for LocalStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        LocalStream::set_read_timeout(self, timeout)
            .map_err(|error| ApiError::Transport(format!("read timeout could not be set: {error}")))
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        LocalStream::set_write_timeout(self, timeout).map_err(|error| {
            ApiError::Transport(format!("write timeout could not be set: {error}"))
        })
    }

    fn read_line_with_timeout(&mut self, timeout: Duration) -> Result<String, ApiError> {
        let line = read_acknowledgement(self, Instant::now() + timeout);
        // Whatever happened, the stream leaves here blocking: the reader it
        // becomes after the acknowledgement has no deadline.
        let restored = LocalStream::set_read_timeout(self, None).map_err(|error| {
            ApiError::Transport(format!(
                "subscription could not enter blocking mode: {error}"
            ))
        });
        let line = line?;
        restored?;
        String::from_utf8(line).map_err(|error| {
            ApiError::Malformed(format!(
                "subscription acknowledgement was not UTF-8: {error}"
            ))
        })
    }

    fn shutdown_handle(&self) -> Result<Box<dyn ConnectionShutdown>, ApiError> {
        Ok(Box::new(LocalShutdown(LocalStream::shutdown_handle(self))))
    }
}

/// Reads one line a byte at a time, so nothing after it is consumed: the
/// events that follow the acknowledgement stay in the stream for the reader.
fn read_acknowledgement(stream: &mut LocalStream, deadline: Instant) -> Result<Vec<u8>, ApiError> {
    let mut bytes = Vec::new();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ApiError::Transport(
                "subscription acknowledgement timed out".to_owned(),
            ));
        }
        LocalStream::set_read_timeout(stream, Some(remaining)).map_err(|error| {
            ApiError::Transport(format!("read timeout could not be set: {error}"))
        })?;
        let mut byte = [0_u8; 1];
        match stream.read(&mut byte) {
            Ok(0) => {
                return Err(ApiError::Transport(
                    "subscription acknowledgement reached EOF".to_owned(),
                ));
            }
            Ok(_) => {
                bytes.push(byte[0]);
                if byte[0] == b'\n' {
                    return Ok(bytes);
                }
                if bytes.len() > 64 * 1024 {
                    return Err(ApiError::Malformed(
                        "subscription acknowledgement exceeds 64 KiB".to_owned(),
                    ));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                return Err(ApiError::Transport(
                    "subscription acknowledgement timed out".to_owned(),
                ));
            }
            Err(error) => {
                return Err(ApiError::Transport(format!(
                    "subscription acknowledgement could not be read: {error}"
                )));
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApiError {
    /// The server's socket is not there: no server is running at it.
    NotRunning(String),
    Transport(String),
    Remote {
        code: String,
        message: String,
    },
    Malformed(String),
}

impl ApiError {
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Remote { code, .. } => Some(code),
            Self::NotRunning(_) | Self::Transport(_) | Self::Malformed(_) => None,
        }
    }

    /// Whether the request was written and its answer never came: a read
    /// that timed out or a connection that closed unanswered. Herdr may have
    /// done what was asked, so a caller that made a mutation reconciles
    /// before it asks again; a refused connect or write did nothing.
    pub fn reply_unknown(&self) -> bool {
        matches!(self, Self::Transport(message) if message.starts_with("response "))
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotRunning(message) | Self::Transport(message) | Self::Malformed(message) => {
                formatter.write_str(message)
            }
            Self::Remote { code, message } => write!(formatter, "{code}: {message}"),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct ResponseEnvelope {
    id: String,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<ApiErrorBody>,
}

/// The acknowledgement of `events.subscribe`. Herdr's stable release carries
/// nothing in it beyond its type: there is no event journal to resume from,
/// so a subscription always starts at "now" and a consumer that needs the
/// state before that reads a snapshot after subscribing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct SubscriptionStarted {
    #[serde(rename = "type")]
    pub kind: String,
}

pub struct Subscription {
    pub ack: SubscriptionStarted,
    reader: BufReader<Box<dyn ApiStream>>,
    shutdown: Box<dyn ConnectionShutdown>,
}

impl Subscription {
    pub fn into_parts(self) -> (Box<dyn BufRead + Send>, Box<dyn ConnectionShutdown>) {
        (Box::new(self.reader), self.shutdown)
    }
}

pub fn request(socket_path: &Path, method: &str, params: Value) -> Result<Value, String> {
    request_with_timeout(socket_path, method, params, API_TIMEOUT)
        .map_err(|error| format!("{method} failed: {error}"))
}

pub fn request_with_timeout(
    socket_path: &Path,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, ApiError> {
    request_with_connector(
        &LocalSocketConnector::new(socket_path),
        method,
        params,
        timeout,
    )
}

pub fn request_with_connector(
    connector: &dyn ApiConnector,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, ApiError> {
    request_with_correlation_id(
        connector,
        &format!("herdr-core:{method}"),
        method,
        params,
        timeout,
    )
}

/// Read a small response with an absolute read deadline and a 64 KiB frame cap.
/// Connection establishment has the connector's own deadline.
pub fn request_small_response(
    connector: &dyn ApiConnector,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, ApiError> {
    let request_id = format!("herdr-core:{method}");
    let mut stream = connector.connect()?;
    stream.set_write_timeout(Some(timeout))?;
    write_request(stream.as_mut(), &request_id, method, params)?;
    let response = decode_response(&stream.read_line_with_timeout(timeout)?)?;
    response_result(response, &request_id)
}

/// A small read whose write and read share the caller's end time.
/// Connection establishment uses the connector's own bound and cannot be
/// interrupted here. A caller with a short total budget must choose a
/// connector whose connect fits it; the Factory question guard permits only
/// its local connector. Expiry after connecting prevents a request from being
/// written. Responses retain the 64 KiB cap.
pub fn request_small_response_until(
    connector: &dyn ApiConnector,
    method: &str,
    params: Value,
    deadline: Instant,
) -> Result<Value, ApiError> {
    request_small_response_at_deadline(connector, method, params, deadline, false)
}

/// A small request whose connection, write and read share one absolute
/// deadline. Unsupported transports refuse without starting a connection.
pub fn request_small_response_strict_until(
    connector: &dyn ApiConnector,
    method: &str,
    params: Value,
    deadline: Instant,
) -> Result<Value, ApiError> {
    request_small_response_at_deadline(connector, method, params, deadline, true)
}

fn request_small_response_at_deadline(
    connector: &dyn ApiConnector,
    method: &str,
    params: Value,
    deadline: Instant,
    strict_connect: bool,
) -> Result<Value, ApiError> {
    let remaining = || {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or_else(|| ApiError::Transport("request deadline expired".to_owned()))
    };
    remaining()?;
    let request_id = format!("herdr-core:{method}");
    let mut stream = if strict_connect {
        connector.connect_until(deadline)?
    } else {
        connector.connect()?
    };
    let mut request = serde_json::to_vec(&json!({"id":request_id,"method":method,"params":params}))
        .map_err(|error| ApiError::Malformed(format!("request could not be encoded: {error}")))?;
    request.push(b'\n');
    let mut pending = request.as_slice();
    while !pending.is_empty() {
        stream.set_write_timeout(Some(remaining()?))?;
        match stream.write(pending) {
            Ok(0) => {
                return Err(ApiError::Transport(
                    "request write made no progress".to_owned(),
                ));
            }
            Ok(written) => pending = &pending[written..],
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Err(ApiError::Transport(format!(
                    "request could not be written: {error}"
                )));
            }
        }
    }
    let line = stream.read_line_with_timeout(remaining()?)?;
    remaining()?;
    response_result(decode_response(&line)?, &request_id)
}

/// Sends one request with a caller-selected correlation id.
///
/// The pinned Herdr contract explicitly keeps this envelope id separate from
/// mutation idempotency. Callers may use it to correlate one reopen stage in
/// diagnostics, but must reconcile an ambiguous transport failure before
/// submitting that external effect again.
pub fn request_with_correlation_id(
    connector: &dyn ApiConnector,
    request_id: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, ApiError> {
    let mut stream = connector.connect()?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write_request(stream.as_mut(), request_id, method, params)?;
    let response = read_response(&mut BufReader::new(stream))?;
    response_result(response, request_id)
}

/// Sends the single request on a caller-established connection, allowing
/// the caller to recheck its authority after the actual connect.
/// Herdr closes that connection after answering. Responses are capped at 64 KiB.
pub fn request_on_stream(
    stream: &mut dyn ApiStream,
    request_id: &str,
    method: &str,
    params: Value,
    timeout: Duration,
) -> Result<Value, ApiError> {
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    write_request(stream, request_id, method, params)?;
    let response = decode_response(&stream.read_line_with_timeout(timeout)?)?;
    response_result(response, request_id)
}

/// Open an event subscription. `params` is the `events.subscribe` parameter
/// object, which the caller builds from the contract (`herdr-core`'s `wire`).
pub fn subscribe(
    socket_path: &Path,
    params: Value,
    timeout: Duration,
) -> Result<Subscription, ApiError> {
    subscribe_with_connector(&LocalSocketConnector::new(socket_path), params, timeout)
}

pub fn subscribe_with_connector(
    connector: &dyn ApiConnector,
    params: Value,
    timeout: Duration,
) -> Result<Subscription, ApiError> {
    let mut stream = connector.connect()?;
    stream.set_write_timeout(Some(timeout))?;
    let request_id = "herdr-core:events.subscribe";
    write_request(stream.as_mut(), request_id, "events.subscribe", params)?;

    let response = decode_response(&stream.read_line_with_timeout(timeout)?)?;
    let result = response_result(response, request_id)?;
    let ack: SubscriptionStarted = serde_json::from_value(result).map_err(|error| {
        ApiError::Malformed(format!(
            "events.subscribe acknowledgement is malformed: {error}"
        ))
    })?;
    if ack.kind != "subscription_started" {
        return Err(ApiError::Malformed(format!(
            "events.subscribe returned unexpected result type {:?}",
            ack.kind
        )));
    }
    let shutdown = stream.shutdown_handle()?;
    let reader = BufReader::new(stream);
    Ok(Subscription {
        ack,
        reader,
        shutdown,
    })
}

fn write_request(
    stream: &mut dyn ApiStream,
    request_id: &str,
    method: &str,
    params: Value,
) -> Result<(), ApiError> {
    let envelope = json!({
        "id": request_id,
        "method": method,
        "params": params,
    });
    let mut request_line = serde_json::to_vec(&envelope)
        .map_err(|error| ApiError::Malformed(format!("request could not be encoded: {error}")))?;
    request_line.push(b'\n');
    stream
        .write_all(&request_line)
        .map_err(|error| ApiError::Transport(format!("request could not be written: {error}")))
}

fn read_response(reader: &mut dyn BufRead) -> Result<ResponseEnvelope, ApiError> {
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|error| ApiError::Transport(format!("response could not be read: {error}")))?;
    if line.trim().is_empty() {
        return Err(ApiError::Transport("response was empty".to_owned()));
    }
    decode_response(&line)
}

fn decode_response(line: &str) -> Result<ResponseEnvelope, ApiError> {
    serde_json::from_str(line)
        .map_err(|error| ApiError::Malformed(format!("response was not valid JSON: {error}")))
}

fn response_result(response: ResponseEnvelope, request_id: &str) -> Result<Value, ApiError> {
    if response.id != request_id {
        return Err(ApiError::Malformed(format!(
            "response id {:?} does not match request id {request_id:?}",
            response.id
        )));
    }
    match (response.result, response.error) {
        (Some(result), None) => Ok(result),
        (None, Some(error)) => Err(ApiError::Remote {
            code: error.code,
            message: error.message,
        }),
        (Some(_), Some(_)) => Err(ApiError::Malformed(
            "response contains both result and error".to_owned(),
        )),
        (None, None) => Err(ApiError::Malformed(
            "response contains neither result nor error".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    use hide_platform::ipc::LocalListener;

    use super::*;

    struct FixtureShutdown;

    impl ConnectionShutdown for FixtureShutdown {
        fn shutdown(&self) {}
    }

    struct FixtureStream {
        incoming: Cursor<Vec<u8>>,
        outgoing: Arc<Mutex<Vec<u8>>>,
    }

    impl Read for FixtureStream {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.incoming.read(buffer)
        }
    }

    impl Write for FixtureStream {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.outgoing.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl ApiStream for FixtureStream {
        fn set_read_timeout(&self, _timeout: Option<Duration>) -> Result<(), ApiError> {
            Ok(())
        }

        fn set_write_timeout(&self, _timeout: Option<Duration>) -> Result<(), ApiError> {
            Ok(())
        }

        fn read_line_with_timeout(&mut self, _timeout: Duration) -> Result<String, ApiError> {
            let mut line = String::new();
            BufReader::new(self)
                .read_line(&mut line)
                .map_err(|error| ApiError::Transport(error.to_string()))?;
            Ok(line)
        }

        fn shutdown_handle(&self) -> Result<Box<dyn ConnectionShutdown>, ApiError> {
            Ok(Box::new(FixtureShutdown))
        }
    }

    struct FixtureConnector {
        incoming: Vec<u8>,
        outgoing: Arc<Mutex<Vec<u8>>>,
    }

    impl ApiConnector for FixtureConnector {
        fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
            Ok(Box::new(FixtureStream {
                incoming: Cursor::new(self.incoming.clone()),
                outgoing: Arc::clone(&self.outgoing),
            }))
        }
    }

    #[test]
    fn an_expired_small_request_does_not_connect_or_write() {
        struct Unreachable;
        impl ApiConnector for Unreachable {
            fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
                panic!("expired work must not connect");
            }
        }
        let answer = request_small_response_until(
            &Unreachable,
            "agent.get",
            json!({"target":"w1:p1"}),
            Instant::now(),
        );
        assert!(matches!(answer, Err(ApiError::Transport(reason)) if reason.contains("deadline")));
    }

    #[test]
    fn a_strict_request_refuses_an_unbounded_connector_without_using_it() {
        struct Unbounded;
        impl ApiConnector for Unbounded {
            fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
                panic!("strict request must not call unbounded connect");
            }
        }
        let answer = request_small_response_strict_until(
            &Unbounded,
            "agent.get",
            json!({"target":"pane"}),
            Instant::now() + Duration::from_secs(1),
        );
        assert!(
            matches!(answer, Err(ApiError::Transport(reason)) if reason.contains("connection deadline"))
        );
    }

    #[test]
    fn a_strict_request_passes_its_original_deadline_and_does_not_write_after_expiry() {
        struct Delayed {
            deadline: Instant,
            outgoing: Arc<Mutex<Vec<u8>>>,
        }
        impl ApiConnector for Delayed {
            fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
                panic!("strict request must use connect_until");
            }
            fn connect_until(&self, deadline: Instant) -> Result<Box<dyn ApiStream>, ApiError> {
                assert_eq!(deadline, self.deadline);
                let (_hold, release) = std::sync::mpsc::channel::<()>();
                let _ = release.recv_timeout(deadline.saturating_duration_since(Instant::now()));
                Ok(Box::new(FixtureStream {
                    incoming: Cursor::new(Vec::new()),
                    outgoing: Arc::clone(&self.outgoing),
                }))
            }
        }
        let deadline = Instant::now() + Duration::from_millis(20);
        let outgoing = Arc::new(Mutex::new(Vec::new()));
        let answer = request_small_response_strict_until(
            &Delayed {
                deadline,
                outgoing: Arc::clone(&outgoing),
            },
            "agent.get",
            json!({"target":"pane"}),
            deadline,
        );
        assert!(matches!(answer, Err(ApiError::Transport(reason)) if reason.contains("deadline")));
        assert!(outgoing.lock().unwrap().is_empty());
    }

    #[test]
    fn a_small_request_expiring_during_connect_is_never_written() {
        struct SlowConnect {
            deadline: Instant,
            outgoing: Arc<Mutex<Vec<u8>>>,
        }
        impl ApiConnector for SlowConnect {
            fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
                // The transport is the delayed boundary under test. It
                // returns only when the supplied caller budget is spent.
                let (_hold, release) = std::sync::mpsc::channel::<()>();
                let _ =
                    release.recv_timeout(self.deadline.saturating_duration_since(Instant::now()));
                Ok(Box::new(FixtureStream {
                    incoming: Cursor::new(Vec::new()),
                    outgoing: Arc::clone(&self.outgoing),
                }))
            }
        }
        let deadline = Instant::now() + Duration::from_millis(20);
        let outgoing = Arc::new(Mutex::new(Vec::new()));
        let answer = request_small_response_until(
            &SlowConnect {
                deadline,
                outgoing: Arc::clone(&outgoing),
            },
            "agent.get",
            json!({"target":"w1:p1"}),
            deadline,
        );
        assert!(matches!(answer, Err(ApiError::Transport(reason)) if reason.contains("deadline")));
        assert!(outgoing.lock().unwrap().is_empty());
    }

    /// Reads the request line a fake Herdr was sent, leaving the stream
    /// usable for the answer.
    fn read_request(stream: &mut LocalStream) -> String {
        let mut request = String::new();
        BufReader::new(&mut *stream)
            .read_line(&mut request)
            .expect("read request");
        request
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // the sleep is the subject of the test: a fake peer or backend that is slow on purpose
    fn a_request_its_peer_never_answers_has_an_unknown_reply() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("peer.sock");
        let listener = LocalListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            read_request(&mut stream);
            std::thread::sleep(Duration::from_millis(400));
        });
        let answer = request_with_connector(
            &LocalSocketConnector::new(&socket),
            "worktree.create",
            json!({}),
            Duration::from_millis(100),
        )
        .expect_err("the peer never answers");
        assert!(answer.reply_unknown(), "{answer}");
        server.join().unwrap();
        // A socket nothing listens on did nothing, so it is not unknown.
        let absent = request_with_connector(
            &LocalSocketConnector::new(root.path().join("absent.sock")),
            "worktree.create",
            json!({}),
            Duration::from_millis(100),
        )
        .expect_err("no server is running");
        assert!(!absent.reply_unknown(), "{absent}");
        assert!(
            !ApiError::Remote {
                code: "x".into(),
                message: "y".into()
            }
            .reply_unknown()
        );
    }

    #[test]
    #[allow(clippy::disallowed_methods)] // the sleep is the subject of the test: a fake peer or backend that is slow on purpose
    fn small_response_rejects_a_peer_that_never_finishes_its_frame() {
        let root = tempfile::tempdir().unwrap();
        let socket = root.path().join("peer.sock");
        let listener = LocalListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().unwrap();
            read_request(&mut stream);
            // Keep making progress, so a per-read timeout would never fire.
            while stream.write_all(b" ").is_ok() {
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let started = std::time::Instant::now();
        let result = request_small_response(
            &LocalSocketConnector::new(&socket),
            "pane.process_info",
            json!({"pane_id":"w1:p1"}),
            Duration::from_millis(50),
        );
        assert!(
            matches!(result, Err(ApiError::Transport(message)) if message.contains("timed out"))
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }

    #[test]
    fn request_codec_accepts_a_transport_neutral_stream() {
        let outgoing = Arc::new(Mutex::new(Vec::new()));
        let connector = FixtureConnector {
            incoming: format!(
                "{}\n",
                json!({
                    "id": "herdr-core:pane.focus",
                    "result": {"type": "pane_focused", "pane_id": "w1:p2"}
                })
            )
            .into_bytes(),
            outgoing: Arc::clone(&outgoing),
        };

        let result = request_with_connector(
            &connector,
            "pane.focus",
            json!({"pane_id": "w1:p2"}),
            Duration::from_secs(1),
        )
        .expect("request succeeds over fixture transport");

        assert_eq!(result["pane_id"], "w1:p2");
        let written = String::from_utf8(outgoing.lock().unwrap().clone()).unwrap();
        let request: Value = serde_json::from_str(written.trim()).unwrap();
        assert_eq!(request["method"], "pane.focus");
        assert_eq!(request["params"], json!({"pane_id": "w1:p2"}));
    }

    #[test]
    fn request_rejects_a_response_for_another_request() {
        let root = tempfile::tempdir().expect("create socket directory");
        let socket_path = root.path().join("herdr.sock");
        let listener = LocalListener::bind(&socket_path).expect("bind fake socket");
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().expect("accept request");
            read_request(&mut stream);
            writeln!(stream, "{}", json!({"id": "somebody-else", "result": {}}))
                .expect("write response");
        });

        let error = request_with_timeout(
            &socket_path,
            "session.snapshot",
            json!({}),
            Duration::from_secs(1),
        )
        .expect_err("mismatched id must fail");
        assert!(matches!(error, ApiError::Malformed(_)));

        server.join().expect("fake server joins");
    }

    #[test]
    fn subscribe_keeps_replay_buffered_after_the_acknowledgement() {
        let root = tempfile::tempdir().expect("create socket directory");
        let socket_path = root.path().join("herdr.sock");
        let listener = LocalListener::bind(&socket_path).expect("bind fake socket");
        let server = std::thread::spawn(move || {
            let mut stream = listener.accept().expect("accept request");
            let request_line = read_request(&mut stream);
            let request: Value = serde_json::from_str(&request_line).expect("request JSON");
            assert_eq!(request["method"], "events.subscribe");
            assert_eq!(request["params"].get("after_sequence"), None);
            assert_eq!(
                request["params"]["subscriptions"],
                json!([{"type": "pane.focused"}])
            );
            writeln!(
                stream,
                "{}",
                json!({
                    "id": "herdr-core:events.subscribe",
                    "result": {
                        "type": "subscription_started"
                    }
                })
            )
            .expect("write ack");
            writeln!(
                stream,
                "{}",
                json!({
                    "event": "pane_focused",
                    "data": {
                        "type": "pane_focused",
                        "workspace_id": "w1",
                        "pane_id": "w1:p1"
                    }
                })
            )
            .expect("write replay");
        });

        let subscription = subscribe(
            &socket_path,
            json!({"subscriptions": [{"type": "pane.focused"}]}),
            Duration::from_secs(1),
        )
        .expect("subscribe");
        assert_eq!(subscription.ack.kind, "subscription_started");
        let (mut reader, shutdown) = subscription.into_parts();
        let mut replay = String::new();
        reader.read_line(&mut replay).expect("read replay");
        let replay: Value = serde_json::from_str(&replay).expect("replay JSON");
        assert_eq!(replay["event"], "pane_focused");
        drop(shutdown);

        server.join().expect("fake server joins");
    }
}
