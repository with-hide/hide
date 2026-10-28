//! What a core reaches on a node's machine through the node's link (PRD
//! core-host-node-remote-core D-18, D-19, B15; PRD wsl-device B9): each
//! stream the core opens with `link_open` is one local connection to the end
//! it names, its bytes carried by the link both ways. On a node that dialed
//! its core, the node's own Herdr socket is one end and the browser relay of
//! the node's daemon, which carries a caller's CDP to this machine's desktop
//! window, another. On every node, a server on this machine's loopback is
//! the third, which a Browser View of the core's machine shows. Nothing on
//! the core's machine listens for any of them; the link is the only way in,
//! and it ends every stream when it ends.
//!
//! A stream is a byte pipe and nothing here reads what it carries. Each end
//! has its own cap ([`LinkEnd::cap`]); the next open is refused, and the
//! refusal goes to the core, which logs it with the node.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};
use std::thread::Scope;
use std::time::Duration;

use base64::Engine as _;
use hide_node_link::panes::{MAX_CHUNK, NodeEvent};
use hide_node_link::protocol::LinkEnd;
use hide_platform::ipc::{LocalStream, ShutdownHandle};
use serde_json::Value;

use crate::error::{ErrorCode, HostError, HostResult};
use crate::panes::write_event;

/// How long one chunk's write to a stream's end may take before the stream
/// is ended.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long the node's daemon has to take a browser relay connection, and
/// a loopback server a page stream.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// The browser gateway of the desktop window on a node that dialed its
/// core, as the node's daemon holds its registration. The core decides the
/// scope; the gateway's capability URLs never leave this machine except as
/// the core's answer to a caller on it.
pub trait BrowserGateway: Send + Sync {
    /// The gateway's capability for `scope` (`{"workspace", "area_id",
    /// "display_id"?}`), or with `relay` a one-shot `relay_url` on the
    /// daemon. A refusal is one of the browser control reasons.
    fn capability(&self, scope: &Value, relay: bool) -> Result<Value, String>;
    /// The loopback address of the daemon's browser relay.
    fn relay_address(&self) -> SocketAddr;
}

/// One end's connection.
enum Connection {
    Local(LocalStream),
    Tcp(TcpStream),
}

impl Connection {
    fn writer(&self) -> io::Result<Box<dyn Write + Send>> {
        Ok(match self {
            Self::Local(stream) => Box::new(stream.duplicate()),
            Self::Tcp(stream) => Box::new(stream.try_clone()?),
        })
    }

    fn reader(&self) -> io::Result<Box<dyn Read + Send>> {
        Ok(match self {
            Self::Local(stream) => Box::new(stream.duplicate()),
            Self::Tcp(stream) => Box::new(stream.try_clone()?),
        })
    }

    fn closer(&self) -> io::Result<Closer> {
        Ok(match self {
            Self::Local(stream) => Closer::Local(stream.shutdown_handle()),
            Self::Tcp(stream) => Closer::Tcp(stream.try_clone()?),
        })
    }
}

enum Closer {
    Local(ShutdownHandle),
    Tcp(TcpStream),
}

impl Closer {
    fn close(&self) {
        match self {
            Self::Local(handle) => handle.shutdown(),
            Self::Tcp(stream) => {
                let _ = stream.shutdown(Shutdown::Both);
            }
        }
    }
}

/// One chunk for a stream's writer, and where it answers how the write
/// ended.
type Chunk = (Vec<u8>, mpsc::Sender<io::Result<()>>);

struct Open {
    end: LinkEnd,
    /// The stream's writer thread takes one chunk at a time; holding the
    /// lock is a write's turn on the stream.
    chunks: Mutex<mpsc::Sender<Chunk>>,
    closer: Closer,
}

/// The link streams of one node link.
pub struct LinkBridge<'a> {
    herdr: Option<PathBuf>,
    browser: Option<&'a dyn BrowserGateway>,
    streams: Mutex<HashMap<u64, std::sync::Arc<Open>>>,
    write_timeout: Duration,
}

impl<'a> LinkBridge<'a> {
    /// A bridge to this node's own Herdr at `herdr` and its daemon's
    /// browser relay, whichever it has.
    pub fn new(herdr: Option<PathBuf>, browser: Option<&'a dyn BrowserGateway>) -> Self {
        Self {
            herdr,
            browser,
            streams: Mutex::new(HashMap::new()),
            write_timeout: WRITE_TIMEOUT,
        }
    }

    /// The socket of this node's own Herdr, the one server it bridges.
    pub fn herdr_socket(&self) -> Option<&Path> {
        self.herdr.as_deref()
    }

