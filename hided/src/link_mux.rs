//! What crosses one node's connection to its core once the core took the
//! link (PRD system-ssh-device-link B33): the node reaches its core's
//! machine through one owned `ssh`, whose input and output are the attach
//! role's, and those carry several streams at once.
//!
//! Stream 0 is the node's link. Every other stream is one TCP connection
//! the node opens to an address on the core machine's loopback: its
//! screens' and terminals' relays to the core's port, the browser
//! requests beside them, and a Browser View page of the core's machine. The
//! core's end connects each, and only to a loopback address, so nothing the
//! core machine's `sshd` forwards or refuses to forward matters. A ping the
//! core answers tells whether the whole way still answers after this
//! machine slept or moved.
//!
//! Both ends run the same frames from the line after the core's acceptance
//! on, so the attach role between them stays a byte pipe of any build.
//! Each frame is a kind, a stream and a length, then that many bytes, at
//! most [`MAX_FRAME`]. One thread writes the connection's input from a
//! bounded outbox whose answers and opens go ahead of data; a stream or the
//! link that would add data past it waits, so a slow connection slows its
//! writers and holds nothing more.
//!
//! A stream owns its socket from its open to its end, and its slot among
//! [`MAX_STREAMS`] stays taken until every thread of it ended. Its bytes from
//! the other end wait for its socket in a queue of [`MAX_PENDING`] frames;
//! one whose socket does not take them is aborted rather than hold up every
//! other stream: its queue is dropped and its socket shut both ways, which
//! wakes a write stuck on a peer that stopped reading. A stream the other
//! end closed writes what is already queued, each write within
//! [`WRITE_WITHIN`], then shuts its socket. The connection's end aborts
//! every stream.

use std::collections::{HashMap, VecDeque};
use std::io::{self, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use hide_platform::ipc::LocalStream;
use serde_json::json;

/// The most bytes one frame carries.
pub const MAX_FRAME: usize = 64 * 1024;
/// Frames one stream holds for its socket; past it the stream is aborted.
pub const MAX_PENDING: usize = 64;
/// Streams besides the link on one connection, counted until each has
/// ended; one past it is refused.
pub const MAX_STREAMS: usize = 64;
/// Data frames waiting for the connection's input; a writer that would add
/// one past it waits.
const MAX_OUT: usize = 64;
/// Answers and opens waiting for the connection's input; one past it ends
/// the connection, since only a peer that asks faster than the connection
/// carries fills it.
const MAX_CONTROL: usize = 256;
/// How long the core's end waits for a loopback address to take a
/// connection.
const CONNECT_WITHIN: Duration = Duration::from_secs(5);
/// How long one write to a stream's socket may wait for its peer to take
/// the bytes; a peer that stopped reading longer ends its stream.
pub const WRITE_WITHIN: Duration = Duration::from_secs(30);

const DATA: u8 = 0;
const OPEN: u8 = 1;
const CLOSE: u8 = 2;
const PING: u8 = 3;
const PONG: u8 = 4;

/// The link's stream.
const LINK: u32 = 0;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn frame(kind: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    debug_assert!(payload.len() <= MAX_FRAME);
    let mut frame = Vec::with_capacity(9 + payload.len());
    frame.push(kind);
    frame.extend_from_slice(&stream.to_be_bytes());
    frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    frame.extend_from_slice(payload);
    frame
}

fn ended_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotConnected,
        "the connection to the core ended",
    )
}

/// Which end of the connection this is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    /// The node's, which opens streams and asks pings.
    Node,
    /// The core's, which connects each stream and answers pings.
    Core,
}

/// What waits for the connection's input, written by one thread.
#[derive(Default)]
struct Outbox {
    state: Mutex<Out>,
    changed: Condvar,
}

#[derive(Default)]
struct Out {
    control: VecDeque<Vec<u8>>,
    data: VecDeque<Vec<u8>>,
    ended: bool,
}

