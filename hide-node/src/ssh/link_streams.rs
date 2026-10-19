//! What the core reaches on the machine of a node that dialed it, through
//! the node's link (PRD core-host-node-remote-core D-18, D-19, B15): every
//! connection is one stream inside the link, which the node bridges to the
//! end the core names (`hide_host::link_bridge`). The node's Herdr is one
//! end, taken as an [`ApiConnector`] by the session sync, controls,
//! doorbell, find, phone reply and agent sleep where they take a local
//! socket or an SSH device's channel; the browser relay of the node's
//! daemon is the other, which carries a caller's CDP to the node's desktop
//! window ([`RemoteHost::browser_relay_stream`]).
//!
//! A stream reads what the node sent, in order, from a bounded queue: one
//! the reader leaves past [`MAX_HERDR_PENDING`] chunks is ended as stalled,
//! and a link holds at most [`LinkEnd::cap`] streams to each end at once.
//! The link ending ends every stream on it. A stream a runtime thread
//! closes is told to the node by the link's one close worker, in order,
//! with at most [`MAX_CLOSE_NOTICES`] waiting.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use base64::Engine as _;
use hide_herdr_client::{ApiConnector, ApiError, ApiStream, ConnectionShutdown};
use hide_node_link::panes::MAX_CHUNK;
use hide_node_link::protocol::{Call, LinkEnd};
use serde_json::json;

use super::{RemoteHost, lock_recover};

/// Chunks one Herdr stream holds for its reader; past it the stream ends.
pub const MAX_HERDR_PENDING: usize = 64;
/// Chunks one browser relay stream holds: more than the 4 MiB a CDP
/// message may be, so a large answer to a caller that is slow to read is
/// held whole rather than ending the relay.
pub const MAX_BROWSER_PENDING: usize = 128;
/// How long an open, a write or a close may take on the link.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Close notices waiting for the link's close worker: every stream a link
/// can hold at once. A notice past it is dropped and logged; the node's
/// end of that stream closes with the link.
pub const MAX_CLOSE_NOTICES: usize =
    hide_node_link::panes::MAX_HERDR_STREAMS + hide_node_link::panes::MAX_BROWSER_STREAMS;

fn pending(end: LinkEnd) -> usize {
    match end {
        LinkEnd::Herdr => MAX_HERDR_PENDING,
        LinkEnd::BrowserRelay => MAX_BROWSER_PENDING,
    }
}

/// Writes at most one chunk of `buffer` to `stream`, waiting `timeout` for
/// the node to take it.
fn write_chunk(
    link: &RemoteHost,
    stream: u64,
    buffer: &[u8],
    timeout: Duration,
) -> io::Result<usize> {
    let taken = buffer.len().min(MAX_CHUNK);
    let data = base64::engine::general_purpose::STANDARD.encode(&buffer[..taken]);
    link.call(Call::LinkWrite { stream, data }, timeout)
        .map_err(|error| io::Error::new(io::ErrorKind::BrokenPipe, error.to_string()))?;
    Ok(taken)
}

enum Piece {
    Data(Vec<u8>),
    Ended(String),
}

/// The streams open on one link, by the id the core gave each. A
/// stream this side gave up (its reader fell behind, or the node sent what
/// is not base64) keeps its entry, with no sender, until its owner drops
/// it and the node is told to close it; only the node ending a stream
/// removes its entry unasked. So every stream the node holds is one the
/// cap counts, and none is left open on the node unseen.
#[derive(Default)]
pub(super) struct LinkStreams {
    open: Mutex<HashMap<u64, Entry>>,
    next: AtomicU64,
    /// The close worker's queue, once a stream closed; dropped with the
    /// link, which ends the worker.
    closes: Mutex<Option<mpsc::SyncSender<u64>>>,
}

struct Entry {
    end: LinkEnd,
    sender: Option<mpsc::SyncSender<Piece>>,
}

