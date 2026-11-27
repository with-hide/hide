//! A peer connection's streams over one carrier (PRD device-connect-iroh
//! D-27, D-62, D-79): what [`crate::pipe`] hands the seam as a connection's
//! streams, where a QUIC connection has streams of its own.
//!
//! Either end opens a stream, each with a priority, and each direction of a
//! stream ends on its own: its writer finishes it ([`PipeSend`]'s shutdown
//! or drop), or its reader stops it ([`PipeRecv`]'s drop), which fails the
//! writer's next write. An end opens at most [`MAX_STREAMS`] at once, and a
//! stream's place is free once both its directions ended.
//!
//! Nothing a slow reader leaves unread holds another stream up, and nothing
//! is dropped for it: a writer sends only what its reader gave it credit
//! for, [`WINDOW`] frames at the start and more as the reader takes them, so
//! a stream's bytes wait in its writer, never in the carrier. Frames of a
//! larger priority are written first, and each priority has its own
//! [`CLASS_ROOM`] places to wait for the one writer in, so keys typed into a
//! pane never wait behind a file. What an end keeps for a connection is
//! therefore bounded by counts: the streams it opened, the other end's up
//! to [`MAX_THEIRS`], and for each [`WINDOW`] frames of at most
//! [`super::MAX_FRAME`] bytes for its reader.
//!
//! The first frame each end writes says the version of these frames it
//! speaks; two ends that differ end the connection. A connection closed on
//! purpose says why in its last frame ([`Pipe::close`]), which the other
//! end reads as the connection's close code; a carrier that just ends reads
//! as lost.
//!
//! The carrier's two threads never wait on a stream: a frame for a stream
//! goes into that stream's queue or is a contract violation, which ends the
//! connection.

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Semaphore, mpsc, watch};

use super::{
    BYE, Carrier, HELLO, PEER_CREDIT, PEER_DATA, PEER_FIN, PEER_OPEN, PEER_STOP, Shared, Side,
    frame, lock, spawn_thread, start_with,
};

/// The version of the peer frames this build writes and reads.
pub const VERSION: u8 = 1;

/// The priority of the stream a connection's two ends prove their keys on,
/// ahead of anything else on the carrier.
pub const PROOF_PRIORITY: i32 = i32::MAX;

/// Whether a stream may be opened with `priority`: a stream kind's, or the
/// proof's. Each priority keeps its frames' place in the outbox for as long
/// as the connection lives and the other end names the number, so the
/// connection carries these few and no other.
fn carried(priority: i32) -> bool {
    priority == PROOF_PRIORITY
        || hide_peer_protocol::StreamKind::ALL
            .iter()
            .any(|kind| kind.priority() == priority)
}

/// Streams one end has open at once, counted until both directions of each
/// ended; its next open is refused.
pub const MAX_STREAMS: usize = crate::limits::MAX_STREAMS;

/// Streams the other end opened that this end holds a place for. An end
/// that keeps to [`MAX_STREAMS`] never reaches it: its open of a new stream
/// goes ahead of the last frames of one it just ended, so this end may
/// count both for a moment, and twice the cap covers that. One past it is
/// refused.
pub const MAX_THEIRS: usize = 2 * MAX_STREAMS;

/// Frames a stream's reader holds unread, which is the credit its writer
/// starts with.
pub const WINDOW: usize = 16;

/// The most bytes a writer puts in one frame, so a window is at most 256 KiB
/// and a frame of a larger priority waits behind at most this many bytes.
pub const MAX_CHUNK: usize = 16 * 1024;

/// Frames of one priority that wait for the connection's one writer; a
/// stream that would add one past it waits.
pub(super) const CLASS_ROOM: usize = 16;

/// Streams the other end opened that wait to be accepted; one past it is
/// refused.
const BACKLOG: usize = 16;

/// How long a close waits for its last frame to be written before the
/// carrier is ended without it, and how long an end whose write failed
/// waits to read the other end's.
pub(super) const BYE_WITHIN: Duration = crate::limits::BYE_WITHIN;

/// What a connection closed with when its carrier ended without a word:
/// `CloseCode::Lost`.
pub(super) fn lost() -> u32 {
    hide_peer_protocol::CloseCode::Lost.number()
}

/// A peer connection's part of the shared state.
pub(super) struct State {
    streams: Mutex<HashMap<u32, Entry>>,
    /// Where the streams the other end opened go; none once the connection
    /// ended.
    incoming: Mutex<Option<mpsc::Sender<(PipeSend, PipeRecv)>>>,
    /// The other end said which frames it speaks.
    greeted: AtomicBool,
    /// The code the connection closed with, once it has.
    closed: watch::Sender<Option<u32>>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            streams: Mutex::new(HashMap::new()),
            incoming: Mutex::new(None),
            greeted: AtomicBool::new(false),
            closed: watch::channel(None).0,
        }
    }
}

impl State {
    /// A peer connection's state, with where the other end's streams go.
    /// It is whole before the connection's threads start, so an open the
    /// carrier already holds is not refused for want of it.
    fn accepting(incoming: mpsc::Sender<(PipeSend, PipeRecv)>) -> Self {
        Self {
            incoming: Mutex::new(Some(incoming)),
            ..Self::default()
        }
    }
}