impl Outbox {
    /// Adds a data frame, waiting while [`MAX_OUT`] wait already; refused
    /// once the connection ended or `cancel`, its stream's, is set, which
    /// ends the wait too.
    fn data(&self, frame: Vec<u8>, cancel: Option<&AtomicBool>) -> io::Result<()> {
        let cancelled = || cancel.is_some_and(|cancel| cancel.load(Ordering::SeqCst));
        let mut out = lock(&self.state);
        while out.data.len() >= MAX_OUT && !out.ended && !cancelled() {
            out = self
                .changed
                .wait(out)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        if out.ended {
            return Err(ended_error());
        }
        if cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "the stream closed",
            ));
        }
        out.data.push_back(frame);
        self.changed.notify_all();
        Ok(())
    }

    /// Wakes every waiter, after a stream's cancel was set: taken under the
    /// outbox's lock, so a waiter that read the flag before it was set is
    /// already waiting and hears this.
    fn wake(&self) {
        let _out = lock(&self.state);
        self.changed.notify_all();
    }

    /// Adds an answer or an open ahead of data, without waiting; refused
    /// once [`MAX_CONTROL`] wait or the connection ended.
    fn control(&self, frame: Vec<u8>) -> io::Result<()> {
        let mut out = lock(&self.state);
        if out.ended {
            return Err(ended_error());
        }
        if out.control.len() >= MAX_CONTROL {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "the connection's answers are full",
            ));
        }
        out.control.push_back(frame);
        self.changed.notify_all();
        Ok(())
    }

    /// The next frame to write, answers first; none once the connection
    /// ended.
    fn next(&self) -> Option<Vec<u8>> {
        let mut out = lock(&self.state);
        loop {
            if out.ended {
                return None;
            }
            if let Some(frame) = out.control.pop_front().or_else(|| out.data.pop_front()) {
                self.changed.notify_all();
                return Some(frame);
            }
            out = self
                .changed
                .wait(out)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Drops what waits and wakes every waiter.
    fn end(&self) {
        let mut out = lock(&self.state);
        out.ended = true;
        out.control.clear();
        out.data.clear();
        self.changed.notify_all();
    }
}

/// One stream, from its open to the end of its last thread.
struct Stream {
    /// The other end's bytes for the socket; dropped when the stream closes,
    /// which lets its writer finish.
    queue: Option<mpsc::SyncSender<Vec<u8>>>,
    /// The socket, shut both ways when the stream is aborted; none until it
    /// is connected.
    socket: Option<TcpStream>,
    closed: bool,
    /// Set when the stream is aborted (the other end's close, a failed
    /// write, an overflow, the connection's end): its reader stops sending,
    /// a wait for room in the outbox included. A normal end never sets it,
    /// so the reader's close stays behind the data it queued.
    cancel: Arc<AtomicBool>,
    /// The stream's threads still running (its connect, its reader, its
    /// writer); its slot is free once none is.
    running: usize,
    /// Told when the slot is free.
    ended: Option<tokio::sync::oneshot::Sender<()>>,
}

struct Shared {
    side: Side,
    outbox: Outbox,
    carrier: hide_platform::ipc::ShutdownHandle,
    link: hide_platform::ipc::ShutdownHandle,
    streams: Mutex<HashMap<u32, Stream>>,
    pings: Mutex<HashMap<u32, mpsc::SyncSender<()>>>,
    next: AtomicU32,
    ended: AtomicBool,
}

/// Frees `id`'s slot once it is closed and none of its threads runs.
fn release(streams: &mut HashMap<u32, Stream>, id: u32) {
    if streams
        .get(&id)
        .is_some_and(|stream| stream.closed && stream.running == 0)
        && let Some(ended) = streams.remove(&id).and_then(|stream| stream.ended)
    {
        let _ = ended.send(());
    }
}

/// Closes `stream`: its queue is dropped, and when `abort` its reader is
/// cancelled and its socket shut both ways at once.
fn shut(stream: &mut Stream, abort: bool) {
    stream.closed = true;
    stream.queue.take();
    if abort {
        stream.cancel.store(true, Ordering::SeqCst);
        if let Some(socket) = &stream.socket {
            let _ = socket.shutdown(Shutdown::Both);
        }
    }
}

impl Shared {
    /// Sends a data frame, waiting for room until the connection ends or
    /// `cancel` is set.
    fn data(
        &self,
        kind: u8,
        stream: u32,
        payload: &[u8],
        cancel: Option<&AtomicBool>,
    ) -> io::Result<()> {
        self.outbox.data(frame(kind, stream, payload), cancel)
    }

    /// Sends an answer ahead of data; a connection whose answers are full
    /// ends.
    fn tell(&self, kind: u8, stream: u32) {
        if let Err(error) = self.outbox.control(frame(kind, stream, &[]))
            && error.kind() == io::ErrorKind::WouldBlock
        {
            self.end("answers_full");
        }
    }

    /// Ends the connection and aborts every stream on it.
    fn end(&self, reason: &str) {
        if self.ended.swap(true, Ordering::SeqCst) {
            return;
        }
        self.outbox.end();
        self.carrier.shutdown();
        self.link.shutdown();
        {
            let mut streams = lock(&self.streams);
            let ids: Vec<u32> = streams.keys().copied().collect();
            for id in ids {
                if let Some(stream) = streams.get_mut(&id) {
                    shut(stream, true);
                }
                release(&mut streams, id);
            }
        }
        lock(&self.pings).clear();
        herdr_core::diagnostic!(json!({
            "component": "link_mux",
            "kind": "mux.ended",
            "side": format!("{:?}", self.side),
            "reason": reason,
        }));
    }

    /// Closes `id`, and answers whether this call closed it; an `abort`
    /// shuts its socket at once, a close the other end asked for lets its
    /// writer finish what is queued. Either cancels its reader.
    fn close(&self, id: u32, abort: bool) -> bool {
        self.close_by(id, abort, true)
    }

    /// [`Shared::close`]; `cancel` is false for a normal end, the stream's
    /// own reader at its socket's end or its writer once it wrote all it
    /// was given, so the reader still sends its close in order.
    fn close_by(&self, id: u32, abort: bool, cancel: bool) -> bool {
        let first = {
            let mut streams = lock(&self.streams);
            let Some(stream) = streams.get_mut(&id) else {
                return false;
            };
            let first = !stream.closed;
            if cancel {
                stream.cancel.store(true, Ordering::SeqCst);
            }
            if first || abort {
                shut(stream, abort);
            }
            release(&mut streams, id);
            first
        };
        if cancel {
            self.outbox.wake();
        }
        first
    }

    /// One of `id`'s threads ended.
    fn finished(&self, id: u32) {
        let mut streams = lock(&self.streams);
        if let Some(stream) = streams.get_mut(&id) {
            stream.running = stream.running.saturating_sub(1);
        }
        release(&mut streams, id);
    }

    /// Aborts `id` and tells the other end.
    fn abort(&self, id: u32) {
        if self.close(id, true) {
            self.tell(CLOSE, id);
        }
    }

    /// Carries `socket` as `id`, whose bytes from the other end already
    /// wait in `queued`: a reader and a writer, each counted in the slot. A
    /// stream closed meanwhile shuts the socket instead.
    fn attach(self: &Arc<Self>, id: u32, socket: TcpStream, queued: mpsc::Receiver<Vec<u8>>) {
        let _ = socket.set_write_timeout(Some(WRITE_WITHIN));
        let handles = socket
            .try_clone()
            .and_then(|reading| socket.try_clone().map(|held| (reading, held)));
        let (reading, cancel) = {
            let mut streams = lock(&self.streams);
            match (streams.get_mut(&id), handles) {
                (Some(stream), Ok((reading, held))) if !stream.closed => {
                    stream.socket = Some(held);
                    stream.running += 2;
                    (reading, Arc::clone(&stream.cancel))
                }
                (stream, _) => {
                    let _ = socket.shutdown(Shutdown::Both);
                    if let Some(stream) = stream {
                        shut(stream, true);
                    }
                    release(&mut streams, id);
                    drop(streams);
                    self.tell(CLOSE, id);
                    return;
                }
            }
        };
        let shared = Arc::clone(self);
        let reader = std::thread::Builder::new()
            .name("link-mux-read".to_owned())
            .spawn(move || shared.read_stream(id, reading, &cancel));
        if reader.is_err() {
            self.abort(id);
            self.finished(id);
        }
        let shared = Arc::clone(self);
        let writer = std::thread::Builder::new()
            .name("link-mux-write".to_owned())
            .spawn(move || shared.write_stream(id, socket, queued));
        if writer.is_err() {
            self.abort(id);
            self.finished(id);
        }
    }

    /// The socket's bytes to the other end, then a close in order after
    /// them, until the stream is cancelled; a close that could not wait its
    /// turn goes ahead of data instead.
    fn read_stream(&self, id: u32, mut socket: TcpStream, cancel: &AtomicBool) {
        let mut buffer = vec![0_u8; MAX_FRAME];
        loop {
            match socket.read(&mut buffer) {
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    if self.data(DATA, id, &buffer[..read], Some(cancel)).is_err() {
                        break;
                    }
                }
            }
        }
        if self.close_by(id, false, false) && self.data(CLOSE, id, &[], Some(cancel)).is_err() {
            self.tell(CLOSE, id);
        }
        self.finished(id);
    }

    /// The other end's bytes to the socket, until the stream closes or a
    /// write fails or waits past [`WRITE_WITHIN`], which aborts it. A
    /// stream that closed once its writer wrote all it was given ends
    /// normally: its reader is not cancelled.
    fn write_stream(&self, id: u32, mut socket: TcpStream, queued: mpsc::Receiver<Vec<u8>>) {
        let mut failed = false;
        while let Ok(bytes) = queued.recv() {
            if socket.write_all(&bytes).is_err() {
                failed = true;
                break;
            }
        }
        drop(queued);
        let _ = socket.shutdown(Shutdown::Both);
        if failed {
            self.abort(id);
        } else {
            self.close_by(id, false, false);
        }
        self.finished(id);
    }
}

