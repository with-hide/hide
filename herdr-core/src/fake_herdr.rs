//! A fake Herdr socket server for tests.
//!
//! The core talks to Herdr one request per connection: connect, write one JSON
//! line, read one JSON line back. Every test that exercised that path used to
//! carry its own listener, accept loop and response writer, fifteen copies in
//! `live.rs` and `runtime.rs`, and the one copy that differed in a socket
//! detail (a non-blocking listener whose accepted streams inherited the flag
//! on macOS) failed under load for weeks before anyone found it. The socket is
//! handled here, once; a test supplies only the answers.
//!
//! The listeners that remain in `hide-herdr-client` and `session_sync.rs` are not
//! candidates: they test the transport itself, answering with a foreign id or
//! streaming subscription frames over one connection, which is exactly what
//! this fake does not let a test do.
//!
//! ```ignore
//! let herdr = FakeHerdr::start("pane-focus", |method, params| match method {
//!     "pane.focus" => json!({"type": "pane_info", "pane": {...}}),
//!     other => panic!("unexpected {other}"),
//! });
//! focus_pane(&herdr.connector(), "w1:p1");
//! assert_eq!(herdr.methods(), ["pane.focus"]);
//! ```
//!
//! Every response goes through [`wire::checked_response_fixture`], so an answer
//! that does not match the pinned generated schema fails the test at the fake
//! rather than being silently accepted by the client.
//!
//! The server stops when the value drops: the accept loop is woken by one
//! empty connection, the thread is joined, and a panic inside the responder
//! (an assertion on a request, an unexpected method) is re-raised on the test
//! thread so it fails the test instead of only dying in the log.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::wire;
use hide_herdr_client::LocalSocketConnector;
use hide_platform::ipc::{LocalListener, LocalStream};

static NEXT_FAKE_ID: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct FakeHerdr {
    root: PathBuf,
    socket_path: PathBuf,
    requests: Arc<Mutex<Vec<Value>>>,
    stopping: Arc<AtomicBool>,
    server: Option<JoinHandle<()>>,
}

impl FakeHerdr {
    /// Starts a server on its own socket. `respond` receives each request's
    /// method and params and returns the `result` the client should read.
    ///
    /// `name` labels the socket directory so a failure names the test that
    /// left it behind. A Unix socket path has a hard length limit, so the
    /// directory sits directly under `/tmp` however deep the test's other
    /// fixtures are; Windows names a pipe after the path and has no such
    /// limit.
    pub(crate) fn start(
        name: &str,
        mut respond: impl FnMut(&str, &Value) -> Value + Send + 'static,
    ) -> Self {
        Self::start_with_errors(name, move |method, params| Ok(respond(method, params)))
    }

    /// Like [`FakeHerdr::start`], but a request may be refused: `Err((code,
    /// message))` is written as Herdr's error envelope, which the client
    /// reads as `ApiError::Remote`.
    pub(crate) fn start_with_errors(
        name: &str,
        respond: impl FnMut(&str, &Value) -> Result<Value, (String, String)> + Send + 'static,
    ) -> Self {
        Self::start_with_errors_mode(name, respond, false)
    }

    fn start_with_errors_mode(
        name: &str,
        respond: impl FnMut(&str, &Value) -> Result<Value, (String, String)> + Send + 'static,
        concurrent: bool,
    ) -> Self {
        let temporary = if cfg!(unix) {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let root = temporary.join(format!(
            "herdr-core-fake-{name}-{}-{}",
            std::process::id(),
            NEXT_FAKE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).expect("create fake herdr socket directory");
        let socket_path = root.join("herdr.sock");
        let listener = LocalListener::bind(&socket_path).expect("bind fake herdr socket");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let server = {
            let requests = Arc::clone(&requests);
            let stopping = Arc::clone(&stopping);
            std::thread::Builder::new()
                .name(format!("fake-herdr-{name}"))
                .spawn(move || {
                    let respond = Mutex::new(respond);
                    std::thread::scope(|scope| {
                        loop {
                            let stream = listener.accept().expect("accept fake herdr request");
                            if stopping.load(Ordering::Acquire) {
                                return;
                            }
                            let requests = &requests;
                            let respond = &respond;
                            let answer = move || {
                                let mut line = String::new();
                                let mut stream = BufReader::new(stream);
                                stream
                                    .read_line(&mut line)
                                    .expect("read fake herdr request");
                                if line.trim().is_empty() {
                                    // A client that connected and hung up sent nothing to
                                    // answer; the next connection may still carry a request.
                                    return;
                                }
                                let request: Value =
                                    serde_json::from_str(&line).expect("fake herdr request JSON");
                                let method = request["method"]
                                    .as_str()
                                    .expect("fake herdr request names a method")
                                    .to_owned();
                                let response =
                                    match respond.lock().unwrap()(&method, &request["params"]) {
                                        Ok(result) => {
                                            wire::checked_response_fixture(&request["id"], result)
                                        }
                                        Err((code, message)) => serde_json::json!({
                                            "id": request["id"],
                                            "error": {"code": code, "message": message}
                                        }),
                                    };
                                requests.lock().unwrap().push(request.clone());
                                writeln!(stream.get_mut(), "{response}")
                                    .expect("write fake herdr response");
                            };
                            if concurrent {
                                scope.spawn(answer);
                            } else {
                                answer();
                            }
                        }
                    });
                })
                .expect("spawn fake herdr thread")
        };
        Self {
            root,
            socket_path,
            requests,
            stopping,
            server: Some(server),
        }
    }

    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub(crate) fn connector(&self) -> LocalSocketConnector {
        LocalSocketConnector::new(&self.socket_path)
    }

    /// Every request answered so far, in arrival order.
    pub(crate) fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }

    /// Every request answered so far as `(method, params)`, in arrival order.
    pub(crate) fn calls(&self) -> Vec<(String, Value)> {
        self.requests()
            .iter()
            .map(|request| {
                (
                    request["method"].as_str().unwrap_or_default().to_owned(),
                    request["params"].clone(),
                )
            })
            .collect()
    }

    /// Blocks until at least `count` requests have been answered, for a
    /// caller that fired its request from another thread and returned.
    #[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
    pub(crate) fn wait_for_requests(&self, count: usize, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while self.requests.lock().unwrap().len() < count {
            assert!(
                Instant::now() < deadline,
                "fake herdr answered {} request(s), expected {count} within {timeout:?}",
                self.requests.lock().unwrap().len()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// The method of every request answered so far, in arrival order.
    pub(crate) fn methods(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|request| request["method"].as_str().unwrap_or_default().to_owned())
            .collect()
    }
}

impl Drop for FakeHerdr {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        // One connection wakes the blocking accept so the loop sees the flag.
        let _ = LocalStream::connect(&self.socket_path);
        let outcome = self.server.take().map(JoinHandle::join);
        let _ = std::fs::remove_dir_all(&self.root);
        if matches!(outcome, Some(Err(_))) && !std::thread::panicking() {
            panic!("the fake herdr responder panicked; its message is above");
        }
    }
}