/// One stream, from its open until both its directions ended.
struct Entry {
    /// The other end's bytes for this end's reader; none once that
    /// direction ended.
    inbound: Option<mpsc::Sender<Vec<u8>>>,
    /// The other end finished its direction, so the reader's end is an end
    /// of stream and not a loss.
    finished: Arc<AtomicBool>,
    /// The frames this end's writer may still send.
    credit: Arc<Semaphore>,
    /// The other end stopped reading, or the connection ended: writes fail.
    stopped: Arc<AtomicBool>,
    /// This end's writer has not finished and was not stopped.
    sending: bool,
}

impl Entry {
    fn over(&self) -> bool {
        !self.sending && self.inbound.is_none()
    }
}

impl State {
    /// The connection ended: every writer fails, every reader that was not
    /// finished reads a loss, and nothing more is accepted. Its code was
    /// recorded before it ended ([`State::close_with`]).
    pub(super) fn end(&self) {
        lock(&self.incoming).take();
        for (_, entry) in lock(&self.streams).drain() {
            entry.stopped.store(true, Ordering::SeqCst);
            entry.credit.close();
        }
    }

    /// Records the code the connection closes with; the first one stands.
    pub(super) fn close_with(&self, code: u32) {
        self.closed.send_if_modified(|closed| {
            if closed.is_some() {
                return false;
            }
            *closed = Some(code);
            true
        });
    }
}

/// Whether `id` is one the end that `dialed` opens: the dialing end's
/// streams are odd and the other end's even, so two opens never collide.
fn opened_by(id: u32, dialed: bool) -> bool {
    id != 0 && (id % 2 == 1) == dialed
}

/// Makes `id`'s two halves and its entry.
fn halves(shared: &Arc<Shared>, id: u32, priority: i32) -> (Entry, PipeSend, PipeRecv) {
    let (inbound, queued) = mpsc::channel(WINDOW);
    let finished = Arc::new(AtomicBool::new(false));
    let credit = Arc::new(Semaphore::new(WINDOW));
    let stopped = Arc::new(AtomicBool::new(false));
    let entry = Entry {
        inbound: Some(inbound),
        finished: Arc::clone(&finished),
        credit: Arc::clone(&credit),
        stopped: Arc::clone(&stopped),
        sending: true,
    };
    let send = PipeSend {
        shared: Arc::clone(shared),
        id,
        priority,
        credit,
        room: shared.outbox.room(priority),
        stopped,
        waiting: None,
        finished: false,
    };
    let recv = PipeRecv {
        shared: Arc::clone(shared),
        id,
        queued,
        chunk: Vec::new(),
        at: 0,
        owed: 0,
        finished,
        over: false,
    };
    (entry, send, recv)
}

/// One frame a peer connection read; an error is why the connection ends.
/// It never waits.
pub(super) fn frame_in(
    shared: &Arc<Shared>,
    kind: u8,
    stream: u32,
    payload: Vec<u8>,
) -> Result<(), String> {
    let Side::Peer { dialed } = shared.side else {
        return Err("a peer frame on a connection of another kind".to_owned());
    };
    let state = &shared.peer;
    if !state.greeted.load(Ordering::SeqCst) {
        // The first frame says which frames follow.
        return match (kind, payload.as_slice()) {
            (HELLO, [VERSION]) => {
                state.greeted.store(true, Ordering::SeqCst);
                Ok(())
            }
            (HELLO, [version]) => Err(format!("peer frames of version {version}")),
            _ => Err(format!("a frame of kind {kind} before the greeting")),
        };
    }
    match kind {
        PEER_OPEN => {
            let Ok(priority) = <[u8; 4]>::try_from(payload.as_slice()).map(i32::from_be_bytes)
            else {
                return Err("an open with no priority".to_owned());
            };
            if !opened_by(stream, !dialed) {
                return Err(format!(
                    "an open of stream {stream}, which is not its end's"
                ));
            }
            let mut streams = lock(&state.streams);
            if streams.contains_key(&stream) {
                return Err(format!("a second open of stream {stream}"));
            }
            let incoming = lock(&state.incoming).clone();
            let refused = |shared: &Shared| {
                // Both directions over: its writer fails and its reader ends.
                shared.tell(PEER_STOP, stream);
                shared.tell(PEER_FIN, stream);
            };
            let theirs = streams.keys().filter(|id| opened_by(**id, !dialed)).count();
            let Some(incoming) = incoming.filter(|_| theirs < MAX_THEIRS && carried(priority))
            else {
                drop(streams);
                refused(shared);
                return Ok(());
            };
            let (entry, send, recv) = halves(shared, stream, priority);
            streams.insert(stream, entry);
            drop(streams);
            // Nobody accepts as fast as the other end opens: the halves are
            // dropped, which ends both directions for it.
            let _ = incoming.try_send((send, recv));
        }
        PEER_DATA => {
            let queue = lock(&state.streams)
                .get(&stream)
                .and_then(|entry| entry.inbound.clone());
            // A frame for a direction that already ended crossed its end.
            if let Some(queue) = queue
                && let Err(mpsc::error::TrySendError::Full(_)) = queue.try_send(payload)
            {
                return Err(format!("stream {stream} sent past its credit"));
            }
        }
        PEER_FIN => {
            let mut streams = lock(&state.streams);
            if let Some(entry) = streams.get_mut(&stream) {
                entry.finished.store(true, Ordering::SeqCst);
                entry.inbound = None;
                if entry.over() {
                    streams.remove(&stream);
                }
            }
        }
        PEER_STOP => {
            let mut streams = lock(&state.streams);
            if let Some(entry) = streams.get_mut(&stream) {
                entry.stopped.store(true, Ordering::SeqCst);
                entry.credit.close();
                entry.sending = false;
                if entry.over() {
                    streams.remove(&stream);
                }
            }
        }
        PEER_CREDIT => {
            let Ok(more) = <[u8; 4]>::try_from(payload.as_slice()).map(u32::from_be_bytes) else {
                return Err("a credit with no count".to_owned());
            };
            if let Some(entry) = lock(&state.streams).get(&stream) {
                // Never more than a window at once, whatever the other end
                // says it took.
                let room = WINDOW.saturating_sub(entry.credit.available_permits());
                entry.credit.add_permits(room.min(more as usize));
            }
        }
        BYE => {
            let code = <[u8; 4]>::try_from(payload.as_slice()).map_or(lost(), u32::from_be_bytes);
            state.close_with(code);
            return Err("closed_by_peer".to_owned());
        }
        _ => return Err(format!("an unexpected frame of kind {kind}")),
    }
    Ok(())
}