/// Starts a thread of the connection: `name` running `work`.
type Spawn<'a> = &'a dyn Fn(&str, Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>>;

fn spawn_thread(name: &str, work: Box<dyn FnOnce() + Send>) -> io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(work)
}

/// A start in progress: a thread that fails to start ends the connection,
/// and the threads already started are waited for, so nothing holds the
/// carrier or the shared state past a failed start.
struct Starting {
    shared: Arc<Shared>,
    started: Vec<JoinHandle<()>>,
    done: bool,
}

impl Drop for Starting {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        self.shared.end("start_failed");
        for thread in self.started.drain(..) {
            let _ = thread.join();
        }
    }
}

/// Starts both ends' threads on `carrier`, whose first bytes the handshake
/// may already have read (`pending`); answers the shared state and the
/// link's own end.
fn start(
    side: Side,
    carrier: LocalStream,
    pending: Vec<u8>,
) -> io::Result<(Arc<Shared>, LocalStream)> {
    start_with(side, carrier, pending, &spawn_thread)
}

fn start_with(
    side: Side,
    carrier: LocalStream,
    pending: Vec<u8>,
    spawn: Spawn<'_>,
) -> io::Result<(Arc<Shared>, LocalStream)> {
    let (ours, link) = LocalStream::pair()?;
    let shared = Arc::new(Shared {
        side,
        outbox: Outbox::default(),
        carrier: carrier.shutdown_handle(),
        link: ours.shutdown_handle(),
        streams: Mutex::new(HashMap::new()),
        pings: Mutex::new(HashMap::new()),
        next: AtomicU32::new(1),
        ended: AtomicBool::new(false),
    });
    let mut starting = Starting {
        shared: Arc::clone(&shared),
        started: Vec::new(),
        done: false,
    };
    // The connection's input, from the outbox.
    {
        let shared = Arc::clone(&shared);
        let mut out = carrier.duplicate();
        starting.started.push(spawn(
            "link-mux-out",
            Box::new(move || {
                while let Some(frame) = shared.outbox.next() {
                    if out.write_all(&frame).and_then(|()| out.flush()).is_err() {
                        break;
                    }
                }
                shared.end("connection_failed");
            }),
        )?);
    }
    // The link's bytes, as frames.
    {
        let shared = Arc::clone(&shared);
        let mut from_link = ours.duplicate();
        starting.started.push(spawn(
            "link-mux-link",
            Box::new(move || {
                let mut buffer = vec![0_u8; MAX_FRAME];
                let reason = loop {
                    match from_link.read(&mut buffer) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Ok(0) | Err(_) => break "link_closed",
                        Ok(read) => {
                            if shared.data(DATA, LINK, &buffer[..read], None).is_err() {
                                break "connection_closed";
                            }
                        }
                    }
                };
                shared.end(reason);
            }),
        )?);
    }
    // The connection's frames, to the link and each stream.
    {
        let shared = Arc::clone(&shared);
        let reader = io::Cursor::new(pending).chain(carrier);
        starting.started.push(spawn(
            "link-mux-frames",
            Box::new(move || {
                let reason = demux(&shared, BufReader::new(reader), ours);
                shared.end(&reason);
            }),
        )?);
    }
    // Started: the threads end with the connection, not with the start.
    starting.done = true;
    Ok((shared, link))
}

