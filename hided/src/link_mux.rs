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
//! most [`MAX_FRAME`]. A stream's bytes wait for its socket in a queue of
//! [`MAX_PENDING`] frames; one whose socket does not take them is closed
//! rather than hold up every other stream, as a link stream is.

use std::collections::HashMap;
use std::io::{self, BufReader, Read, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use hide_platform::ipc::LocalStream;
use serde_json::json;

/// The most bytes one frame carries.
pub const MAX_FRAME: usize = 64 * 1024;
/// Frames one stream holds for its socket; past it the stream is closed.
pub const MAX_PENDING: usize = 64;
/// Streams besides the link open at once on one connection; one past it is
/// refused.
pub const MAX_STREAMS: usize = 64;
/// Answers the frame reader leaves for the control thread.
const MAX_CONTROL: usize = 256;
/// How long the core's end waits for a loopback address to take a
/// connection.
const CONNECT_WITHIN: Duration = Duration::from_secs(5);

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

/// Which end of the connection this is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Side {
    /// The node's, which opens streams and asks pings.
    Node,
    /// The core's, which connects each stream and answers pings.
    Core,
}

struct Shared {
    side: Side,
    /// The connection's input: one frame at a time.
    out: Mutex<LocalStream>,
    carrier: hide_platform::ipc::ShutdownHandle,
    link: hide_platform::ipc::ShutdownHandle,
    streams: Mutex<HashMap<u32, mpsc::SyncSender<Vec<u8>>>>,
    pings: Mutex<HashMap<u32, mpsc::SyncSender<()>>>,
    /// Frames the frame reader answers with, sent by a thread of their own:
    /// the reader never waits on the connection's input, so two ends whose
    /// inputs are both full still read each other's frames.
    control: Mutex<Option<mpsc::SyncSender<(u8, u32)>>>,
    next: AtomicU32,
    ended: AtomicBool,
}

impl Shared {
    fn send(&self, kind: u8, stream: u32, payload: &[u8]) -> io::Result<()> {
        debug_assert!(payload.len() <= MAX_FRAME);
        let mut frame = Vec::with_capacity(9 + payload.len());
        frame.push(kind);
        frame.extend_from_slice(&stream.to_be_bytes());
        frame.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        frame.extend_from_slice(payload);
        let mut out = lock(&self.out);
        out.write_all(&frame).and_then(|()| out.flush())
    }

    /// Queues a frame with no payload for the control thread; one past its
    /// queue is dropped, and the stream it closed ends with its own socket.
    fn tell(&self, kind: u8, stream: u32) {
        if let Some(control) = lock(&self.control).as_ref()
            && control.try_send((kind, stream)).is_err()
        {
            herdr_core::diagnostic!(json!({
                "component": "link_mux",
                "kind": "control.dropped",
                "frame": kind,
                "stream": stream,
            }));
        }
    }

    /// Ends the connection and every stream on it.
    fn end(&self, reason: &str) {
        if self.ended.swap(true, Ordering::SeqCst) {
            return;
        }
        self.carrier.shutdown();
        self.link.shutdown();
        lock(&self.control).take();
        lock(&self.streams).clear();
        lock(&self.pings).clear();
        herdr_core::diagnostic!(json!({
            "component": "link_mux",
            "kind": "mux.ended",
            "side": format!("{:?}", self.side),
            "reason": reason,
        }));
    }

    /// Takes `stream` out, so its socket closes, and tells the other end.
    fn close(&self, stream: u32) {
        if lock(&self.streams).remove(&stream).is_some() {
            let _ = self.send(CLOSE, stream, &[]);
        }
    }