/// One machine's end of a peer connection's carrier. Every clone names the
/// same connection, which ends when [`Pipe::close`] is called or its
/// carrier ends, never because a clone was dropped.
#[derive(Clone)]
pub struct Pipe {
    shared: Arc<Shared>,
    dialed: bool,
    next: Arc<AtomicU32>,
}

impl Pipe {
    /// Starts on `carrier`, as the end that made it (`dialed`) or the one
    /// it arrived at; answers the streams the other end opens, in the order
    /// it opened them.
    pub fn start(
        carrier: Carrier,
        dialed: bool,
    ) -> io::Result<(Self, mpsc::Receiver<(PipeSend, PipeRecv)>)> {
        Self::start_on(carrier, dialed, &spawn_thread)
    }

    /// [`Pipe::start`], on threads `spawn` starts.
    fn start_on(
        carrier: Carrier,
        dialed: bool,
        spawn: super::Spawn<'_>,
    ) -> io::Result<(Self, mpsc::Receiver<(PipeSend, PipeRecv)>)> {
        let (incoming, accepted) = mpsc::channel(BACKLOG);
        let shared = start_with(
            Side::Peer { dialed },
            carrier,
            None,
            Vec::new(),
            State::accepting(incoming),
            spawn,
        )?;
        shared.tell_with(HELLO, 0, &[VERSION]);
        Ok((
            Self {
                shared,
                dialed,
                next: Arc::new(AtomicU32::new(if dialed { 1 } else { 2 })),
            },
            accepted,
        ))
    }

    /// Opens a stream; frames of a larger `priority` are written first.
    /// Refused once [`MAX_STREAMS`] streams this end opened hold a place, or
    /// the connection ended, and for a priority no stream kind has, which
    /// the other end would refuse.
    pub fn open(&self, priority: i32) -> io::Result<(PipeSend, PipeRecv)> {
        let shared = &self.shared;
        // Closed is the one word for it: a connection has its code from
        // the moment either end closes it, before its last frame is written
        // or its carrier ended, and it opens nothing from then on.
        if shared.peer.closed.borrow().is_some() {
            return Err(super::ended_error());
        }
        if !carried(priority) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("no stream kind has priority {priority}"),
            ));
        }
        let id = self.next.fetch_add(2, Ordering::SeqCst);
        let (send, recv) = {
            let mut streams = lock(&shared.peer.streams);
            let ours = streams
                .keys()
                .filter(|id| opened_by(**id, self.dialed))
                .count();
            if ours >= MAX_STREAMS {
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    "the connection carries as many streams as it may",
                ));
            }
            let (entry, send, recv) = halves(shared, id, priority);
            streams.insert(id, entry);
            (send, recv)
        };
        // The open goes ahead of every stream's data, so the other end
        // knows the stream before its first frame.
        shared.tell_with(PEER_OPEN, id, &priority.to_be_bytes());
        Ok((send, recv))
    }

    /// Closes the connection: the other end reads `code` as why, after the
    /// frames already waiting ahead of it, and this end reads it at once. A
    /// carrier that takes nothing is ended without the word.
    pub fn close(&self, code: u32) {
        let shared = &self.shared;
        shared.peer.close_with(code);
        if shared.ended.load(Ordering::SeqCst) {
            return;
        }
        if shared
            .outbox
            .control(frame(BYE, 0, &code.to_be_bytes()))
            .is_err()
        {
            shared.end("closed");
            return;
        }
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => {
                let shared = Arc::clone(shared);
                runtime.spawn(async move {
                    tokio::time::sleep(BYE_WITHIN).await;
                    shared.end("closed");
                });
            }
            // Nothing here can wait for the word to be written.
            Err(_) => shared.end("closed"),
        }
    }

    /// The code the connection closed with, once it has.
    pub fn ended(&self) -> Option<u32> {
        *self.shared.peer.closed.borrow()
    }

    /// Hears the connection's close.
    pub fn closed(&self) -> watch::Receiver<Option<u32>> {
        self.shared.peer.closed.subscribe()
    }

    /// How many streams hold a place now.
    pub fn streams(&self) -> usize {
        lock(&self.shared.peer.streams).len()
    }
}

type Waiting = Pin<Box<dyn Future<Output = Result<(), tokio::sync::AcquireError>> + Send>>;