/// Reads frames until the connection ends; answers why it did. It never
/// waits on the connection's input, so two ends whose inputs are both full
/// still read each other's frames.
fn demux(shared: &Arc<Shared>, mut reader: impl Read, mut to_link: LocalStream) -> String {
    let mut header = [0_u8; 9];
    loop {
        if let Err(error) = reader.read_exact(&mut header) {
            return if error.kind() == io::ErrorKind::UnexpectedEof {
                "connection_closed".to_owned()
            } else {
                format!("connection_failed: {error}")
            };
        }
        let kind = header[0];
        let stream = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
        let length = u32::from_be_bytes([header[5], header[6], header[7], header[8]]) as usize;
        if length > MAX_FRAME {
            return format!("a frame of {length} bytes");
        }
        let mut payload = vec![0_u8; length];
        if reader.read_exact(&mut payload).is_err() {
            return "connection_closed".to_owned();
        }
        match (kind, shared.side) {
            (DATA, _) if stream == LINK => {
                if to_link.write_all(&payload).is_err() {
                    return "link_closed".to_owned();
                }
            }
            (DATA, _) => {
                let sent = {
                    let streams = lock(&shared.streams);
                    match streams.get(&stream).and_then(|found| found.queue.as_ref()) {
                        Some(queue) => queue.try_send(payload),
                        None => continue,
                    }
                };
                if let Err(error) = sent {
                    if matches!(error, mpsc::TrySendError::Full(_)) {
                        herdr_core::diagnostic!(json!({
                            "component": "link_mux",
                            "kind": "stream.fell_behind",
                            "side": format!("{:?}", shared.side),
                            "stream": stream,
                        }));
                    }
                    shared.abort(stream);
                }
            }
            (CLOSE, _) => {
                shared.close(stream, false);
            }
            (OPEN, Side::Core) if stream != LINK => open(shared, stream, &payload),
            (PING, Side::Core) => shared.tell(PONG, stream),
            (PONG, Side::Node) => {
                if let Some(heard) = lock(&shared.pings).remove(&stream) {
                    let _ = heard.try_send(());
                }
            }
            _ => return format!("an unexpected frame of kind {kind}"),
        }
    }
}

/// The addresses an open names: one or more, each on loopback.
fn addresses(payload: &[u8]) -> Option<Vec<SocketAddr>> {
    let text = std::str::from_utf8(payload).ok()?;
    let addresses: Vec<SocketAddr> = text
        .split(' ')
        .map(|address| address.parse().ok())
        .collect::<Option<_>>()?;
    (!addresses.is_empty()
        && addresses
            .iter()
            .all(|address| address.ip().is_loopback() && address.port() != 0))
    .then_some(addresses)
}