impl LinkStreams {
    /// Bytes the node read from `stream`'s end. A reader that has fallen
    /// its end's pending chunks behind loses its stream.
    pub(super) fn data(&self, target: &str, stream: u64, data: &str) {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else {
            self.give_up(stream, "the node sent a link chunk that is not base64");
            return;
        };
        let mut open = lock_recover(&self.open);
        let Some(entry) = open.get_mut(&stream) else {
            return;
        };
        let Some(sender) = &entry.sender else {
            return;
        };
        match sender.try_send(Piece::Data(bytes)) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                // The reader finds the queue's end after what it holds.
                entry.sender = None;
                let end = entry.end;
                drop(open);
                crate::diagnostic!(json!({
                    "component": "remote_host",
                    "kind": "link_stream.overflow",
                    "target": target,
                    "stream": stream,
                    "end": end,
                    "cap": pending(end),
                }));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                entry.sender = None;
            }
        }
    }

    /// This side stops reading `stream`: its reader reads `reason` as the
    /// end, and the entry stays until the reader's owner closes it.
    fn give_up(&self, stream: u64, reason: &str) {
        let mut open = lock_recover(&self.open);
        if let Some(entry) = open.get_mut(&stream)
            && let Some(sender) = entry.sender.take()
        {
            let _ = sender.try_send(Piece::Ended(reason.to_owned()));
        }
    }

    /// The node ended `stream`; nothing is left open there to close.
    pub(super) fn end(&self, stream: u64, reason: &str) {
        if let Some(Entry {
            sender: Some(sender),
            ..
        }) = lock_recover(&self.open).remove(&stream)
        {
            let _ = sender.try_send(Piece::Ended(reason.to_owned()));
        }
    }

    /// The end `stream` reaches, while it is open.
    pub(super) fn end_of(&self, stream: u64) -> Option<LinkEnd> {
        lock_recover(&self.open).get(&stream).map(|entry| entry.end)
    }

    /// The link ended: every stream reads its end, and the close worker
    /// ends.
    pub(super) fn end_all(&self) {
        lock_recover(&self.open).clear();
        lock_recover(&self.closes).take();
    }

    /// Queues the notice that `stream` closed for the link's close worker,
    /// starting it on the first. The worker holds the link only while it
    /// tells one notice, so the link's last clone still ends it.
    fn tell_closed(&self, link: &RemoteHost, stream: u64) {
        let mut closes = lock_recover(&self.closes);
        let queue = match closes.as_ref() {
            Some(queue) => queue,
            None => {
                let (queue, notices) = mpsc::sync_channel(MAX_CLOSE_NOTICES);
                let weak = Arc::downgrade(&link.inner);
                let started = std::thread::Builder::new()
                    .name("link-stream-close".into())
                    .spawn(move || tell_closes(&weak, &notices));
                if let Err(error) = started {
                    crate::diagnostic!(json!({
                        "component": "remote_host",
                        "kind": "link_stream.close_unsent",
                        "target": link.target(),
                        "stream": stream,
                        "reason": format!("the close worker could not start: {error}"),
                    }));
                    return;
                }
                closes.insert(queue)
            }
        };
        if let Err(error) = queue.try_send(stream) {
            let reason = match error {
                mpsc::TrySendError::Full(_) => "the close queue is full",
                mpsc::TrySendError::Disconnected(_) => "the close worker ended",
            };
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "link_stream.close_unsent",
                "target": link.target(),
                "stream": stream,
                "reason": reason,
                "cap": MAX_CLOSE_NOTICES,
            }));
        }
    }
}

/// Connects to the Herdr of the node at the other end of `link`.
#[derive(Clone)]
pub struct LinkHerdrConnector {
    link: RemoteHost,
}

impl std::fmt::Debug for LinkHerdrConnector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LinkHerdrConnector")
            .field("target", &self.link.target())
            .finish()
    }
}

impl RemoteHost {
    /// The Herdr of the node at the other end of this link, reached through
    /// it: a node that dialed its core (PRD core-host-node-remote-core D-18).
    pub fn herdr_connector(&self) -> LinkHerdrConnector {
        LinkHerdrConnector { link: self.clone() }
    }
}

impl ApiConnector for LinkHerdrConnector {
    fn connect(&self) -> Result<Box<dyn ApiStream>, ApiError> {
        match self.link.open_stream(LinkEnd::Herdr) {
            Ok(stream) => Ok(Box::new(stream)),
            Err(error) => {
                let message = format!("the node's Herdr could not be reached: {error}");
                Err(match &error {
                    OpenError::Refused(hide_node_link::LinkError::Refused(refusal))
                        if refusal.code == hide_node_link::ErrorCode::NotFound =>
                    {
                        ApiError::NotRunning(message)
                    }
                    _ => ApiError::Transport(message),
                })
            }
        }
    }
}