    /// Starts carrying `socket` as `stream`, whose bytes from the other end
    /// already wait in `queue`.
    fn carry(self: &Arc<Self>, stream: u32, socket: TcpStream, queue: mpsc::Receiver<Vec<u8>>) {
        let Ok(mut reading) = socket.try_clone() else {
            self.close(stream);
            return;
        };
        let shared = Arc::clone(self);
        let started = std::thread::Builder::new()
            .name("link-mux-read".to_owned())
            .spawn(move || {
                let mut buffer = vec![0_u8; MAX_FRAME];
                loop {
                    match reading.read(&mut buffer) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            if !lock(&shared.streams).contains_key(&stream)
                                || shared.send(DATA, stream, &buffer[..read]).is_err()
                            {
                                break;
                            }
                        }
                    }
                }
                shared.close(stream);
                let _ = reading.shutdown(Shutdown::Both);
            });
        if started.is_err() {
            self.close(stream);
            return;
        }
        write_queue(socket, queue);
    }
}

/// Writes what the other end sent for one stream until the stream is
/// taken out, then closes its socket both ways.
fn write_queue(mut socket: TcpStream, queue: mpsc::Receiver<Vec<u8>>) {
    while let Ok(bytes) = queue.recv() {
        if socket.write_all(&bytes).is_err() {
            break;
        }
    }
    let _ = socket.shutdown(Shutdown::Both);
}

/// Starts both ends' threads on `carrier`, whose first bytes the handshake
/// may already have read (`pending`); answers the shared state and the
/// link's own end.
fn start(
    side: Side,
    carrier: LocalStream,
    pending: Vec<u8>,
) -> io::Result<(Arc<Shared>, LocalStream)> {
    let (ours, link) = LocalStream::pair()?;
    let (control, told) = mpsc::sync_channel::<(u8, u32)>(MAX_CONTROL);
    let shared = Arc::new(Shared {
        side,
        out: Mutex::new(carrier.duplicate()),
        carrier: carrier.shutdown_handle(),
        link: ours.shutdown_handle(),
        streams: Mutex::new(HashMap::new()),
        pings: Mutex::new(HashMap::new()),
        control: Mutex::new(Some(control)),
        next: AtomicU32::new(1),
        ended: AtomicBool::new(false),
    });
    {
        let shared = Arc::clone(&shared);
        std::thread::Builder::new()
            .name("link-mux-control".to_owned())
            .spawn(move || {
                while let Ok((kind, stream)) = told.recv() {
                    if shared.send(kind, stream, &[]).is_err() {
                        break;
                    }
                }
            })?;
    }
    // The link's bytes, as frames.
    {
        let shared = Arc::clone(&shared);
        let mut from_link = ours.duplicate();
        std::thread::Builder::new()
            .name("link-mux-link".to_owned())
            .spawn(move || {
                let mut buffer = vec![0_u8; MAX_FRAME];
                let reason = loop {
                    match from_link.read(&mut buffer) {
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                        Ok(0) | Err(_) => break "link_closed",
                        Ok(read) => {
                            if shared.send(DATA, LINK, &buffer[..read]).is_err() {
                                break "connection_closed";
                            }
                        }
                    }
                };
                shared.end(reason);
            })?;
    }
    // The connection's frames, to the link and each stream.
    {
        let shared = Arc::clone(&shared);
        let reader = io::Cursor::new(pending).chain(carrier);
        std::thread::Builder::new()
            .name("link-mux-frames".to_owned())
            .spawn(move || {
                let reason = demux(&shared, BufReader::new(reader), ours);
                shared.end(&reason);
            })?;
    }
    Ok((shared, link))
}

/// Reads frames until the connection ends; answers why it did.
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
                let mut streams = lock(&shared.streams);
                let Some(queue) = streams.get(&stream) else {
                    continue;
                };
                match queue.try_send(payload) {
                    Ok(()) => {}
                    Err(mpsc::TrySendError::Full(_)) => {
                        streams.remove(&stream);
                        drop(streams);
                        shared.tell(CLOSE, stream);
                        herdr_core::diagnostic!(json!({
                            "component": "link_mux",
                            "kind": "stream.fell_behind",
                            "side": format!("{:?}", shared.side),
                            "stream": stream,
                        }));
                    }
                    Err(mpsc::TrySendError::Disconnected(_)) => {
                        streams.remove(&stream);
                    }
                }
            }
            (CLOSE, _) => {
                lock(&shared.streams).remove(&stream);
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
/// a thread of its own, to the first address that takes it.
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
        streams.insert(stream, queue);
    }
    let connecting = Arc::clone(shared);
    let started = std::thread::Builder::new()
        .name("link-mux-open".to_owned())
        .spawn(move || {
            let socket = to
                .iter()
                .find_map(|address| TcpStream::connect_timeout(address, CONNECT_WITHIN).ok());
            match socket {
                Some(socket) => connecting.carry(stream, socket, queued),
                None => connecting.close(stream),
            }
        });
    if started.is_err() {
        shared.close(stream);
    }
}

