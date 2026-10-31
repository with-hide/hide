//! JSON-RPC 2.0 over a child's standard streams, one JSON value per line, as
//! ACP v1 frames it.
//!
//! The reader thread owns the child's stdout: it answers the requests this
//! side sent, either to a caller waiting on [`Connection::call`] or as an
//! [`Incoming::Reply`] carrying the tag [`Connection::send`] was given, and
//! hands every request and notification the agent sends to the sink. Writes
//! come from the one owner of the connection.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use serde_json::{Value, json};

/// The longest line the agent may send. A `session/load` replays a whole
/// conversation as separate updates, so no single message needs more; a
/// longer one is a broken stream, and the connection ends.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;
/// Requests this side may have unanswered at once. Desk sends one prompt and
/// a few control requests at a time.
pub const MAX_PENDING: usize = 16;

/// A JSON-RPC error the agent answered with.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
}

/// Why a request got no result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CallError {
    /// The agent answered with an error.
    Rejected(RpcError),
    /// No answer came within the deadline; the request may still run.
    Unanswered,
    /// The stream ended, or could not be written.
    Closed(String),
    /// [`MAX_PENDING`] requests are already out.
    Busy,
}

impl std::fmt::Display for CallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rejected(error) => write!(f, "rejected {}: {}", error.code, error.message),
            Self::Unanswered => f.write_str("unanswered"),
            Self::Closed(why) => write!(f, "closed: {why}"),
            Self::Busy => f.write_str("busy"),
        }
    }
}

/// What the agent sent that no caller is waiting for.
#[derive(Debug)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// A request the agent waits on; [`Connection::respond`] answers it.
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The answer to a request sent with [`Connection::send`].
    Reply {
        tag: u64,
        result: Result<Value, CallError>,
    },
    /// The stream ended; nothing more comes.
    Closed {
        why: String,
    },
}

pub type Sink = Arc<dyn Fn(Incoming) + Send + Sync>;

enum Waiter {
    Call(mpsc::Sender<Result<Value, CallError>>),
    Tagged(u64),
}

struct Shared {
    pending: Mutex<HashMap<u64, Waiter>>,
    closed: Mutex<Option<String>>,
}