/// Why a link stream did not open.
#[derive(Debug)]
pub enum OpenError {
    /// The link has ended.
    Ended(String),
    /// The link already carries its cap of streams to this end.
    Cap(usize),
    /// The node refused it or never answered.
    Refused(hide_node_link::LinkError),
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ended(reason) => write!(formatter, "the node's link has ended ({reason})"),
            Self::Cap(cap) => write!(
                formatter,
                "the node's link already carries {cap} streams to this end"
            ),
            Self::Refused(error) => error.fmt(formatter),
        }
    }
}

impl RemoteHost {
    /// A stream to the browser relay of the node's daemon at the other end
    /// of this link (B15): its first bytes ask for a relay ticket the node
    /// handed out over this link.
    pub fn browser_relay_stream(&self) -> Result<LinkStream, OpenError> {
        self.open_stream(LinkEnd::BrowserRelay)
    }

    fn open_stream(&self, end: LinkEnd) -> Result<LinkStream, OpenError> {
        let streams = &self.inner.streams;
        if let Some(reason) = self.closed_reason() {
            return Err(OpenError::Ended(reason));
        }
        let stream = streams.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = mpsc::sync_channel(pending(end));
        {
            let mut open = lock_recover(&streams.open);
            let cap = end.cap();
            if open.values().filter(|entry| entry.end == end).count() >= cap {
                drop(open);
                crate::diagnostic!(json!({
                    "component": "remote_host",
                    "kind": "link_stream.cap_reached",
                    "target": self.target(),
                    "end": end,
                    "cap": cap,
                }));
                return Err(OpenError::Cap(cap));
            }
            open.insert(
                stream,
                Entry {
                    end,
                    sender: Some(sender),
                },
            );
        }
        if let Err(error) = self.call(Call::LinkOpen { stream, end }, CALL_TIMEOUT) {
            // An open whose answer never came may have opened the stream on
            // the node, so it is told to close; one refused or never sent
            // left nothing there.
            if matches!(error, hide_node_link::LinkError::Unknown(_)) {
                close_stream(self, stream);
            } else {
                lock_recover(&streams.open).remove(&stream);
            }
            return Err(OpenError::Refused(error));
        }
        // A browser relay stream carries a caller's CDP across the link: the
        // count a check of where CDP runs reads (B4). Herdr's are too many to
        // log one by one.
        if end == LinkEnd::BrowserRelay {
            crate::diagnostic!(json!({
                "component": "remote_host",
                "kind": "link_stream.opened",
                "target": self.target(),
                "end": end,
                "stream": stream,
            }));
        }
        Ok(LinkStream {
            link: self.clone(),
            stream,
            state: Arc::new(StreamState {
                receiver: Mutex::new(receiver),
                ended: Mutex::new(None),
            }),
            held: Vec::new(),
            read_timeout: Mutex::new(None),
            write_timeout: Mutex::new(None),
        })
    }
}

struct StreamState {
    receiver: Mutex<mpsc::Receiver<Piece>>,
    ended: Mutex<Option<String>>,
}

/// One stream of a linked node's link, to the end it was opened for.
pub struct LinkStream {
    link: RemoteHost,
    stream: u64,
    state: Arc<StreamState>,
    /// Bytes read from a chunk and not yet taken.
    held: Vec<u8>,
    read_timeout: Mutex<Option<Duration>>,
    write_timeout: Mutex<Option<Duration>>,
}

impl LinkStream {
    /// The next chunk, waiting at most until `deadline`; `None` at the end.
    fn next_chunk(&mut self, deadline: Option<Instant>) -> io::Result<Option<Vec<u8>>> {
        if self
            .state
            .ended
            .lock()
            .map(|ended| ended.is_some())
            .unwrap_or(true)
        {
            return Ok(None);
        }
        let receiver = lock_recover(&self.state.receiver);
        let piece = match deadline {
            None => receiver.recv().map_err(|_| None),
            Some(deadline) => receiver
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => Some(()),
                    mpsc::RecvTimeoutError::Disconnected => None,
                }),
        };
        drop(receiver);
        match piece {
            Ok(Piece::Data(bytes)) => Ok(Some(bytes)),
            Ok(Piece::Ended(reason)) => {
                *lock_recover(&self.state.ended) = Some(reason);
                Ok(None)
            }
            Err(Some(())) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the node sent nothing on the stream in time",
            )),
            Err(None) => {
                *lock_recover(&self.state.ended) = Some("the stream ended".to_owned());
                Ok(None)
            }
        }
    }
}