/// The core's end of a node's connection, started once the core accepted
/// the node: answers the link's own end, which the core reads as the node's
/// link; the connection ends when the link does.
pub fn serve(carrier: LocalStream) -> io::Result<LocalStream> {
    start(Side::Core, carrier, Vec::new()).map(|(_, link)| link)
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
    /// closes. Refused once [`MAX_STREAMS`] streams are open or the
    /// connection ended; the core's end closes the stream when no address
    /// takes it.
    pub fn carry(&self, local: TcpStream, to: &[SocketAddr]) -> io::Result<()> {
        let shared = &self.shared;
        if shared.ended.load(Ordering::SeqCst) {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "the connection to the core ended",
            ));
        }
        if to.is_empty() || !to.iter().all(|address| address.ip().is_loopback()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a stream goes only to the core machine's loopback",
            ));
        }
        let stream = shared.next.fetch_add(1, Ordering::SeqCst);
        let (queue, queued) = mpsc::sync_channel(MAX_PENDING);
        {
            let mut streams = lock(&shared.streams);
            if streams.len() >= MAX_STREAMS {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "the connection to the core carries as many streams as it may",
                ));
            }
            streams.insert(stream, queue);
        }
        let named = to
            .iter()
            .map(SocketAddr::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        if let Err(error) = shared.send(OPEN, stream, named.as_bytes()) {
            lock(&shared.streams).remove(&stream);
            return Err(error);
        }
        let carrying = Arc::clone(shared);
        std::thread::Builder::new()
            .name("link-mux-write".to_owned())
            .spawn(move || carrying.carry(stream, local, queued))
            .map(|_| ())
            .inspect_err(|_| shared.close(stream))
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
    /// `within`. Asked after this machine slept or its network changed,
    /// when a connection that looks up may lead nowhere.
    pub fn alive(&self, within: Duration) -> bool {
        let shared = &self.shared;
        if shared.ended.load(Ordering::SeqCst) {
            return false;
        }
        let ping = shared.next.fetch_add(1, Ordering::SeqCst);
        let (heard, answer) = mpsc::sync_channel(1);
        lock(&shared.pings).insert(ping, heard);
        let answered = shared.send(PING, ping, &[]).is_ok() && answer.recv_timeout(within).is_ok();
        lock(&shared.pings).remove(&ping);
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
    fn joined() -> (Arc<Mux>, LocalStream, LocalStream) {
        let (node_end, core_end) = LocalStream::pair().unwrap();
        let core_link = serve(core_end).unwrap();
        let (mux, node_link) = Mux::start(node_end, Vec::new()).unwrap();
        (Arc::new(mux), node_link, core_link)
    }

    #[test]
    fn the_link_and_a_loopback_stream_cross_the_one_connection() {
        let (mux, mut node_link, mut core_link) = joined();
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
        let (mux, _node_link, _core_link) = joined();
        let local = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let socket = TcpStream::connect(local.local_addr().unwrap()).unwrap();
        assert!(
            mux.carry(socket, &["192.0.2.1:80".parse().unwrap()])
                .is_err()
        );
    }

    /// The link's end ends the whole connection, and with it every stream
    /// and the next ping.
    #[test]
    fn the_link_ending_ends_the_connection() {
        let (mux, node_link, mut core_link) = joined();
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