/// A stream's writing half. Dropping it finishes the direction, as its
/// shutdown does: the other end reads what was written and then the end.
pub struct PipeSend {
    shared: Arc<Shared>,
    id: u32,
    priority: i32,
    credit: Arc<Semaphore>,
    room: Arc<Semaphore>,
    stopped: Arc<AtomicBool>,
    /// The wait for a frame's credit and its place, kept across polls.
    waiting: Option<Waiting>,
    finished: bool,
}

fn stream_over() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "the stream's other end stopped reading, or the connection ended",
    )
}

impl PipeSend {
    fn dead(&self) -> bool {
        self.stopped.load(Ordering::SeqCst) || self.shared.ended.load(Ordering::SeqCst)
    }

    fn finish(&mut self) {
        if std::mem::replace(&mut self.finished, true) {
            return;
        }
        self.waiting = None;
        if !self.dead() {
            // Behind the stream's own data, so it takes the same way; it
            // takes no place, since a stream sends one.
            let _ = self
                .shared
                .outbox
                .class(self.priority, frame(PEER_FIN, self.id, &[]), false);
        }
        let mut streams = lock(&self.shared.peer.streams);
        if let Some(entry) = streams.get_mut(&self.id) {
            entry.sending = false;
            if entry.over() {
                streams.remove(&self.id);
            }
        }
    }
}

impl AsyncWrite for PipeSend {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = &mut *self;
        if this.finished || this.dead() {
            return Poll::Ready(Err(stream_over()));
        }
        if buffer.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let waiting = this.waiting.get_or_insert_with(|| {
            let (credit, room) = (Arc::clone(&this.credit), Arc::clone(&this.room));
            Box::pin(async move {
                // The reader's credit first: a stream whose reader is slow
                // takes no place among its priority's frames meanwhile.
                credit.acquire_owned().await?.forget();
                room.acquire_owned().await?.forget();
                Ok(())
            })
        });
        match waiting.as_mut().poll(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(waited) => {
                this.waiting = None;
                if waited.is_err() {
                    return Poll::Ready(Err(stream_over()));
                }
                let taken = buffer.len().min(MAX_CHUNK);
                let sent = frame(PEER_DATA, this.id, &buffer[..taken]);
                Poll::Ready(
                    this.shared
                        .outbox
                        .class(this.priority, sent, true)
                        .map(|()| taken),
                )
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.finish();
        Poll::Ready(Ok(()))
    }
}

impl Drop for PipeSend {
    fn drop(&mut self) {
        self.finish();
    }
}

/// A stream's reading half. Dropping it before the other end finished stops
/// the direction: the other end's next write fails.
pub struct PipeRecv {
    shared: Arc<Shared>,
    id: u32,
    queued: mpsc::Receiver<Vec<u8>>,
    /// The frame being read, and how far.
    chunk: Vec<u8>,
    at: usize,
    /// Frames taken whose credit has not been given back yet.
    owed: u32,
    finished: Arc<AtomicBool>,
    /// The direction's end was read.
    over: bool,
}

impl PipeRecv {
    /// Gives the writer back the credit of the frames taken, half a window
    /// at a time, so a stream of small frames costs one answer per eight.
    fn took_one(&mut self) {
        self.owed += 1;
        if self.owed as usize >= WINDOW / 2 {
            self.shared
                .tell_with(PEER_CREDIT, self.id, &self.owed.to_be_bytes());
            self.owed = 0;
        }
    }
}

impl AsyncRead for PipeRecv {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = &mut *self;
        loop {
            if this.at < this.chunk.len() {
                let taken = buffer.remaining().min(this.chunk.len() - this.at);
                buffer.put_slice(&this.chunk[this.at..this.at + taken]);
                this.at += taken;
                if this.at == this.chunk.len() {
                    this.chunk.clear();
                    this.at = 0;
                    this.took_one();
                }
                return Poll::Ready(Ok(()));
            }
            if this.over {
                return Poll::Ready(Ok(()));
            }
            match this.queued.poll_recv(context) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(chunk)) if chunk.is_empty() => this.took_one(),
                Poll::Ready(Some(chunk)) => {
                    this.chunk = chunk;
                    this.at = 0;
                }
                Poll::Ready(None) => {
                    this.over = true;
                    return Poll::Ready(match this.finished.load(Ordering::SeqCst) {
                        true => Ok(()),
                        false => Err(io::Error::new(
                            io::ErrorKind::ConnectionReset,
                            "the connection ended before the stream did",
                        )),
                    });
                }
            }
        }
    }
}