impl Read for LinkStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.held.is_empty() {
            let deadline = lock_recover(&self.read_timeout).map(|timeout| Instant::now() + timeout);
            match self.next_chunk(deadline)? {
                Some(bytes) => self.held = bytes,
                None => return Ok(0),
            }
        }
        let taken = self.held.len().min(buffer.len());
        buffer[..taken].copy_from_slice(&self.held[..taken]);
        self.held.drain(..taken);
        Ok(taken)
    }
}

impl Write for LinkStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let timeout = lock_recover(&self.write_timeout).unwrap_or(CALL_TIMEOUT);
        write_chunk(&self.link, self.stream, buffer, timeout)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl ApiStream for LinkStream {
    fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        *lock_recover(&self.read_timeout) = timeout;
        Ok(())
    }

    fn set_write_timeout(&self, timeout: Option<Duration>) -> Result<(), ApiError> {
        *lock_recover(&self.write_timeout) = timeout;
        Ok(())
    }

    fn read_line_with_timeout(&mut self, timeout: Duration) -> Result<String, ApiError> {
        let deadline = Instant::now() + timeout;
        let mut line = Vec::new();
        loop {
            if let Some(end) = self.held.iter().position(|byte| *byte == b'\n') {
                line.extend(self.held.drain(..=end));
                break;
            }
            line.append(&mut self.held);
            if line.len() > 64 * 1024 {
                return Err(ApiError::Malformed(
                    "subscription acknowledgement exceeds 64 KiB".to_owned(),
                ));
            }
            match self.next_chunk(Some(deadline)) {
                Ok(Some(bytes)) => self.held = bytes,
                Ok(None) => {
                    return Err(ApiError::Transport(
                        "subscription acknowledgement reached EOF".to_owned(),
                    ));
                }
                Err(_) => {
                    return Err(ApiError::Transport(
                        "subscription acknowledgement timed out".to_owned(),
                    ));
                }
            }
        }
        // The reader it becomes after the acknowledgement has no deadline.
        *lock_recover(&self.read_timeout) = None;
        String::from_utf8(line).map_err(|error| {
            ApiError::Malformed(format!(
                "subscription acknowledgement was not UTF-8: {error}"
            ))
        })
    }

    fn shutdown_handle(&self) -> Result<Box<dyn ConnectionShutdown>, ApiError> {
        Ok(Box::new(self.closer()))
    }
}

impl Drop for LinkStream {
    fn drop(&mut self) {
        close_stream(&self.link, self.stream);
    }
}

/// Ends one link stream from any thread.
pub struct StreamCloser {
    link: RemoteHost,
    stream: u64,
}

impl StreamCloser {
    /// Ends the stream: its reader reads the end, and the node is told.
    pub fn close(&self) {
        close_stream(&self.link, self.stream);
    }
}

impl ConnectionShutdown for StreamCloser {
    fn shutdown(&self) {
        self.close();
    }
}

/// Writes to one link stream from another thread than its reader's.
pub struct LinkWriter {
    link: RemoteHost,
    stream: u64,
}

impl Write for LinkWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        write_chunk(&self.link, self.stream, buffer, CALL_TIMEOUT)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl LinkStream {
    /// A writer for this stream that another thread holds, while this one
    /// reads; dropping the stream ends both.
    pub fn writer(&self) -> LinkWriter {
        LinkWriter {
            link: self.link.clone(),
            stream: self.stream,
        }
    }

    /// What ends this stream from another thread.
    pub fn closer(&self) -> StreamCloser {
        StreamCloser {
            link: self.link.clone(),
            stream: self.stream,
        }
    }
}