/// The core's end of an open: the stream's queue first, so bytes the node
/// sends before the connection is made wait for it, then the connection on
/// a thread of its own, counted in the slot, to the first address that
/// takes it. A stream closed while it connects drops the connection when
/// it lands.
fn open(shared: &Arc<Shared>, stream: u32, payload: &[u8]) {
    let refuse = |reason: &str| {
        herdr_core::diagnostic!(json!({
            "component": "link_mux",
            "kind": "stream.refused",
            "stream": stream,
            "reason": reason,
        }));
        shared.tell(CLOSE, stream);
    };
    let Some(to) = addresses(payload) else {
        refuse("not_loopback");
        return;
    };
    let (queue, queued) = mpsc::sync_channel(MAX_PENDING);
    {
        let mut streams = lock(&shared.streams);
        if streams.len() >= MAX_STREAMS || streams.contains_key(&stream) {
            drop(streams);
            refuse("streams_full");
            return;
        }
        streams.insert(
            stream,
            Stream {
                queue: Some(queue),
                socket: None,
                closed: false,
                cancel: Arc::new(AtomicBool::new(false)),
                running: 1,
                ended: None,
            },
        );
    }
    let connecting = Arc::clone(shared);
    let started = std::thread::Builder::new()
        .name("link-mux-open".to_owned())
        .spawn(move || {
            let closed = || {
                lock(&connecting.streams)
                    .get(&stream)
                    .is_none_or(|found| found.closed)
            };
            let socket = to.iter().find_map(|address| {
                if closed() {
                    return None;
                }
                TcpStream::connect_timeout(address, CONNECT_WITHIN).ok()
            });
            match socket {
                Some(socket) => connecting.attach(stream, socket, queued),
                None => connecting.abort(stream),
            }
            connecting.finished(stream);
        });
    if started.is_err() {
        shared.abort(stream);
        shared.finished(stream);
    }
}

/// The core's end of a node's connection, started once the core accepted
/// the node: answers its owner and the link's own end, which the core reads
/// as the node's link. The connection ends when the link's other end does,
/// or when the owner is dropped, which the core does once it is done with
/// the link, whatever the connection's threads are waiting on.
pub fn serve(carrier: LocalStream) -> io::Result<(Served, LocalStream)> {
    start(Side::Core, carrier, Vec::new()).map(|(shared, link)| (Served { shared }, link))
}

/// The core's hold on a node's connection: dropping it ends the
/// connection and every stream on it.
pub struct Served {
    shared: Arc<Shared>,
}

impl Drop for Served {
    fn drop(&mut self) {
        self.shared.end("link_owner_ended");
    }
}

/// The node's end of its connection to its core.
pub struct Mux {
    shared: Arc<Shared>,
}

impl Mux {
    /// Starts over `carrier` once the core accepted the node; `pending` is
    /// what the handshake's reader already read past the acceptance.
    /// Answers the link's own end, which the node serves the core's calls
    /// on; the connection ends when the link does, and the link when the
    /// connection does.
    pub fn start(carrier: LocalStream, pending: Vec<u8>) -> io::Result<(Self, LocalStream)> {
        start(Side::Node, carrier, pending).map(|(shared, link)| (Self { shared }, link))
    }

    /// Carries `local`, a connection on this machine, to the first of `to`
    /// on the core machine's loopback that takes it, until either end
    /// closes; the answer hears when the stream's slot is free. It never
    /// waits on the connection. Refused once [`MAX_STREAMS`] streams hold a
    /// slot, the connection's answers are full, or the connection ended;
    /// the core's end closes the stream when no address takes it.
    pub fn carry(&self, local: TcpStream, to: &[SocketAddr]) -> io::Result<Carried> {
        let shared = &self.shared;
        if shared.ended.load(Ordering::SeqCst) {
            return Err(ended_error());
        }
        if to.is_empty() || !to.iter().all(|address| address.ip().is_loopback()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a stream goes only to the core machine's loopback",
            ));
        }
        let id = shared.next.fetch_add(1, Ordering::SeqCst);
        let (queue, queued) = mpsc::sync_channel(MAX_PENDING);
        let (ended, freed) = tokio::sync::oneshot::channel();
        {
            let mut streams = lock(&shared.streams);
            if streams.len() >= MAX_STREAMS {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "the connection to the core carries as many streams as it may",
                ));
            }
            streams.insert(
                id,
                Stream {
                    queue: Some(queue),
                    socket: None,
                    closed: false,
                    cancel: Arc::new(AtomicBool::new(false)),
                    running: 0,
                    ended: Some(ended),
                },
            );
        }
        let named = to
            .iter()
            .map(SocketAddr::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        if let Err(error) = shared.outbox.control(frame(OPEN, id, named.as_bytes())) {
            lock(&shared.streams).remove(&id);
            return Err(error);
        }
        shared.attach(id, local, queued);
        Ok(Carried { freed })
    }

    /// A listener on this machine's loopback whose every connection is
    /// carried to `port` on the core machine's loopback: the screens' way
    /// to the core's port. Dropping it stops the listener; the connections
    /// it carried end with their own ends or the connection.
    pub fn forward(self: &Arc<Self>, port: u16) -> io::Result<Forward> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let local = listener.local_addr()?;
        let stopped = Arc::new(AtomicBool::new(false));
        let to = [SocketAddr::from((Ipv4Addr::LOCALHOST, port))];
        let mux = Arc::clone(self);
        let stop = Arc::clone(&stopped);
        std::thread::Builder::new()
            .name("link-mux-forward".to_owned())
            .spawn(move || {
                for accepted in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(socket) = accepted else {
                        continue;
                    };
                    if let Err(error) = mux.carry(socket, &to) {
                        herdr_core::diagnostic!(json!({
                            "component": "link_mux",
                            "kind": "forward.refused",
                            "reason": error.to_string(),
                        }));
                        if !mux.live() {
                            break;
                        }
                    }
                }
            })?;
        Ok(Forward { local, stopped })
    }

    /// Whether the core's end still answers: a ping answered within
    /// `within`, which covers its wait to be written as well as the answer.
    /// One not answered in time ends the connection, which also wakes a
    /// write stuck on it. Asked after this machine slept or its network
    /// changed, when a connection that looks up may lead nowhere.
    pub fn alive(&self, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        let shared = &self.shared;
        if shared.ended.load(Ordering::SeqCst) {
            return false;
        }
        let ping = shared.next.fetch_add(1, Ordering::SeqCst);
        let (heard, answer) = mpsc::sync_channel(1);
        lock(&shared.pings).insert(ping, heard);
        let answered = shared.outbox.control(frame(PING, ping, &[])).is_ok()
            && answer
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .is_ok();
        lock(&shared.pings).remove(&ping);
        if !answered {
            shared.end("ping_unanswered");
        }
        answered
    }

    /// Whether the connection still carries streams.
    pub fn live(&self) -> bool {
        !self.shared.ended.load(Ordering::SeqCst)
    }

    /// Ends the connection and every stream on it.
    pub fn close(&self) {
        self.shared.end("closed");
    }
}