impl Drop for PipeRecv {
    fn drop(&mut self) {
        let stop = {
            let mut streams = lock(&self.shared.peer.streams);
            let Some(entry) = streams.get_mut(&self.id) else {
                return;
            };
            // Still open at this end: the other end is told to stop.
            let stop = entry.inbound.take().is_some();
            if entry.over() {
                streams.remove(&self.id);
            }
            stop
        };
        if stop {
            self.shared.tell(PEER_STOP, self.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_platform::ipc::LocalStream;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A hang guard, never a pause.
    const WITHIN: Duration = Duration::from_secs(20);

    async fn within<T>(what: &str, wait: impl Future<Output = T>) -> T {
        tokio::time::timeout(WITHIN, wait)
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    type Accepted = mpsc::Receiver<(PipeSend, PipeRecv)>;

    /// Two ends joined by a local pair, as a carrier joins two machines.
    fn joined() -> (Pipe, Accepted, Pipe, Accepted) {
        let (near, far) = LocalStream::pair().unwrap();
        let (dialer, dialer_accepts) = Pipe::start(near.into(), true).unwrap();
        let (acceptor, acceptor_accepts) = Pipe::start(far.into(), false).unwrap();
        (dialer, dialer_accepts, acceptor, acceptor_accepts)
    }

    async fn until(what: &str, done: impl Fn() -> bool) {
        within(what, async {
            while !done() {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }

    /// Either end opens a stream, bytes cross both ways, and each direction
    /// ends on its own: a finished writer is an end of stream for its
    /// reader while the other direction goes on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn either_end_opens_a_stream_and_each_direction_ends_on_its_own() {
        let (dialer, mut dialer_accepts, acceptor, mut acceptor_accepts) = joined();
        let (mut send, mut recv) = dialer.open(0).unwrap();
        send.write_all(b"ask").await.unwrap();
        send.shutdown().await.unwrap();
        let (mut their_send, mut their_recv) =
            within("the stream", acceptor_accepts.recv()).await.unwrap();
        let mut asked = Vec::new();
        within("the ask", their_recv.read_to_end(&mut asked))
            .await
            .unwrap();
        assert_eq!(asked, b"ask");
        // The other direction is still open after the first one ended.
        their_send.write_all(b"answer").await.unwrap();
        drop(their_send);
        let mut answered = Vec::new();
        within("the answer", recv.read_to_end(&mut answered))
            .await
            .unwrap();
        assert_eq!(answered, b"answer");
        drop((recv, their_recv));
        until("the stream's places", || {
            dialer.streams() == 0 && acceptor.streams() == 0
        })
        .await;

        // The accepting end opens one too, with an id of its own.
        let (mut send, _recv) = acceptor.open(0).unwrap();
        send.write_all(b"back").await.unwrap();
        let (_their_send, mut their_recv) =
            within("the stream", dialer_accepts.recv()).await.unwrap();
        let mut back = [0_u8; 4];
        within("the bytes", their_recv.read_exact(&mut back))
            .await
            .unwrap();
        assert_eq!(&back, b"back");
    }

    /// A reader that takes nothing holds its writer after a window, holds
    /// no other stream, and loses nothing: what was written arrives whole
    /// once it reads.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_slow_reader_holds_its_own_writer_and_no_other_stream() {
        let (dialer, _dialer_accepts, _acceptor, mut acceptor_accepts) = joined();
        let (mut slow, _slow_recv) = dialer.open(0).unwrap();
        let (_held_send, mut held_recv) = within("the slow stream", acceptor_accepts.recv())
            .await
            .unwrap();
        let chunk = vec![7_u8; MAX_CHUNK];
        let total = 4 * WINDOW * MAX_CHUNK;
        let writing = tokio::spawn(async move {
            for _ in 0..4 * WINDOW {
                slow.write_all(&chunk).await.unwrap();
            }
            slow.shutdown().await.unwrap();
        });
        // Another stream of the same priority crosses while the first one's
        // writer waits for its reader.
        let (mut other, _other_recv) = dialer.open(0).unwrap();
        other.write_all(b"through").await.unwrap();
        let (_other_send, mut other_recv) = within("the other stream", acceptor_accepts.recv())
            .await
            .unwrap();
        let mut through = [0_u8; 7];
        within(
            "the other stream's bytes",
            other_recv.read_exact(&mut through),
        )
        .await
        .unwrap();
        assert_eq!(&through, b"through");
        assert!(!writing.is_finished(), "the writer never waited for credit");
        let mut read = Vec::new();
        within("the held bytes", held_recv.read_to_end(&mut read))
            .await
            .unwrap();
        assert_eq!(read.len(), total);
        assert!(read.iter().all(|byte| *byte == 7));
        within("the writer", writing).await.unwrap();
    }

    /// A reader that goes away fails its writer's next write and frees the
    /// stream's place; the connection and its other streams go on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_reader_that_goes_away_fails_its_writer_and_frees_the_place() {
        let (dialer, _dialer_accepts, acceptor, mut acceptor_accepts) = joined();
        let (mut send, recv) = dialer.open(0).unwrap();
        send.write_all(b"x").await.unwrap();
        let (their_send, their_recv) = within("the stream", acceptor_accepts.recv()).await.unwrap();
        drop(their_recv);
        within("the writer's failure", async {
            let chunk = vec![0_u8; MAX_CHUNK];
            while send.write_all(&chunk).await.is_ok() {}
        })
        .await;
        drop((send, recv, their_send));
        until("the stream's places", || {
            dialer.streams() == 0 && acceptor.streams() == 0
        })
        .await;
        assert!(dialer.ended().is_none());
        let (mut send, _recv) = dialer.open(0).unwrap();
        send.write_all(b"on").await.unwrap();
        let (_their_send, mut their_recv) = within("the next stream", acceptor_accepts.recv())
            .await
            .unwrap();
        let mut on = [0_u8; 2];
        within("its bytes", their_recv.read_exact(&mut on))
            .await
            .unwrap();
    }

    /// A close says why to the other end, and ends every stream there: a
    /// reader that was not finished reads a loss, never an end of stream.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_close_is_read_with_its_code_and_ends_every_stream() {
        let (dialer, _dialer_accepts, acceptor, mut acceptor_accepts) = joined();
        let (mut send, _recv) = dialer.open(0).unwrap();
        send.write_all(b"x").await.unwrap();
        let (mut their_send, mut their_recv) =
            within("the stream", acceptor_accepts.recv()).await.unwrap();
        let mut closed = acceptor.closed();
        dialer.close(7);
        assert_eq!(dialer.ended(), Some(7));
        let code = within("the close", closed.wait_for(Option::is_some))
            .await
            .map(|code| *code)
            .unwrap();
        assert_eq!(code, Some(7));
        let mut rest = Vec::new();
        let read = within("the reader's end", their_recv.read_to_end(&mut rest)).await;
        assert_eq!(read.unwrap_err().kind(), io::ErrorKind::ConnectionReset);
        assert!(their_send.write_all(b"y").await.is_err());
        assert!(acceptor.open(0).is_err());
        assert!(
            within("no more streams", acceptor_accepts.recv())
                .await
                .is_none()
        );
    }

    /// A carrier that ends with no word reads as lost on the end that is
    /// left.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_carrier_that_just_ends_reads_as_lost() {
        let (near, far) = LocalStream::pair().unwrap();
        let (dialer, _accepts) = Pipe::start(near.into(), true).unwrap();
        let mut closed = dialer.closed();
        far.shutdown_handle().shutdown();
        let code = within("the end", closed.wait_for(Option::is_some))
            .await
            .map(|code| *code)
            .unwrap();
        assert_eq!(code, Some(lost()));
    }

    /// An end that speaks other frames, or none, is not a connection: the
    /// first frame decides.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_end_that_speaks_other_frames_is_refused_at_its_first() {
        use std::io::Write;
        for first in [
            frame(HELLO, 0, &[VERSION + 1]),
            frame(PEER_OPEN, 1, &[0; 4]),
        ] {
            let (near, mut far) = LocalStream::pair().unwrap();
            let (acceptor, _accepts) = Pipe::start(near.into(), false).unwrap();
            let mut closed = acceptor.closed();
            far.write_all(&first).unwrap();
            let code = within("the refusal", closed.wait_for(Option::is_some))
                .await
                .map(|code| *code)
                .unwrap();
            assert_eq!(code, Some(lost()));
        }
    }

    /// An end's own stream past the cap is refused, and a place freed is a
    /// place for the next, which the other end takes though the freed
    /// stream's last frame is still behind the open.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stream_past_the_cap_is_refused_until_a_place_is_free() {
        let (dialer, _dialer_accepts, acceptor, mut acceptor_accepts) = joined();
        let mut held = Vec::new();
        let mut theirs = Vec::new();
        for _ in 0..MAX_STREAMS {
            held.push(dialer.open(0).unwrap());
            theirs.push(within("the stream", acceptor_accepts.recv()).await.unwrap());
        }
        assert_eq!(
            dialer.open(0).map(drop).unwrap_err().kind(),
            io::ErrorKind::OutOfMemory
        );
        // The other end's own streams are counted on their own.
        let (_send, _recv) = acceptor.open(0).unwrap();
        held.pop();
        let (mut send, _recv) = dialer.open(0).unwrap();
        send.write_all(b"x").await.unwrap();
        let (_their_send, mut their_recv) = within("the next stream", acceptor_accepts.recv())
            .await
            .unwrap();
        let mut taken = [0_u8; 1];
        within("its byte", their_recv.read_exact(&mut taken))
            .await
            .unwrap();
    }