/// Ends `stream` on this side at once, so its reader reads the end, and
/// tells the node, which closes its connection to the stream's end. Telling
/// the node waits on the link: a plain thread tells it before it goes on, so
/// an open it makes next finds the node's slot free, and a runtime thread,
/// which must not wait, leaves it to the link's close worker.
fn close_stream(link: &RemoteHost, stream: u64) {
    let open = lock_recover(&link.inner.streams.open).remove(&stream);
    let Some(Entry { sender, .. }) = open else {
        return;
    };
    if let Some(sender) = sender {
        let _ = sender.try_send(Piece::Ended("closed by this side".to_owned()));
    }
    if link.closed_reason().is_some() {
        return;
    }
    if tokio::runtime::Handle::try_current().is_ok() {
        link.inner.streams.tell_closed(link, stream);
    } else {
        tell_close(link, stream);
    }
}

fn tell_close(link: &RemoteHost, stream: u64) {
    if let Err(error) = link.call(Call::LinkClose { stream }, CALL_TIMEOUT) {
        crate::diagnostic!(json!({
            "component": "remote_host",
            "kind": "link_stream.close_unanswered",
            "target": link.target(),
            "stream": stream,
            "error": error.to_string(),
        }));
    }
}

/// The link's close worker: tells the node each stream a runtime thread
/// closed, one at a time, until the link ends or its last clone is dropped.
fn tell_closes(link: &std::sync::Weak<super::Inner>, notices: &mpsc::Receiver<u64>) {
    while let Ok(stream) = notices.recv() {
        let Some(inner) = link.upgrade() else {
            return;
        };
        let link = RemoteHost { inner };
        if link.closed_reason().is_some() {
            return;
        }
        tell_close(&link, stream);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_node_link::panes::MAX_HERDR_STREAMS;
    use hide_platform::ipc::LocalListener;
    use std::io::{BufRead, BufReader};

    /// A Herdr that answers each line it reads with the same line, until
    /// its connection ends.
    fn echo_herdr(folder: &std::path::Path) -> std::path::PathBuf {
        let socket = folder.join("h.sock");
        let listener = LocalListener::bind(&socket).expect("a Herdr socket");
        std::thread::spawn(move || {
            while let Ok(connection) = listener.accept() {
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(connection.duplicate());
                    let mut writer = connection;
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|read| read > 0) {
                        if writer.write_all(line.as_bytes()).is_err() {
                            return;
                        }
                        line.clear();
                    }
                });
            }
        });
        socket
    }

    /// A link whose other end is a node serving `herdr_socket` as its own.
    fn linked_node(herdr_socket: std::path::PathBuf) -> RemoteHost {
        linked_node_with(herdr_socket, None)
    }

    /// A browser gateway whose relay echoes what it reads, and whose
    /// capability names the scope it was asked for.
    struct EchoGateway(std::net::SocketAddr);

    impl hide_host::link_bridge::BrowserGateway for EchoGateway {
        fn capability(
            &self,
            scope: &serde_json::Value,
            relay: bool,
        ) -> Result<serde_json::Value, String> {
            if scope["workspace"] == "refused" {
                return Err("browser_control_unavailable".to_owned());
            }
            Ok(json!({"scope": scope, "relay": relay}))
        }

        fn relay_address(&self) -> std::net::SocketAddr {
            self.0
        }
    }

    fn echo_relay() -> &'static EchoGateway {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a relay port");
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            while let Ok((connection, _)) = listener.accept() {
                std::thread::spawn(move || {
                    let mut reader = connection.try_clone().unwrap();
                    let mut writer = connection;
                    let _ = std::io::copy(&mut reader, &mut writer);
                });
            }
        });
        Box::leak(Box::new(EchoGateway(address)))
    }

    fn linked_node_with(
        herdr_socket: std::path::PathBuf,
        browser: Option<&'static dyn hide_host::link_bridge::BrowserGateway>,
    ) -> RemoteHost {
        let (core_end, node_end) = hide_platform::ipc::LocalStream::pair().expect("a pair");
        std::thread::spawn(move || {
            let input = BufReader::new(node_end.duplicate());
            let _ = hide_host::serve::serve_with(
                input,
                node_end,
                hide_host::serve::Services {
                    terminals: None,
                    herdr_socket: Some(herdr_socket),
                    heartbeat: false,
                    checkout_callers: false,
                    opened_roots: None,
                    worktree_facts: None,
                    browser,
                    kit: hide_host::serve::KitPlace::Standalone,
                },
            );
        });
        super::super::over_local_stream("inbound:test", core_end, None, Box::new(|_| {}))
            .expect("a local link")
    }

    /// The core reaches a linked node's Herdr through the link: a request
    /// written on a stream gets the node's Herdr's answer, line by line, and
    /// two streams do not share bytes (D-18).
    #[test]
    fn a_linked_node_s_herdr_answers_through_the_link() {
        let folder = tempfile::tempdir().unwrap();
        let link = linked_node(echo_herdr(folder.path()));
        let connector = link.herdr_connector();
        let mut first = connector.connect().expect("a first stream");
        let mut second = connector.connect().expect("a second stream");
        first.write_all(b"{\"id\":\"a\"}\n").unwrap();
        second.write_all(b"{\"id\":\"b\"}\n").unwrap();
        assert_eq!(
            first
                .read_line_with_timeout(Duration::from_secs(10))
                .unwrap(),
            "{\"id\":\"a\"}\n"
        );
        assert_eq!(
            second
                .read_line_with_timeout(Duration::from_secs(10))
                .unwrap(),
            "{\"id\":\"b\"}\n"
        );
        drop(first);
        // A read past its timeout says so rather than returning early.
        second
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let mut byte = [0_u8; 1];
        let error = second.read(&mut byte).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    /// At most `MAX_HERDR_STREAMS` streams are open on a link; the next is
    /// refused rather than queued, and one closing makes room (D-20).
    #[test]
    fn a_link_holds_at_most_its_cap_of_herdr_streams() {
        let folder = tempfile::tempdir().unwrap();
        let link = linked_node(echo_herdr(folder.path()));
        let connector = link.herdr_connector();
        let mut open: Vec<_> = (0..MAX_HERDR_STREAMS)
            .map(|_| connector.connect().expect("a stream under the cap"))
            .collect();
        let refused = connector.connect().err().expect("the stream past the cap");
        assert!(refused.to_string().contains("already carries"), "{refused}");
        open.pop();
        connector.connect().expect("room after a close");
    }

    /// A node that opens every stream at once and holds its answer to each
    /// close until the test lets it go; each close it reads is reported.
    fn node_holding_closes() -> (RemoteHost, mpsc::Receiver<u64>, mpsc::Sender<()>) {
        use hide_node_link::protocol::{Outcome, Request, Response};
        let (core_end, node_end) = hide_platform::ipc::LocalStream::pair().expect("a pair");
        let (closed, closes) = mpsc::channel();
        let (release, released) = mpsc::channel::<()>();
        let writer = Arc::new(Mutex::new(node_end.duplicate()));
        let answer = |writer: &Mutex<hide_platform::ipc::LocalStream>, id: u64| {
            let response = Response {
                id,
                outcome: Outcome::Ok(json!({})),
            };
            let mut line = serde_json::to_vec(&response).unwrap();
            line.push(b'\n');
            let _ = lock_recover(writer).write_all(&line);
        };
        let (held, holding) = mpsc::channel::<u64>();
        {
            let writer = Arc::clone(&writer);
            std::thread::spawn(move || {
                while let Ok(id) = holding.recv() {
                    if released.recv().is_err() {
                        return;
                    }
                    answer(&writer, id);
                }
            });
        }
        std::thread::spawn(move || {
            let mut reader = BufReader::new(node_end);
            let mut line = String::new();
            while reader.read_line(&mut line).is_ok_and(|read| read > 0) {
                let request: Request = serde_json::from_str(&line).unwrap();
                line.clear();
                match request.call {
                    Call::LinkClose { stream } => {
                        let _ = closed.send(stream);
                        let _ = held.send(request.id);
                    }
                    _ => answer(&writer, request.id),
                }
            }
        });
        let link =
            super::super::over_local_stream("inbound:test", core_end, None, Box::new(|_| {}))
                .expect("a local link");
        (link, closes, release)
    }

    /// Streams closed together on a runtime thread are told to the node one
    /// at a time by the link's one close worker, not by a thread each: while
    /// the node holds its answer to the first close, no second is sent, and
    /// every close arrives in turn once it answers (arch review item 4).
    #[test]
    fn closes_reach_the_node_one_at_a_time_from_one_worker() {
        let (link, closes, release) = node_holding_closes();
        let connector = link.herdr_connector();
        let streams: Vec<_> = (0..MAX_HERDR_STREAMS)
            .map(|_| connector.connect().expect("a stream"))
            .collect();
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async move { drop(streams) });
        let first = closes
            .recv_timeout(Duration::from_secs(10))
            .expect("the first close reaches the node");
        // A second close would be on its way at once had each its own
        // thread; with one worker none is until the first is answered.
        assert_eq!(
            closes.recv_timeout(Duration::from_millis(300)).ok(),
            None,
            "a second close was sent while the first waited"
        );
        let mut told = vec![first];
        for _ in 1..MAX_HERDR_STREAMS {
            release.send(()).unwrap();
            told.push(
                closes
                    .recv_timeout(Duration::from_secs(10))
                    .expect("the next close once the last was answered"),
            );
        }
        told.sort_unstable();
        assert_eq!(told, (1..=MAX_HERDR_STREAMS as u64).collect::<Vec<_>>());
    }

    /// A stream this side gives up, its reader fallen behind, is closed on
    /// the node once its owner drops it, so it takes none of the node's
    /// stream slots after: a full set of streams opens again (R2).
    #[test]
    fn a_stream_given_up_for_a_slow_reader_is_closed_on_the_node() {
        let folder = tempfile::tempdir().unwrap();
        let link = linked_node(echo_herdr(folder.path()));
        let connector = link.herdr_connector();
        let given_up = connector.connect().expect("a stream");
        // The node's chunks for it, past what its reader holds.
        let chunk = base64::engine::general_purpose::STANDARD.encode(b"x");
        for _ in 0..=MAX_HERDR_PENDING {
            link.inner.streams.data("inbound:test", 1, &chunk);
        }
        drop(given_up);
        let open: Vec<_> = (0..MAX_HERDR_STREAMS)
            .map(|index| {
                connector
                    .connect()
                    .unwrap_or_else(|error| panic!("stream {index} was refused: {error}"))
            })
            .collect();
        assert_eq!(open.len(), MAX_HERDR_STREAMS);
    }

    /// Every machine call slot held by a call that runs until stopped: a
    /// Herdr stream still opens, writes and reads at once, since link
    /// control has its own lane at both ends (R4).
    #[test]
    fn herdr_streams_do_not_wait_behind_machine_calls() {
        let folder = tempfile::tempdir().unwrap();
        let common = folder.path().join("common");
        std::fs::create_dir_all(common.join("refs/heads")).unwrap();
        let link = linked_node(echo_herdr(folder.path()));
        let (reported, reports) = mpsc::channel();
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let watches: Vec<_> = (0..crate::ssh::host::MAX_RUNNING)
            .map(|_| {
                let link = link.clone();
                let common = common.to_string_lossy().into_owned();
                let reported = reported.clone();
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    let _ = link.call_with_progress(
                        Call::GitWatch {
                            common_dirs: vec![common],
                        },
                        Duration::from_secs(60),
                        &mut |_| {
                            let _ = reported.send(());
                            !stop.load(Ordering::SeqCst)
                        },
                    );
                })
            })
            .collect();
        for _ in 0..crate::ssh::host::MAX_RUNNING {
            reports
                .recv_timeout(Duration::from_secs(20))
                .expect("a watch that reports");
        }
        let started = Instant::now();
        let mut stream = link.herdr_connector().connect().expect("a stream");
        stream.write_all(b"{\"id\":\"c\"}\n").unwrap();
        assert_eq!(
            stream
                .read_line_with_timeout(Duration::from_secs(10))
                .unwrap(),
            "{\"id\":\"c\"}\n"
        );
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "the stream waited {:?} behind machine calls",
            started.elapsed()
        );
        stop.store(true, Ordering::SeqCst);
        // A watch reports again on a change; the link ending ends the rest.
        link.close("test finished");
        for watch in watches {
            watch.join().unwrap();
        }
    }

    /// The link ending ends every stream on it: a blocked reader reads the
    /// end, and a new stream is refused.
    #[test]
    fn the_link_ending_ends_its_herdr_streams() {
        let folder = tempfile::tempdir().unwrap();
        let link = linked_node(echo_herdr(folder.path()));
        let mut stream = link.herdr_connector().connect().expect("a stream");
        let reader = std::thread::spawn(move || {
            let mut byte = [0_u8; 1];
            stream.read(&mut byte)
        });
        link.close("test ended the link");
        assert_eq!(reader.join().unwrap().unwrap(), 0);
        assert!(link.herdr_connector().connect().is_err());
    }

    fn answer_value(answer: hide_node_link::LinkAnswer) -> serde_json::Value {
        match answer {
            hide_node_link::LinkAnswer::Parsed(value) => value,
            hide_node_link::LinkAnswer::Raw(raw) => serde_json::from_str(raw.get()).unwrap(),
        }
    }

    /// A stream to the node's browser relay carries bytes both ways, and the
    /// relay has its own cap: past it the next open is refused while Herdr
    /// streams still open, and one closing makes room (B15, D-20).
    #[test]
    fn browser_relay_streams_reach_the_node_s_relay_under_their_own_cap() {
        let folder = tempfile::tempdir().unwrap();
        let link = linked_node_with(echo_herdr(folder.path()), Some(echo_relay()));
        let mut open: Vec<_> = (0..hide_node_link::panes::MAX_BROWSER_STREAMS)
            .map(|_| link.browser_relay_stream().expect("a stream under the cap"))
            .collect();
        open[0].write_all(b"GET /browser-relay/t\r\n").unwrap();
        let mut read = [0_u8; 22];
        open[0].read_exact(&mut read).unwrap();
        assert_eq!(&read, b"GET /browser-relay/t\r\n");
        let refused = link.browser_relay_stream().err().expect("past the cap");
        assert!(matches!(refused, OpenError::Cap(4)), "{refused}");
        link.herdr_connector()
            .connect()
            .expect("a Herdr stream beside a full relay");
        open.pop();
        link.browser_relay_stream().expect("room after a close");
    }

    /// The core's question for a capability reaches the node's gateway with
    /// the scope the core decided, and the gateway's refusal comes back as
    /// its reason; a node with no gateway refuses both the question and a
    /// relay stream.
    #[test]
    fn the_node_s_gateway_answers_the_core_s_capability_question() {
        let folder = tempfile::tempdir().unwrap();
        let link = linked_node_with(echo_herdr(folder.path()), Some(echo_relay()));
        let scope = json!({"workspace": "local\u{0}/checkout", "area_id": "area"});
        let answer = link
            .call(
                Call::BrowserGateway {
                    scope: scope.clone(),
                    relay: true,
                },
                CALL_TIMEOUT,
            )
            .expect("the gateway's answer");
        assert_eq!(answer_value(answer), json!({"scope": scope, "relay": true}));
        let refused = link
            .call(
                Call::BrowserGateway {
                    scope: json!({"workspace": "refused"}),
                    relay: false,
                },
                CALL_TIMEOUT,
            )
            .expect_err("the gateway's refusal");
        assert!(
            refused.to_string().contains("browser_control_unavailable"),
            "{refused}"
        );
        let other = tempfile::tempdir().unwrap();
        let bare = linked_node(echo_herdr(other.path()));
        let refused = bare
            .call(
                Call::BrowserGateway {
                    scope,
                    relay: false,
                },
                CALL_TIMEOUT,
            )
            .expect_err("no gateway");
        assert!(
            refused.to_string().contains("no browser relay"),
            "{refused}"
        );
        let refused = bare.browser_relay_stream().err().expect("no relay");
        assert!(
            refused.to_string().contains("no browser relay"),
            "{refused}"
        );
    }

    /// A node with no Herdr bridge, a device its core dialed, refuses a
    /// Herdr stream, and its refusal reaches the caller.
    #[test]
    fn a_node_without_a_bridge_refuses_herdr_streams() {
        let (core_end, node_end) = hide_platform::ipc::LocalStream::pair().expect("a pair");
        std::thread::spawn(move || {
            let input = BufReader::new(node_end.duplicate());
            let _ =
                hide_host::serve::serve_with(input, node_end, hide_host::serve::Services::none());
        });
        let link =
            super::super::over_local_stream("inbound:test", core_end, None, Box::new(|_| {}))
                .expect("a local link");
        let refused = link.herdr_connector().connect().err().expect("a refusal");
        assert!(
            refused.to_string().contains("not reached through its link"),
            "{refused}"
        );
    }
}