    /// The browser gateway of this node's desktop window.
    pub fn browser(&self) -> Option<&'a dyn BrowserGateway> {
        self.browser
    }

    fn connect(&self, end: LinkEnd) -> HostResult<Connection> {
        match end {
            LinkEnd::Herdr => {
                let socket = self.herdr.as_deref().ok_or_else(|| unreached(end))?;
                let connection = LocalStream::connect(socket).map_err(|error| {
                    let code = if error.kind() == io::ErrorKind::NotFound {
                        ErrorCode::NotFound
                    } else {
                        ErrorCode::Io
                    };
                    HostError::new(code, format!("The node's Herdr did not answer: {error}"))
                })?;
                Ok(Connection::Local(connection))
            }
            LinkEnd::BrowserRelay => {
                let browser = self.browser.ok_or_else(|| unreached(end))?;
                let connection =
                    TcpStream::connect_timeout(&browser.relay_address(), CONNECT_TIMEOUT).map_err(
                        |error| {
                            HostError::new(
                                ErrorCode::Io,
                                format!("The node's browser relay did not answer: {error}"),
                            )
                        },
                    )?;
                Ok(Connection::Tcp(connection))
            }
            LinkEnd::Loopback { address } => {
                // A page stream reaches this machine's own loopback and
                // nothing past it, whatever the core asked.
                if !address.ip().is_loopback() || address.port() == 0 {
                    return Err(HostError::new(
                        ErrorCode::InvalidRequest,
                        "A page stream reaches only a port on this machine's loopback",
                    ));
                }
                let connection =
                    TcpStream::connect_timeout(&address, CONNECT_TIMEOUT).map_err(|error| {
                        HostError::new(
                            ErrorCode::Io,
                            format!("The page server on this machine did not answer: {error}"),
                        )
                    })?;
                Ok(Connection::Tcp(connection))
            }
        }
    }

    /// Connects `stream` to `end` and reads it on `scope` until either side
    /// ends it.
    pub fn open<'scope, 'env, W: Write + Send>(
        &'scope self,
        scope: &'scope Scope<'scope, 'env>,
        output: &'scope Mutex<W>,
        stream: u64,
        end: LinkEnd,
    ) -> HostResult<Value> {
        let admitted = |streams: &HashMap<u64, std::sync::Arc<Open>>| {
            if streams.contains_key(&stream) {
                return Err(HostError::new(
                    ErrorCode::InvalidRequest,
                    "A link stream with this id is still open",
                ));
            }
            let cap = end.cap();
            if streams
                .values()
                .filter(|open| open.end.shares_cap(end))
                .count()
                >= cap
            {
                return Err(HostError::new(
                    ErrorCode::Busy,
                    format!("The node already has {cap} {} streams open", name(end)),
                ));
            }
            Ok(())
        };
        admitted(&lock(&self.streams))?;
        // Connected outside the table's lock, so an end slow to accept
        // holds only this open, never another stream's write or close.
        let connection = self.connect(end)?;
        let mut reader = connection.reader().map_err(io_error)?;
        let mut writer = connection.writer().map_err(io_error)?;
        let (chunks, waiting) = mpsc::channel::<Chunk>();
        let open = std::sync::Arc::new(Open {
            end,
            chunks: Mutex::new(chunks),
            closer: connection.closer().map_err(io_error)?,
        });
        {
            let mut streams = lock(&self.streams);
            if let Err(error) = admitted(&streams) {
                drop(streams);
                open.closer.close();
                return Err(error);
            }
            streams.insert(stream, std::sync::Arc::clone(&open));
        }
        // An end that stops reading holds this thread, never a worker of
        // the link's control lane: `write` waits for it only until its
        // deadline, and the stream's close then ends the held write, on
        // every system (`hide_platform::ipc`, `ShutdownHandle`). The thread
        // ends when the stream does and its last chunk sender goes.
        scope.spawn(move || {
            for (bytes, answer) in waiting {
                let written = writer.write_all(&bytes);
                let failed = written.is_err();
                let _ = answer.send(written);
                if failed {
                    break;
                }
            }
        });
        scope.spawn(move || {
            let mut buffer = vec![0_u8; MAX_CHUNK];
            let reason = loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break "end_closed",
                    Ok(read) => {
                        let data =
                            base64::engine::general_purpose::STANDARD.encode(&buffer[..read]);
                        if write_event(output, &NodeEvent::LinkData { stream, data }).is_err() {
                            break "link_closed";
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                    Err(_) => break "end_read_failed",
                }
            };
            // A stream the core closed is already gone from the map, and the
            // core heard nothing more of it on purpose.
            if lock(&self.streams).remove(&stream).is_some() {
                let _ = write_event(
                    output,
                    &NodeEvent::LinkClosed {
                        stream,
                        reason: reason.to_owned(),
                    },
                );
            }
        });
        Ok(Value::Null)
    }

    /// Writes base64 `data` to `stream`'s end, waiting at most the write
    /// deadline; an end that takes no more by then, or fails the write,
    /// ends the stream.
    pub fn write(&self, stream: u64, data: &str) -> HostResult<Value> {
        if data.len() > MAX_CHUNK.div_ceil(3) * 4 {
            return Err(HostError::new(
                ErrorCode::TooLarge,
                "A link chunk is larger than the link carries",
            ));
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|_| HostError::new(ErrorCode::InvalidRequest, "A link chunk is not base64"))?;
        let open = lock(&self.streams)
            .get(&stream)
            .cloned()
            .ok_or_else(ended)?;
        let written = {
            let chunks = lock(&open.chunks);
            let (answer, answered) = mpsc::channel();
            if chunks.send((bytes, answer)).is_err() {
                return Err(ended());
            }
            match answered.recv_timeout(self.write_timeout) {
                Ok(written) => written.map_err(|error| error.to_string()),
                Err(mpsc::RecvTimeoutError::Timeout) => Err(format!(
                    "The node's {} took no data for {} s",
                    name(open.end),
                    self.write_timeout.as_secs_f32()
                )),
                Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ended()),
            }
        };
        written.map_err(|error| {
            self.close(stream);
            HostError::new(ErrorCode::Io, error)
        })?;
        Ok(Value::Null)
    }

    /// Ends `stream` from the core's side. A close runs outside the
    /// streams' lock: on Windows it cancels the end's pending I/O and may
    /// wait on it, and no other stream's write or the link's control lane
    /// waits behind that.
    pub fn close(&self, stream: u64) {
        let open = lock(&self.streams).remove(&stream);
        if let Some(open) = open {
            open.closer.close();
        }
    }

    /// The link is gone: every stream ends, and its reader with it.
    pub fn stop(&self) {
        let open: Vec<_> = lock(&self.streams).drain().map(|(_, open)| open).collect();
        for open in open {
            open.closer.close();
        }
    }
}