    /// An end that opens past what this end holds a place for is refused
    /// stream by stream: both directions of the stream are over for it, and
    /// the connection goes on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_other_ends_stream_past_what_is_held_is_refused() {
        use std::io::{Read, Write};
        let (near, mut far) = LocalStream::pair().unwrap();
        let (acceptor, mut accepts) = Pipe::start(near.into(), false).unwrap();
        far.write_all(&frame(HELLO, 0, &[VERSION])).unwrap();
        let mut held = Vec::new();
        for opened in 0..MAX_THEIRS as u32 {
            far.write_all(&frame(PEER_OPEN, 2 * opened + 1, &[0; 4]))
                .unwrap();
            held.push(within("the stream", accepts.recv()).await.unwrap());
        }
        let past = 2 * MAX_THEIRS as u32 + 1;
        far.write_all(&frame(PEER_OPEN, past, &[0; 4])).unwrap();
        far.set_read_timeout(Some(WITHIN)).unwrap();
        let reading = tokio::task::spawn_blocking(move || {
            let mut said = Vec::new();
            while said.len() < 2 {
                let mut header = [0_u8; 9];
                far.read_exact(&mut header).unwrap();
                let stream = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
                let length = u32::from_be_bytes([header[5], header[6], header[7], header[8]]);
                far.read_exact(&mut vec![0_u8; length as usize]).unwrap();
                if stream == past {
                    said.push(header[0]);
                }
            }
            (said, far)
        });
        let (said, _far) = within("the refusal", reading).await.unwrap();
        assert_eq!(said, [PEER_STOP, PEER_FIN]);
        assert_eq!(acceptor.streams(), MAX_THEIRS);
        assert!(acceptor.ended().is_none());
    }

    /// An open the carrier already holds when the connection starts is
    /// taken: where the other end's streams go is in place before the
    /// thread that reads the carrier runs. The start here returns only once
    /// that thread has read all the carrier held, the order in which a
    /// stream was once refused for arriving first.
    #[test]
    fn an_open_the_carrier_holds_at_the_start_is_accepted() {
        /// The other end's greeting and first open, then a word that its
        /// reader came back for more (both frames are behind it then), and
        /// nothing until the carrier is ended.
        struct Holding {
            said: io::Cursor<Vec<u8>>,
            read: Option<std::sync::mpsc::Sender<()>>,
            ended: std::sync::mpsc::Receiver<()>,
        }

        impl io::Read for Holding {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                let held = io::Read::read(&mut self.said, buffer)?;
                if held > 0 {
                    return Ok(held);
                }
                if let Some(read) = self.read.take() {
                    let _ = read.send(());
                }
                // Ends when the carrier's end drops the sender.
                let _ = self.ended.recv();
                Ok(0)
            }
        }

        let mut said = frame(HELLO, 0, &[VERSION]);
        said.extend(frame(PEER_OPEN, 1, &0_i32.to_be_bytes()));
        let (read, drained) = std::sync::mpsc::channel();
        let (end, ended) = std::sync::mpsc::channel();
        let end = Mutex::new(Some(end));
        let carrier = Carrier::new(
            Holding {
                said: io::Cursor::new(said),
                read: Some(read),
                ended,
            },
            io::sink(),
            move || drop(lock(&end).take()),
        );
        let spawn = |name: &str, work: Box<dyn FnOnce() + Send>| {
            let started = spawn_thread(name, work)?;
            if name == "link-mux-frames" {
                drained
                    .recv_timeout(WITHIN)
                    .expect("the reader read what the carrier held");
            }
            Ok(started)
        };
        let (acceptor, mut accepts) = Pipe::start_on(carrier, false, &spawn).unwrap();
        let (send, _recv) = accepts
            .try_recv()
            .expect("the stream the carrier held is accepted");
        assert_eq!(send.id, 1);
        assert_eq!(acceptor.streams(), 1);
        acceptor.close(lost());
    }

    /// The other end's close says why in its last frame, and this end's
    /// writer can fail on the carrier that end already ended before its
    /// reader reads the frame. The connection still closes with the code:
    /// a failed write gives up writing and leaves the end to the reader.
    /// The reader here gets the frames only once the writer is done with
    /// the carrier, the order that read a close as a lost connection.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_close_is_read_with_its_code_though_a_write_failed_first() {
        /// A carrier's writing direction after the other end ended it;
        /// dropping it lets the reader on.
        struct Broken {
            _done: std::sync::mpsc::Sender<()>,
        }

        impl io::Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        /// What the other end wrote before it ended the carrier, read once
        /// the writer is done.
        struct Held {
            done: Option<std::sync::mpsc::Receiver<()>>,
            said: io::Cursor<Vec<u8>>,
        }

        impl io::Read for Held {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                if let Some(done) = self.done.take() {
                    // Ends when the writer is dropped.
                    let _ = done.recv();
                }
                io::Read::read(&mut self.said, buffer)
            }
        }

        let code = hide_peer_protocol::CloseCode::Replaced.number();
        let mut said = frame(HELLO, 0, &[VERSION]);
        said.extend(frame(BYE, 0, &code.to_be_bytes()));
        let (done, writer_done) = std::sync::mpsc::channel();
        let carrier = Carrier::new(
            Held {
                done: Some(writer_done),
                said: io::Cursor::new(said),
            },
            Broken { _done: done },
            || {},
        );
        let (acceptor, _accepts) = Pipe::start(carrier, false).unwrap();
        let mut closed = acceptor.closed();
        let read = within("the close", closed.wait_for(Option::is_some))
            .await
            .map(|code| *code)
            .unwrap();
        assert_eq!(read, Some(code));
    }

    /// A connection that closed opens no stream from the moment it has its
    /// code, though its last frame still waits to be written and nothing
    /// ended yet: the writer here is held at its first frame, the state in
    /// which an open once went through on a closed connection.
    #[tokio::test]
    async fn a_closed_connection_opens_no_stream_before_it_ended() {
        /// A carrier's writing direction that takes nothing until released,
        /// and fails then.
        struct Stuck {
            released: std::sync::mpsc::Receiver<()>,
        }

        impl io::Write for Stuck {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                // Ends when the test drops the sender.
                let _ = self.released.recv();
                Err(io::ErrorKind::BrokenPipe.into())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        /// Nothing to read until the carrier is ended.
        struct Silent {
            ended: std::sync::mpsc::Receiver<()>,
        }

        impl io::Read for Silent {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                let _ = self.ended.recv();
                Ok(0)
            }
        }

        let (release, released) = std::sync::mpsc::channel();
        let (end, ended) = std::sync::mpsc::channel();
        let end = Mutex::new(Some(end));
        let carrier = Carrier::new(Silent { ended }, Stuck { released }, move || {
            drop(lock(&end).take());
        });
        let (acceptor, _accepts) = Pipe::start(carrier, false).unwrap();
        let code = hide_peer_protocol::CloseCode::Replaced.number();
        acceptor.close(code);
        assert_eq!(acceptor.ended(), Some(code));
        assert!(
            !acceptor.shared.ended.load(Ordering::SeqCst),
            "nothing ended yet"
        );
        assert!(acceptor.open(0).is_err());
        assert_eq!(acceptor.streams(), 0);
        drop(release);
    }

    /// The other end names each stream's priority, and each priority keeps
    /// a place in the outbox while the connection lives: an open with a
    /// number no stream kind has is refused like one past the cap, and
    /// leaves nothing behind.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_other_ends_stream_of_a_priority_no_kind_has_is_refused() {
        use std::io::{Read, Write};
        let (near, mut far) = LocalStream::pair().unwrap();
        let (acceptor, mut accepts) = Pipe::start(near.into(), false).unwrap();
        far.write_all(&frame(HELLO, 0, &[VERSION])).unwrap();
        // More numbers than a connection has stream kinds.
        let strange: Vec<u32> = (0..64).map(|opened| 2 * opened + 1).collect();
        for (stream, priority) in strange.iter().zip(1_000_i32..) {
            far.write_all(&frame(PEER_OPEN, *stream, &priority.to_be_bytes()))
                .unwrap();
        }
        // One a kind has, opened after them, is the first to arrive.
        let kept = 2 * strange.len() as u32 + 1;
        far.write_all(&frame(PEER_OPEN, kept, &PROOF_PRIORITY.to_be_bytes()))
            .unwrap();
        let (send, _recv) = within("the stream", accepts.recv()).await.unwrap();
        assert_eq!(send.id, kept);
        far.set_read_timeout(Some(WITHIN)).unwrap();
        let wanted = strange.len();
        let reading = tokio::task::spawn_blocking(move || {
            let mut said = Vec::new();
            while said.len() < 2 * wanted {
                let mut header = [0_u8; 9];
                far.read_exact(&mut header).unwrap();
                let stream = u32::from_be_bytes([header[1], header[2], header[3], header[4]]);
                let length = u32::from_be_bytes([header[5], header[6], header[7], header[8]]);
                far.read_exact(&mut vec![0_u8; length as usize]).unwrap();
                if stream != 0 && stream != kept {
                    said.push((stream, header[0]));
                }
            }
            (said, far)
        });
        let (said, _far) = within("the refusals", reading).await.unwrap();
        let refused: Vec<(u32, u8)> = strange
            .iter()
            .flat_map(|stream| [(*stream, PEER_STOP), (*stream, PEER_FIN)])
            .collect();
        assert_eq!(said, refused);
        assert_eq!(acceptor.streams(), 1);
        let classes: Vec<i32> = lock(&acceptor.shared.outbox.state)
            .classes
            .keys()
            .copied()
            .collect();
        assert_eq!(classes, [PROOF_PRIORITY]);
        assert!(acceptor.ended().is_none());
        // This end's own open is refused before it is told.
        assert_eq!(
            acceptor.open(1_000).map(drop).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    /// A carrier's writing direction that hands each frame to the test, one
    /// at a time: the connection's writer waits in it until the test takes
    /// the frame, and `offered` counts the frames it has brought.
    struct Handed {
        frames: std::sync::mpsc::SyncSender<Vec<u8>>,
        offered: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl std::io::Write for Handed {
        fn write(&mut self, frame: &[u8]) -> io::Result<usize> {
            self.offered
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            self.frames
                .send(frame.to_vec())
                .map(|()| frame.len())
                .map_err(|_| io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Frames of a larger priority are written first: a key written after a
    /// flood that already waits is the next frame the writer takes, behind
    /// only the one it holds.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_larger_priority_is_written_first() {
        let (silent, _other_end) = LocalStream::pair().unwrap();
        let (frames, taken) = std::sync::mpsc::sync_channel(0);
        let offered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handed = Handed {
            frames,
            offered: Arc::clone(&offered),
        };
        let end = silent.shutdown_handle();
        let carrier = Carrier::new(silent, handed, move || end.shutdown());
        let (dialer, _accepts) = Pipe::start(carrier, true).unwrap();
        let next = move || {
            let frame: Vec<u8> = taken.recv_timeout(WITHIN).expect("the next frame");
            let stream = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
            (frame[0], stream)
        };
        assert_eq!(next(), (HELLO, 0));
        let (mut bulk, _bulk_recv) = dialer.open(-100).unwrap();
        let (mut keys, _keys_recv) = dialer.open(100).unwrap();
        assert_eq!(next(), (PEER_OPEN, 1));
        assert_eq!(next(), (PEER_OPEN, 3));
        // The flood spends its whole credit: one frame in the writer's
        // hands, its fourth after the hello and the two opens, and the rest
        // waiting behind it. The key is written only then: a writer that
        // had not yet taken a flood frame would take the key first.
        let flooding = tokio::spawn(async move {
            let chunk = vec![1_u8; MAX_CHUNK];
            while bulk.write_all(&chunk).await.is_ok() {}
        });
        let waiting = |priority: i32| {
            lock(&dialer.shared.outbox.state)
                .classes
                .get(&priority)
                .map_or(0, |class| class.waiting.len())
        };
        until("the writer to hold a flood frame", || {
            offered.load(std::sync::atomic::Ordering::Acquire) == 4
        })
        .await;
        until("the flood to wait", || waiting(-100) == WINDOW - 1).await;
        keys.write_all(b"k").await.unwrap();
        until("the key to wait", || waiting(100) == 1).await;
        assert_eq!(next(), (PEER_DATA, 1));
        assert_eq!(next(), (PEER_DATA, 3), "the key waited behind the flood");
        assert_eq!(next(), (PEER_DATA, 1));
        dialer.close(0);
        drop(next);
        let _ = within("the flood's end", flooding).await;
    }
}