impl Drop for Mux {
    fn drop(&mut self) {
        self.shared.end("closed");
    }
}

/// A stream [`Mux::carry`] started.
pub struct Carried {
    freed: tokio::sync::oneshot::Receiver<()>,
}

impl Carried {
    /// Returns once the stream ended and its slot is free.
    pub async fn ended(self) {
        let _ = self.freed.await;
    }
}

/// [`Mux::forward`]'s listener.
pub struct Forward {
    local: SocketAddr,
    stopped: Arc<AtomicBool>,
}

impl Forward {
    pub fn port(&self) -> u16 {
        self.local.port()
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        // Wakes the accept, which then sees the stop.
        let _ = TcpStream::connect_timeout(&self.local, Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// A node and a core joined by a local pair, as the attach role joins
    /// them.
    fn joined() -> (Arc<Mux>, LocalStream, LocalStream, Served) {
        let (node_end, core_end) = LocalStream::pair().unwrap();
        let (served, core_link) = serve(core_end).unwrap();
        let (mux, node_link) = Mux::start(node_end, Vec::new()).unwrap();
        (Arc::new(mux), node_link, core_link, served)
    }

    #[test]
    fn the_link_and_a_loopback_stream_cross_the_one_connection() {
        let (mux, mut node_link, mut core_link, _served) = joined();
        node_link.write_all(b"call\n").unwrap();
        let mut heard = [0_u8; 5];
        core_link.read_exact(&mut heard).unwrap();
        assert_eq!(&heard, b"call\n");
        core_link.write_all(b"answer").unwrap();
        let mut heard = [0_u8; 6];
        node_link.read_exact(&mut heard).unwrap();
        assert_eq!(&heard, b"answer");

        // A server on the core machine's loopback, reached through a
        // forward on this machine's.
        let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = server.local_addr().unwrap().port();
        let echo = std::thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            let mut line = [0_u8; 4];
            socket.read_exact(&mut line).unwrap();
            socket.write_all(&line).unwrap();
        });
        let forward = mux.forward(port).unwrap();
        let mut local = TcpStream::connect((Ipv4Addr::LOCALHOST, forward.port())).unwrap();
        local.write_all(b"ping").unwrap();
        let mut back = [0_u8; 4];
        local.read_exact(&mut back).unwrap();
        assert_eq!(&back, b"ping");
        echo.join().unwrap();
        assert!(mux.alive(Duration::from_secs(5)));
    }

    #[test]
    fn a_stream_to_another_address_than_loopback_is_never_opened() {
        assert!(addresses(b"10.0.0.1:80").is_none());
        assert!(addresses(b"127.0.0.1:0").is_none());
        assert!(addresses(b"").is_none());
        assert_eq!(
            addresses(b"127.0.0.1:80 [::1]:80").map(|found| found.len()),
            Some(2)
        );
        let (mux, _node_link, _core_link, _served) = joined();
        let local = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let socket = TcpStream::connect(local.local_addr().unwrap()).unwrap();
        assert!(
            mux.carry(socket, &["192.0.2.1:80".parse().unwrap()])
                .is_err()
        );
    }

    /// A node and a core joined by a local pair, with the core's state.
    fn joined_with_core() -> (Arc<Mux>, Arc<Shared>, LocalStream, LocalStream) {
        let (node_end, core_end) = LocalStream::pair().unwrap();
        let (core, core_link) = start(Side::Core, core_end, Vec::new()).unwrap();
        let (mux, node_link) = Mux::start(node_end, Vec::new()).unwrap();
        (Arc::new(mux), core, node_link, core_link)
    }

    fn slots(shared: &Shared) -> usize {
        lock(&shared.streams).len()
    }