fn name(end: LinkEnd) -> &'static str {
    match end {
        LinkEnd::Herdr => "Herdr",
        LinkEnd::BrowserRelay => "browser relay",
        LinkEnd::Loopback { .. } => "page",
    }
}

/// Why a node refuses a stream to an end it does not bridge: a device its
/// core dialed is reached over SSH, never through the link, a node with no
/// desktop window registered has no browser relay, and a node answered in
/// its core's own process holds no link to carry a page.
pub fn unreached(end: LinkEnd) -> HostError {
    HostError::new(
        ErrorCode::Unsupported,
        match end {
            LinkEnd::Herdr => "This node's Herdr is not reached through its link",
            LinkEnd::BrowserRelay => "This node has no browser relay",
            LinkEnd::Loopback { .. } => "This node holds no link to carry a page",
        },
    )
}

fn ended() -> HostError {
    HostError::new(ErrorCode::NotFound, "The link stream has ended")
}

fn io_error(error: io::Error) -> HostError {
    HostError::new(ErrorCode::Io, error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use hide_platform::ipc::LocalListener;

    use super::*;

    /// A Herdr that takes the stream and never reads holds a write once its
    /// buffer is full. The write fails at the bridge's deadline instead of
    /// holding its caller (a worker of the link's control lane), the stream
    /// ends, and this holds on every system: a Windows pipe takes no write
    /// timeout, so the deadline cannot come from the stream.
    #[test]
    fn a_write_to_an_end_that_stops_reading_ends_the_stream_at_its_deadline() {
        let folder = tempfile::Builder::new().prefix("lb").tempdir().unwrap();
        let socket = folder.path().join("h.sock");
        let listener = LocalListener::bind(&socket).unwrap();
        let (release, released) = mpsc::channel::<()>();
        let herdr = std::thread::spawn(move || {
            let stream = listener.accept().unwrap();
            let _ = released.recv_timeout(Duration::from_secs(60));
            drop(stream);
        });
        let mut bridge = LinkBridge::new(Some(socket), None);
        bridge.write_timeout = Duration::from_millis(500);
        let output = Mutex::new(Vec::<u8>::new());
        let chunk = base64::engine::general_purpose::STANDARD.encode(vec![7_u8; MAX_CHUNK]);
        std::thread::scope(|scope| {
            bridge.open(scope, &output, 1, LinkEnd::Herdr).unwrap();
            // No buffer on any system holds a gigabyte of a silent peer's.
            let mut refused = None;
            for _ in 0..(1 << 30) / MAX_CHUNK {
                let started = Instant::now();
                if let Err(error) = bridge.write(1, &chunk) {
                    refused = Some((error, started.elapsed()));
                    break;
                }
            }
            let (error, waited) = refused.expect("the silent end never held a write");
            assert_eq!(error.code, ErrorCode::Io, "{error:?}");
            assert!(
                waited < Duration::from_secs(3),
                "the write held its caller {waited:?}"
            );
            let after = bridge.write(1, &chunk).unwrap_err();
            assert_eq!(after.code, ErrorCode::NotFound, "{after:?}");
            bridge.stop();
        });
        drop(release);
        herdr.join().unwrap();
    }
}