pub struct Connection {
    writer: Mutex<Box<dyn Write + Send>>,
    shared: Arc<Shared>,
    next_id: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Connection {
    /// Starts reading `reader` on a thread of its own. The returned handle
    /// ends with the stream: the thread exits on EOF or a read error.
    pub fn start(
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        sink: Sink,
    ) -> std::io::Result<(Arc<Self>, std::thread::JoinHandle<()>)> {
        let shared = Arc::new(Shared {
            pending: Mutex::new(HashMap::new()),
            closed: Mutex::new(None),
        });
        let connection = Arc::new(Self {
            writer: Mutex::new(Box::new(writer)),
            shared: shared.clone(),
            next_id: AtomicU64::new(1),
        });
        let thread = std::thread::Builder::new()
            .name("hide-desk-rpc".into())
            .spawn(move || read_loop(BufReader::new(reader), &shared, &sink))?;
        Ok((connection, thread))
    }

    /// Sends `method` and waits for its answer until `timeout` passes.
    pub fn call(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, CallError> {
        let (sender, receiver) = mpsc::channel();
        let id = self.register(Waiter::Call(sender))?;
        if let Err(error) =
            self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
        {
            lock(&self.shared.pending).remove(&id);
            return Err(error);
        }
        match receiver.recv_timeout(timeout) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                lock(&self.shared.pending).remove(&id);
                Err(CallError::Unanswered)
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(self.closed_error()),
        }
    }

    /// Sends `method` without waiting: its answer reaches the sink as an
    /// [`Incoming::Reply`] with `tag`, or a closed reply when the stream
    /// ends first.
    pub fn send(&self, method: &str, params: Value, tag: u64) -> Result<(), CallError> {
        let id = self.register(Waiter::Tagged(tag))?;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .inspect_err(|_| {
                lock(&self.shared.pending).remove(&id);
            })
    }

    pub fn notify(&self, method: &str, params: Value) -> Result<(), CallError> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// Answers a request the agent sent.
    pub fn respond(&self, id: &Value, result: Value) -> Result<(), CallError> {
        self.write(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    /// Refuses a request the agent sent.
    pub fn respond_error(&self, id: &Value, code: i64, message: &str) -> Result<(), CallError> {
        self.write(
            &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
        )
    }

    fn register(&self, waiter: Waiter) -> Result<u64, CallError> {
        if let Some(why) = lock(&self.shared.closed).clone() {
            return Err(CallError::Closed(why));
        }
        let mut pending = lock(&self.shared.pending);
        if pending.len() >= MAX_PENDING {
            return Err(CallError::Busy);
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        pending.insert(id, waiter);
        Ok(id)
    }

    fn write(&self, message: &Value) -> Result<(), CallError> {
        let mut line = serde_json::to_vec(message)
            .map_err(|error| CallError::Closed(format!("encode: {error}")))?;
        line.push(b'\n');
        let mut writer = lock(&self.writer);
        writer
            .write_all(&line)
            .and_then(|()| writer.flush())
            .map_err(|error| CallError::Closed(format!("write: {}", error.kind())))
    }

    fn closed_error(&self) -> CallError {
        CallError::Closed(
            lock(&self.shared.closed)
                .clone()
                .unwrap_or_else(|| "stream ended".into()),
        )
    }

    /// Whether the stream has ended.
    pub fn is_closed(&self) -> bool {
        lock(&self.shared.closed).is_some()
    }
}

/// Reads one line of at most [`MAX_LINE_BYTES`], without its newline.
/// `Ok(None)` is the end of the stream.
fn read_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> std::io::Result<Option<()>> {
    line.clear();
    loop {
        let available = match reader.fill_buf() {
            Ok(available) => available,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if available.is_empty() {
            return Ok((!line.is_empty()).then_some(()));
        }
        let (taken, done) = match memchr(available) {
            Some(at) => (at + 1, true),
            None => (available.len(), false),
        };
        if line.len() + taken > MAX_LINE_BYTES + 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line_too_long",
            ));
        }
        line.extend_from_slice(&available[..taken]);
        reader.consume(taken);
        if done {
            line.pop();
            return Ok(Some(()));
        }
    }
}

fn memchr(bytes: &[u8]) -> Option<usize> {
    bytes.iter().position(|byte| *byte == b'\n')
}

fn read_loop(mut reader: impl BufRead, shared: &Shared, sink: &Sink) {
    let mut line = Vec::new();
    let why = loop {
        match read_line(&mut reader, &mut line) {
            Ok(Some(())) => {}
            Ok(None) => break "eof".to_owned(),
            Err(error) => break format!("read: {error}"),
        }
        // A line that is not a JSON-RPC message (an adapter's stray print)
        // is not part of the protocol and is dropped.
        let Ok(Value::Object(mut message)) = serde_json::from_slice::<Value>(&line) else {
            continue;
        };
        let method = message
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let id = message.remove("id");
        match (method, id) {
            (Some(method), Some(id)) if !id.is_null() => sink(Incoming::Request {
                id,
                method,
                params: message.remove("params").unwrap_or(Value::Null),
            }),
            (Some(method), _) => sink(Incoming::Notification {
                method,
                params: message.remove("params").unwrap_or(Value::Null),
            }),
            (None, Some(id)) => {
                let Some(id) = id.as_u64() else { continue };
                let result = match message.remove("error") {
                    Some(error) => Err(CallError::Rejected(RpcError {
                        code: error.get("code").and_then(Value::as_i64).unwrap_or(0),
                        message: error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                    })),
                    None => Ok(message.remove("result").unwrap_or(Value::Null)),
                };
                match lock(&shared.pending).remove(&id) {
                    Some(Waiter::Call(sender)) => {
                        let _ = sender.send(result);
                    }
                    Some(Waiter::Tagged(tag)) => sink(Incoming::Reply { tag, result }),
                    None => {}
                }
            }
            (None, None) => {}
        }
    };
    *lock(&shared.closed) = Some(why.clone());
    let waiters: Vec<_> = lock(&shared.pending).drain().map(|(_, w)| w).collect();
    for waiter in waiters {
        match waiter {
            Waiter::Call(sender) => {
                let _ = sender.send(Err(CallError::Closed(why.clone())));
            }
            Waiter::Tagged(tag) => sink(Incoming::Reply {
                tag,
                result: Err(CallError::Closed(why.clone())),
            }),
        }
    }
    sink(Incoming::Closed { why });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn a_line_past_the_cap_ends_the_read_rather_than_growing() {
        let mut long = vec![b'x'; MAX_LINE_BYTES + 2];
        long.push(b'\n');
        let mut reader = BufReader::new(Cursor::new(long));
        let mut line = Vec::new();
        let error = read_line(&mut reader, &mut line).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_last_line_without_a_newline_is_still_read() {
        let mut reader = BufReader::new(Cursor::new(b"{\"a\":1}\n{\"b\":2}".to_vec()));
        let mut line = Vec::new();
        assert!(read_line(&mut reader, &mut line).unwrap().is_some());
        assert_eq!(line, b"{\"a\":1}");
        assert!(read_line(&mut reader, &mut line).unwrap().is_some());
        assert_eq!(line, b"{\"b\":2}");
        assert!(read_line(&mut reader, &mut line).unwrap().is_none());
    }

    #[test]
    fn answers_reach_their_caller_and_agent_requests_reach_the_sink() {
        let (to_us, ours) = std::sync::mpsc::channel::<Vec<u8>>();
        struct Pipe(mpsc::Receiver<Vec<u8>>, Vec<u8>);
        impl Read for Pipe {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.1.is_empty() {
                    match self.0.recv() {
                        Ok(bytes) => self.1 = bytes,
                        Err(_) => return Ok(0),
                    }
                }
                let n = buf.len().min(self.1.len());
                buf[..n].copy_from_slice(&self.1[..n]);
                self.1.drain(..n);
                Ok(n)
            }
        }
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let sink: Sink = Arc::new(move |incoming| {
            sink_seen.lock().unwrap().push(format!("{incoming:?}"));
        });
        let written = Arc::new(Mutex::new(Vec::<u8>::new()));
        struct Shared(Arc<Mutex<Vec<u8>>>);
        impl Write for Shared {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let (connection, thread) =
            Connection::start(Pipe(ours, Vec::new()), Shared(written.clone()), sink).unwrap();
        let caller = {
            let connection = connection.clone();
            std::thread::spawn(move || {
                connection.call("initialize", json!({}), Duration::from_secs(30))
            })
        };
        // The request went out as id 1; answer it, then send a request of
        // the agent's own.
        while !String::from_utf8_lossy(&written.lock().unwrap()).contains("\"initialize\"") {
            std::thread::yield_now();
        }
        to_us
            .send(b"not json\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n".to_vec())
            .unwrap();
        assert_eq!(caller.join().unwrap(), Ok(json!({"ok": true})));
        to_us
            .send(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"session/request_permission\",\"params\":{}}\n".to_vec())
            .unwrap();
        drop(to_us);
        thread.join().unwrap();
        let seen = seen.lock().unwrap();
        assert!(seen[0].contains("session/request_permission"), "{seen:?}");
        assert!(seen.last().unwrap().contains("Closed"), "{seen:?}");
        assert_eq!(
            connection.call("x", json!({}), Duration::from_secs(1)),
            Err(CallError::Closed("eof".into()))
        );
    }
}