    fn until(what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while !done() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::yield_now();
        }
    }

    /// Reads what reached `socket` until its end, which must come. The read
    /// runs on a thread of its own rather than under a read timeout: macOS
    /// refuses `SO_RCVTIMEO` once the stream ended at the server, which is
    /// the very end this waits for.
    fn drained(socket: TcpStream) {
        let (ended, end) = mpsc::channel();
        let reading = std::thread::spawn(move || {
            let mut socket = socket;
            let mut buffer = vec![0_u8; MAX_FRAME];
            loop {
                match socket.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            let _ = ended.send(());
        });
        // A read that never ended is left to the test process's exit.
        end.recv_timeout(Duration::from_secs(15))
            .expect("the stream never ended at the server");
        reading.join().unwrap();
    }

    /// A server on the core machine that never reads: the stream falls
    /// behind, is aborted, and its slot is free on both ends once its
    /// threads ended, round after round; the server sees its end.
    #[test]
    fn a_stream_whose_peer_stopped_reading_ends_and_frees_its_slot() {
        let (mux, core, _node_link, _core_link) = joined_with_core();
        let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let forward = mux.forward(server.local_addr().unwrap().port()).unwrap();
        let chunk = vec![7_u8; MAX_FRAME];
        for _ in 0..3 {
            let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, forward.port())).unwrap();
            client
                .set_write_timeout(Some(Duration::from_secs(15)))
                .unwrap();
            let (held, _) = server.accept().unwrap();
            let mut ended = false;
            for _ in 0..4096 {
                if client.write_all(&chunk).is_err() {
                    ended = true;
                    break;
                }
            }
            assert!(ended, "the stream was never aborted");
            until("the slots were never freed", || {
                slots(&core) == 0 && slots(&mux.shared) == 0
            });
            drained(held);
        }
        assert!(mux.alive(Duration::from_secs(5)));
    }

    /// The connection's end frees every stream, a stream whose peer stopped
    /// reading included.
    #[test]
    fn the_connection_ending_frees_every_stream() {
        let (mux, core, node_link, _core_link) = joined_with_core();
        let server = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let forward = mux.forward(server.local_addr().unwrap().port()).unwrap();
        let chunk = vec![7_u8; MAX_FRAME];
        let mut held = Vec::new();
        for _ in 0..3 {
            let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, forward.port())).unwrap();
            client
                .set_write_timeout(Some(Duration::from_secs(15)))
                .unwrap();
            for _ in 0..16 {
                client.write_all(&chunk).unwrap();
            }
            held.push((client, server.accept().unwrap().0));
        }
        node_link.shutdown_handle().shutdown();
        until("the slots outlived the connection", || {
            !mux.live() && slots(&core) == 0 && slots(&mux.shared) == 0
        });
        for (_client, socket) in held {
            drained(socket);
        }
    }

    /// A ping on a connection whose other end takes nothing answers within
    /// its bound, whatever waits to be written, and ends the connection,
    /// which wakes the writers stuck on it.
    #[test]
    fn a_ping_on_a_connection_that_takes_nothing_ends_it_in_time() {
        let (node_end, _never_read) = LocalStream::pair().unwrap();
        let (mux, node_link) = Mux::start(node_end, Vec::new()).unwrap();
        let mut reading = node_link.duplicate();
        let mut writing = node_link;
        std::thread::spawn(move || {
            let chunk = vec![0_u8; MAX_FRAME];
            while writing.write_all(&chunk).is_ok() {}
        });
        let (asked, answer) = mpsc::channel();
        let pinging = Arc::new(mux);
        let ping = Arc::clone(&pinging);
        let started = Instant::now();
        std::thread::spawn(move || {
            let _ = asked.send(ping.alive(Duration::from_millis(300)));
        });
        let answered = answer
            .recv_timeout(Duration::from_secs(10))
            .expect("the ping outlived its bound");
        assert!(!answered);
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(!pinging.live());
        // The link's user hears the end.
        let (read, heard) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 1];
            let _ = read.send(reading.read(&mut buffer).map_err(|error| error.kind()));
        });
        assert!(matches!(
            heard.recv_timeout(Duration::from_secs(10)),
            Ok(Ok(0) | Err(_))
        ));
    }

    /// Writes `chunk` to `into` until a write fails, on a thread of its own.
    fn flood(mut into: impl Write + Send + 'static) {
        std::thread::spawn(move || {
            let chunk = vec![7_u8; MAX_FRAME];
            while into.write_all(&chunk).is_ok() {}
        });
    }

    fn data_full(shared: &Shared) -> bool {
        lock(&shared.outbox.state).data.len() >= MAX_OUT
    }

    /// A stream whose reader waits for room in a full outbox hears its
    /// stream close: the close the other end sends wakes it, and the slot
    /// and its carrier's hold are free while the connection takes nothing.
    #[test]
    fn a_reader_waiting_for_room_hears_its_stream_close() {
        let (node_end, mut peer) = LocalStream::pair().unwrap();
        let (mux, _node_link) = Mux::start(node_end, Vec::new()).unwrap();
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (local, _) = listener.accept().unwrap();
        let carried = mux
            .carry(local, &[SocketAddr::from((Ipv4Addr::LOCALHOST, 9))])
            .unwrap();
        flood(client);
        until("the outbox never filled", || data_full(&mux.shared));
        // The reader is the one waiting: the stream is still open.
        assert_eq!(slots(&mux.shared), 1);
        peer.write_all(&frame(CLOSE, 1, &[])).unwrap();
        until("the slot outlived its stream", || slots(&mux.shared) == 0);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime
            .block_on(async {
                tokio::time::timeout(Duration::from_secs(15), carried.ended()).await
            })
            .expect("the carrier's hold outlived its stream");
        assert!(mux.live());
    }

    /// A stream that ends normally while the connection takes nothing
    /// sends its close behind every frame of its data already waiting: the
    /// writer's normal end does not push the close ahead of them.
    #[test]
    fn a_normal_end_closes_behind_the_streams_queued_data() {
        let (node_end, mut peer) = LocalStream::pair().unwrap();
        let (mux, _node_link) = Mux::start(node_end, Vec::new()).unwrap();
        // The stream's data, as its reader queued it, until the connection's
        // input is full and the outbox stays full behind it.
        let chunk = vec![7_u8; MAX_FRAME];
        let mut queued = 0;
        for _ in 0..1000 {
            while !data_full(&mux.shared) {
                mux.shared.data(DATA, 1, &chunk, None).unwrap();
                queued += 1;
            }
            std::thread::yield_now();
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (local, _) = listener.accept().unwrap();
        let _carried = mux
            .carry(local, &[SocketAddr::from((Ipv4Addr::LOCALHOST, 9))])
            .unwrap();
        // The local end ends normally; its writer has nothing to write.
        client.shutdown(Shutdown::Both).unwrap();
        until("the stream's writer never ended", || {
            lock(&mux.shared.streams)
                .get(&1)
                .is_none_or(|stream| stream.closed && stream.running == 1)
        });
        // The connection takes its input again.
        peer.set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut data = 0;
        loop {
            let mut header = [0_u8; 9];
            peer.read_exact(&mut header).unwrap();
            let stream = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
            let length = u32::from_be_bytes([header[5], header[6], header[7], header[8]]);
            let mut payload = vec![0_u8; length as usize];
            peer.read_exact(&mut payload).unwrap();
            match (header[0], stream) {
                (DATA, 1) => data += payload.len(),
                (CLOSE, 1) => break,
                _ => {}
            }
        }
        assert_eq!(data, queued * MAX_FRAME, "the close overtook its data");
        until("the slot outlived its stream", || slots(&mux.shared) == 0);
    }

    /// The core's owner ends the connection while the link's reader waits
    /// for room and the connection takes nothing: every thread of it ends
    /// and both the link and the connection see their end.
    #[test]
    fn the_cores_owner_ends_a_connection_whose_link_reader_waits() {
        let (core_end, mut peer) = LocalStream::pair().unwrap();
        let (served, mut core_link) = serve(core_end).unwrap();
        let shared = Arc::downgrade(&served.shared);
        flood(core_link.duplicate());
        until("the outbox never filled", || {
            shared.upgrade().is_some_and(|shared| data_full(&shared))
        });
        drop(served);
        until("the connection's threads outlived its owner", || {
            shared.strong_count() == 0
        });
        let mut buffer = [0_u8; 1];
        core_link
            .set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        assert!(matches!(core_link.read(&mut buffer), Ok(0) | Err(_)));
        peer.set_read_timeout(Some(Duration::from_secs(15)))
            .unwrap();
        let mut drained = vec![0_u8; MAX_FRAME];
        loop {
            match peer.read(&mut drained) {
                Ok(0) => break,
                Ok(_) => {}
                Err(error) => panic!("the connection never ended: {error}"),
            }
        }
    }

    /// A thread that fails to start ends the connection and waits for the
    /// ones that started, whichever one failed.
    #[test]
    fn a_start_that_fails_partway_ends_what_it_started() {
        for fail_at in [2, 3] {
            let (carrier, mut peer) = LocalStream::pair().unwrap();
            let calls = AtomicU32::new(0);
            let finished = Arc::new(AtomicU32::new(0));
            let spawn = |name: &str, work: Box<dyn FnOnce() + Send>| {
                if calls.fetch_add(1, Ordering::SeqCst) + 1 == fail_at {
                    return Err(io::Error::other("injected"));
                }
                let finished = Arc::clone(&finished);
                spawn_thread(
                    name,
                    Box::new(move || {
                        work();
                        finished.fetch_add(1, Ordering::SeqCst);
                    }),
                )
            };
            assert!(start_with(Side::Core, carrier, Vec::new(), &spawn).is_err());
            assert_eq!(finished.load(Ordering::SeqCst), fail_at - 1);
            peer.set_read_timeout(Some(Duration::from_secs(15)))
                .unwrap();
            let mut buffer = [0_u8; 1];
            assert!(matches!(peer.read(&mut buffer), Ok(0)), "{fail_at}");
        }
    }

    /// The link's end ends the whole connection, and with it every stream
    /// and the next ping.
    #[test]
    fn the_link_ending_ends_the_connection() {
        let (mux, node_link, mut core_link, _served) = joined();
        node_link.shutdown_handle().shutdown();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buffer = [0_u8; 1];
        assert_eq!(core_link.read(&mut buffer).unwrap(), 0);
        while mux.live() {
            assert!(
                Instant::now() < deadline,
                "the connection outlived its link"
            );
            std::thread::yield_now();
        }
        assert!(!mux.alive(Duration::from_millis(200)));
    }
}
