//! The transport for a carrier that is one byte stream (PRD D-27, D-62,
//! D-63, D-79): a WSL distribution and its PC, joined by the pipe `wsl.exe`
//! gives a program it starts in the distribution.
//!
//! Only the PC makes such a carrier ([`PipeTransport::dialing`]); the
//! distribution's core takes the ones that arrive on its local stream
//! ([`PipeTransport::arrived`]). A connection's streams are the carrier's
//! multiplexed ones ([`crate::mux::Pipe`]).
//!
//! Nothing below it authenticates, so each connection's first stream
//! carries the machine-key proof ([`crate::proof`]) before the connection
//! is handed over. An arriving carrier is admitted as any transport's
//! connection is: [`limits::MAX_ADMITTING`] proofs run at once, each for
//! [`limits::HELLO_WITHIN`] at most, and one that arrives past the cap is
//! ended before any of its bytes are read.
//!
//! [`Beside`] puts this transport beside another, so one [`crate::Peers`]
//! holds a machine's connections whatever carries each: an address that
//! names a distribution ([`Candidate::Wsl`]) is dialed here, and every other
//! one there.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use hide_peer_protocol::{Candidate, CloseCode, PathKind, PeerAddr, PeerKey};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{Semaphore, mpsc, watch};

use crate::key::MachineKey;
use crate::mux::{Carrier, PROOF_PRIORITY, Pipe, PipeRecv, PipeSend};
use crate::transport::{PeerConn, PeerTransport, RelayState, TransportError};
use crate::{limits, proof};

/// Proved connections that wait to be accepted; one past it is closed
/// busy.
const BACKLOG: usize = 16;

/// The distribution `to` is reached through, when it names one.
pub fn distribution(to: &PeerAddr) -> Option<&str> {
    to.candidates.iter().find_map(|candidate| match candidate {
        Candidate::Wsl(name) => Some(name.as_str()),
        Candidate::Ip(_) | Candidate::Relay(_) => None,
    })
}

type Dialing = Pin<Box<dyn Future<Output = io::Result<Carrier>> + Send>>;

/// Makes the carrier to a machine, on a machine that can.
type Dial = Box<dyn Fn(&PeerAddr) -> Dialing + Send + Sync>;

type Accepted = mpsc::Receiver<(PipeSend, PipeRecv)>;

pub struct PipeTransport {
    key: Arc<MachineKey>,
    dial: Option<Dial>,
    runtime: tokio::runtime::Handle,
    /// The proofs of arrived carriers that run.
    admitting: Arc<Semaphore>,
    proved: mpsc::Sender<PipeConn>,
    proving: tokio::sync::Mutex<mpsc::Receiver<PipeConn>>,
    /// The connections that have not ended, for [`PeerTransport::close`].
    conns: Arc<Mutex<Vec<Pipe>>>,
    /// Counted down from the largest, so a transport this one stands
    /// beside, which counts up, never gives a connection the same number.
    next_id: Arc<AtomicU64>,
    closed: watch::Sender<bool>,
}

impl PipeTransport {
    /// A transport that only takes the carriers that arrive, as a
    /// distribution's core does. Called inside the runtime its proofs run
    /// on.
    pub fn accepting(key: MachineKey) -> Self {
        let (proved, proving) = mpsc::channel(BACKLOG);
        Self {
            key: Arc::new(key),
            dial: None,
            runtime: tokio::runtime::Handle::current(),
            admitting: Arc::new(Semaphore::new(limits::MAX_ADMITTING)),
            proved,
            proving: tokio::sync::Mutex::new(proving),
            conns: Arc::default(),
            next_id: Arc::new(AtomicU64::new(u64::MAX)),
            closed: watch::channel(false).0,
        }
    }

    /// A transport that also makes carriers, as the PC does: `dial` makes
    /// the one to the machine an address names, and is dropped when the
    /// dial is given up.
    pub fn dialing<F>(
        key: MachineKey,
        dial: impl Fn(&PeerAddr) -> F + Send + Sync + 'static,
    ) -> Self
    where
        F: Future<Output = io::Result<Carrier>> + Send + 'static,
    {
        Self {
            dial: Some(Box::new(move |to| Box::pin(dial(to)))),
            ..Self::accepting(key)
        }
    }

    fn is_closed(&self) -> bool {
        *self.closed.borrow()
    }

    /// A carrier a peer made arrived. Its proof runs in a task of its own,
    /// so one that never finishes holds up no other; past the cap, or once
    /// the transport closed, the carrier is ended unread.
    pub fn arrived(&self, carrier: Carrier) {
        if self.is_closed() {
            return carrier.end();
        }
        let Ok(admitted) = Arc::clone(&self.admitting).try_acquire_owned() else {
            return carrier.end();
        };
        let Ok((pipe, mut accepted)) = Pipe::start(carrier, false) else {
            return;
        };
        let key = Arc::clone(&self.key);
        let (proved, conns) = (self.proved.clone(), Arc::clone(&self.conns));
        let id = self.next_id.fetch_sub(1, Ordering::AcqRel);
        let closed = self.closed.subscribe();
        self.runtime.spawn(async move {
            let unproved = Unproved(Some(pipe));
            let proving = async {
                let (mut send, mut recv) = accepted.recv().await?;
                proof::acceptor(&key, &mut send, &mut recv).await.ok()?
            };
            let peer = tokio::time::timeout(limits::HELLO_WITHIN, proving).await;
            drop(admitted);
            let Ok(Some(peer)) = peer else {
                return;
            };
            let conn = PipeConn::new(unproved.proved(), peer, id, false, accepted);
            track(&conns, &conn.pipe);
            // Closed while the proof ran, or nobody accepts as fast as
            // carriers arrive.
            if *closed.borrow() {
                conn.close(CloseCode::Shutdown);
            } else if let Err(refused) = proved.try_send(conn) {
                refused.into_inner().close(CloseCode::Busy);
            }
        });
    }
}

fn track(conns: &Mutex<Vec<Pipe>>, pipe: &Pipe) {
    let mut conns = conns.lock().expect("the connection list's lock");
    conns.retain(|known| known.ended().is_none());
    conns.push(pipe.clone());
}

/// A carrier whose other end has not proved its key: given up, it is closed.
struct Unproved(Option<Pipe>);

impl Unproved {
    fn proved(mut self) -> Pipe {
        self.0.take().expect("held until proved")
    }
}

impl Drop for Unproved {
    fn drop(&mut self) {
        if let Some(pipe) = self.0.take() {
            pipe.close(CloseCode::Protocol.number());
        }
    }
}

fn unreachable(why: impl Into<String>) -> TransportError {
    TransportError::Unreachable(why.into())
}

impl PeerTransport for PipeTransport {
    type Conn = PipeConn;

    fn key(&self) -> PeerKey {
        self.key.public()
    }

    /// A machine is never told where a pipe leads: only its own PC makes
    /// one.
    fn candidates(&self) -> Vec<Candidate> {
        Vec::new()
    }

    async fn dial(&self, to: &PeerAddr) -> Result<PipeConn, TransportError> {
        if self.is_closed() {
            return Err(TransportError::Closed);
        }
        let dial = self
            .dial
            .as_ref()
            .ok_or_else(|| unreachable("this machine makes no carrier of this kind"))?;
        let carrier = dial(to)
            .await
            .map_err(|error| unreachable(format!("no carrier: {error}")))?;
        let (pipe, accepted) = Pipe::start(carrier, true)
            .map_err(|error| unreachable(format!("the carrier did not start: {error}")))?;
        let unproved = Unproved(Some(pipe.clone()));
        let failed =
            |error: io::Error| unreachable(format!("the key proof did not finish: {error}"));
        let (mut send, mut recv) = pipe.open(PROOF_PRIORITY).map_err(failed)?;
        if !proof::dialer(&self.key, &to.key, &mut send, &mut recv)
            .await
            .map_err(failed)?
        {
            return Err(unreachable(
                "the machine did not prove the key that was dialed",
            ));
        }
        let id = self.next_id.fetch_sub(1, Ordering::AcqRel);
        let conn = PipeConn::new(unproved.proved(), to.key, id, true, accepted);
        track(&self.conns, &conn.pipe);
        Ok(conn)
    }

    async fn accept(&self) -> Result<PipeConn, TransportError> {
        let mut closed = self.closed.subscribe();
        let mut proving = self.proving.lock().await;
        tokio::select! {
            // This transport holds a sender, so this is a connection.
            conn = proving.recv() => conn.ok_or(TransportError::Closed),
            _ = closed.wait_for(|closed| *closed) => Err(TransportError::Closed),
        }
    }

    async fn relay(&self) -> RelayState {
        RelayState::None
    }

    async fn close(&self) {
        self.closed.send_replace(true);
        let conns = std::mem::take(&mut *self.conns.lock().expect("the connection list's lock"));
        for pipe in conns {
            pipe.close(CloseCode::Shutdown.number());
        }
    }
}

#[derive(Clone)]
pub struct PipeConn {
    pipe: Pipe,
    peer: PeerKey,
    id: u64,
    dialed: bool,
    accepting: Arc<tokio::sync::Mutex<Accepted>>,
}

impl PipeConn {
    fn new(pipe: Pipe, peer: PeerKey, id: u64, dialed: bool, accepted: Accepted) -> Self {
        Self {
            pipe,
            peer,
            id,
            dialed,
            accepting: Arc::new(tokio::sync::Mutex::new(accepted)),
        }
    }

    fn over(&self) -> TransportError {
        TransportError::Ended(self.ended().unwrap_or(CloseCode::Lost))
    }
}

impl PeerConn for PipeConn {
    type Send = PipeSend;
    type Recv = PipeRecv;

    fn peer(&self) -> PeerKey {
        self.peer
    }

    fn id(&self) -> u64 {
        self.id
    }

    fn dialed(&self) -> bool {
        self.dialed
    }

    fn path(&self) -> PathKind {
        PathKind::Local
    }

    async fn open(&self, priority: i32) -> Result<(PipeSend, PipeRecv), TransportError> {
        self.pipe
            .open(priority)
            .map_err(|error| match self.ended() {
                Some(code) => TransportError::Ended(code),
                // Every place of this end's is taken; the connection goes on.
                None => unreachable(error.to_string()),
            })
    }

    async fn accept(&self) -> Result<(PipeSend, PipeRecv), TransportError> {
        let mut accepting = self.accepting.lock().await;
        accepting.recv().await.ok_or_else(|| self.over())
    }

    fn close(&self, code: CloseCode) {
        self.pipe.close(code.number());
    }

    fn ended(&self) -> Option<CloseCode> {
        self.pipe.ended().map(CloseCode::from_number)
    }

    async fn closed(&self) -> CloseCode {
        let mut closed = self.pipe.closed();
        match closed.wait_for(Option::is_some).await {
            Ok(code) => code.map_or(CloseCode::Lost, CloseCode::from_number),
            Err(_) => CloseCode::Lost,
        }
    }
}

/// A transport with a pipe transport beside it, as one: an address that
/// names a distribution is dialed over a pipe and every other one over
/// `main`, and a connection of either is accepted. The machine's addresses
/// and its relay are `main`'s.
pub struct Beside<T> {
    main: T,
    pipe: PipeTransport,
}

impl<T: PeerTransport> Beside<T> {
    /// Both carry the same machine key.
    pub fn new(main: T, pipe: PipeTransport) -> Self {
        assert!(main.key() == pipe.key(), "one machine, one key");
        Self { main, pipe }
    }

    pub fn main(&self) -> &T {
        &self.main
    }

    pub fn pipe(&self) -> &PipeTransport {
        &self.pipe
    }
}

impl<T: PeerTransport> PeerTransport for Beside<T> {
    type Conn = Either<T::Conn, PipeConn>;

    fn key(&self) -> PeerKey {
        self.main.key()
    }

    fn candidates(&self) -> Vec<Candidate> {
        self.main.candidates()
    }

    async fn dial(&self, to: &PeerAddr) -> Result<Self::Conn, TransportError> {
        match distribution(to) {
            Some(_) => self.pipe.dial(to).await.map(Either::Pipe),
            None => self.main.dial(to).await.map(Either::Main),
        }
    }

    async fn accept(&self) -> Result<Self::Conn, TransportError> {
        // Both waits keep nothing of their own between polls: a connection
        // the one that lost had ready is still there for the next call.
        tokio::select! {
            conn = self.main.accept() => conn.map(Either::Main),
            conn = self.pipe.accept() => conn.map(Either::Pipe),
        }
    }

    async fn relay(&self) -> RelayState {
        self.main.relay().await
    }

    async fn close(&self) {
        self.pipe.close().await;
        self.main.close().await;
    }
}

/// A connection, a stream's half or anything else of one of two transports.
#[derive(Clone)]
pub enum Either<M, P> {
    Main(M),
    Pipe(P),
}

impl<M: PeerConn, P: PeerConn> PeerConn for Either<M, P> {
    type Send = Either<M::Send, P::Send>;
    type Recv = Either<M::Recv, P::Recv>;

    fn peer(&self) -> PeerKey {
        match self {
            Self::Main(conn) => conn.peer(),
            Self::Pipe(conn) => conn.peer(),
        }
    }

    fn id(&self) -> u64 {
        match self {
            Self::Main(conn) => conn.id(),
            Self::Pipe(conn) => conn.id(),
        }
    }

    fn dialed(&self) -> bool {
        match self {
            Self::Main(conn) => conn.dialed(),
            Self::Pipe(conn) => conn.dialed(),
        }
    }

    fn path(&self) -> PathKind {
        match self {
            Self::Main(conn) => conn.path(),
            Self::Pipe(conn) => conn.path(),
        }
    }

    async fn open(&self, priority: i32) -> Result<(Self::Send, Self::Recv), TransportError> {
        match self {
            Self::Main(conn) => {
                let (send, recv) = conn.open(priority).await?;
                Ok((Either::Main(send), Either::Main(recv)))
            }
            Self::Pipe(conn) => {
                let (send, recv) = conn.open(priority).await?;
                Ok((Either::Pipe(send), Either::Pipe(recv)))
            }
        }
    }

    async fn accept(&self) -> Result<(Self::Send, Self::Recv), TransportError> {
        match self {
            Self::Main(conn) => {
                let (send, recv) = conn.accept().await?;
                Ok((Either::Main(send), Either::Main(recv)))
            }
            Self::Pipe(conn) => {
                let (send, recv) = conn.accept().await?;
                Ok((Either::Pipe(send), Either::Pipe(recv)))
            }
        }
    }

    fn close(&self, code: CloseCode) {
        match self {
            Self::Main(conn) => conn.close(code),
            Self::Pipe(conn) => conn.close(code),
        }
    }

    fn ended(&self) -> Option<CloseCode> {
        match self {
            Self::Main(conn) => conn.ended(),
            Self::Pipe(conn) => conn.ended(),
        }
    }

    async fn closed(&self) -> CloseCode {
        match self {
            Self::Main(conn) => conn.closed().await,
            Self::Pipe(conn) => conn.closed().await,
        }
    }
}

impl<M: AsyncRead + Unpin, P: AsyncRead + Unpin> AsyncRead for Either<M, P> {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Main(half) => Pin::new(half).poll_read(context, buffer),
            Self::Pipe(half) => Pin::new(half).poll_read(context, buffer),
        }
    }
}

impl<M: AsyncWrite + Unpin, P: AsyncWrite + Unpin> AsyncWrite for Either<M, P> {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Main(half) => Pin::new(half).poll_write(context, buffer),
            Self::Pipe(half) => Pin::new(half).poll_write(context, buffer),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Main(half) => Pin::new(half).poll_flush(context),
            Self::Pipe(half) => Pin::new(half).poll_flush(context),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Main(half) => Pin::new(half).poll_shutdown(context),
            Self::Pipe(half) => Pin::new(half).poll_shutdown(context),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::loopback::Net;
    use hide_platform::ipc::LocalStream;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A hang guard, never a pause.
    const WITHIN: Duration = Duration::from_secs(20);

    async fn within<T>(what: &str, wait: impl Future<Output = T>) -> T {
        tokio::time::timeout(WITHIN, wait)
            .await
            .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
    }

    fn key() -> MachineKey {
        MachineKey::generate().unwrap()
    }

    fn copy(key: &MachineKey) -> MachineKey {
        MachineKey::from_secret(&key.secret())
    }

    /// A PC whose every carrier arrives at `distribution`, as `wsl.exe`'s
    /// arrives at the distribution's core.
    fn pc(key: MachineKey, distribution: &Arc<PipeTransport>) -> PipeTransport {
        let distribution = Arc::clone(distribution);
        PipeTransport::dialing(key, move |_| {
            let (near, far) = LocalStream::pair().unwrap();
            distribution.arrived(far.into());
            async move { Ok(Carrier::from(near)) }
        })
    }

    fn in_wsl(key: PeerKey) -> PeerAddr {
        PeerAddr::new(key, vec![Candidate::Wsl("Ubuntu".to_owned())])
    }

    /// The PC dials, both ends know the other's proved key, and a stream
    /// either end opens crosses; a close is read with its code on the other
    /// end.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_pc_and_its_distribution_connect_and_open_streams_both_ways() {
        let distro = Arc::new(PipeTransport::accepting(key()));
        let pc = pc(key(), &distro);
        let to = in_wsl(distro.key());
        let (dialed, accepted) = within("the connection", async {
            tokio::join!(pc.dial(&to), distro.accept())
        })
        .await;
        let (ours, theirs) = (dialed.unwrap(), accepted.unwrap());
        assert_eq!((ours.peer(), theirs.peer()), (distro.key(), pc.key()));
        assert!(ours.dialed() && !theirs.dialed());
        assert_eq!(ours.path(), PathKind::Local);

        for (opener, taker) in [(&ours, &theirs), (&theirs, &ours)] {
            let (mut send, _recv) = opener.open(0).await.unwrap();
            send.write_all(b"over the pipe").await.unwrap();
            send.shutdown().await.unwrap();
            let (_send, mut recv) = within("the stream", taker.accept()).await.unwrap();
            let mut read = Vec::new();
            within("its bytes", recv.read_to_end(&mut read))
                .await
                .unwrap();
            assert_eq!(read, b"over the pipe");
        }

        ours.close(CloseCode::Replaced);
        assert_eq!(
            within("the close", theirs.closed()).await,
            CloseCode::Replaced
        );
        assert_eq!(ours.ended(), Some(CloseCode::Replaced));
        assert!(matches!(
            theirs.open(0).await,
            Err(TransportError::Ended(CloseCode::Replaced))
        ));
    }

    /// A distribution that holds another key than the one dialed is not
    /// connected to, and a machine that makes no carrier dials nothing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_machine_that_does_not_prove_the_dialed_key_is_not_connected_to() {
        let distro = Arc::new(PipeTransport::accepting(key()));
        let pc = pc(key(), &distro);
        let other = key().public();
        let refused = within("the refusal", pc.dial(&in_wsl(other))).await;
        assert!(matches!(refused, Err(TransportError::Unreachable(_))));
        let no_carrier = distro.dial(&in_wsl(pc.key())).await;
        assert!(matches!(no_carrier, Err(TransportError::Unreachable(_))));
    }

    /// As many arrived carriers prove at once as may and no more: one past
    /// the cap is ended unread, a silent one is given up after
    /// `HELLO_WITHIN`, and an honest dialer connects once a place is free.
    #[tokio::test]
    async fn arrived_carriers_are_admitted_under_a_count_and_a_time() {
        use std::io::Read;
        let distro = Arc::new(PipeTransport::accepting(key()));
        let mut silent = Vec::new();
        for _ in 0..limits::MAX_ADMITTING {
            let (near, far) = LocalStream::pair().unwrap();
            distro.arrived(far.into());
            silent.push(near);
        }
        assert_eq!(distro.admitting.available_permits(), 0);
        // Past the cap: ended before any of its bytes are read.
        let (mut near, far) = LocalStream::pair().unwrap();
        distro.arrived(far.into());
        near.set_read_timeout(Some(WITHIN)).unwrap();
        assert_eq!(near.read(&mut [0_u8; 1]).unwrap(), 0);

        // The time is the table's; the test only shortens the wait for it.
        // Each proof's task runs once first, so its wait started before the
        // clock is moved.
        tokio::task::yield_now().await;
        tokio::time::pause();
        tokio::time::advance(limits::HELLO_WITHIN + Duration::from_secs(1)).await;
        tokio::time::resume();
        within("the silent carriers to be given up", async {
            while distro.admitting.available_permits() < limits::MAX_ADMITTING {
                tokio::task::yield_now().await;
            }
        })
        .await;
        for mut near in silent {
            near.set_read_timeout(Some(WITHIN)).unwrap();
            let mut rest = Vec::new();
            // The greeting, then the end.
            near.read_to_end(&mut rest).unwrap();
        }
        let pc = pc(key(), &distro);
        let to = in_wsl(distro.key());
        let (dialed, accepted) = within("the connection", async {
            tokio::join!(pc.dial(&to), distro.accept())
        })
        .await;
        dialed.unwrap();
        accepted.unwrap();
    }

    /// Closing the transport ends its connections with the shutdown code,
    /// ends a waiting accept, and takes no carrier afterwards.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn closing_the_transport_ends_its_connections_and_takes_no_more() {
        use std::io::Read;
        let distro = Arc::new(PipeTransport::accepting(key()));
        let pc = pc(key(), &distro);
        let to = in_wsl(distro.key());
        let (dialed, accepted) = within("the connection", async {
            tokio::join!(pc.dial(&to), distro.accept())
        })
        .await;
        let (ours, _theirs) = (dialed.unwrap(), accepted.unwrap());
        let waiting = {
            let distro = Arc::clone(&distro);
            tokio::spawn(async move { distro.accept().await })
        };
        distro.close().await;
        assert_eq!(
            within("the close", ours.closed()).await,
            CloseCode::Shutdown
        );
        assert!(matches!(
            within("the accept", waiting).await.unwrap(),
            Err(TransportError::Closed)
        ));
        let (mut near, far) = LocalStream::pair().unwrap();
        distro.arrived(far.into());
        near.set_read_timeout(Some(WITHIN)).unwrap();
        assert_eq!(near.read(&mut [0_u8; 1]).unwrap(), 0);
    }

    /// Beside another transport, an address that names a distribution goes
    /// over the pipe and every other one over the main transport, and one
    /// accept takes a connection of either.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn beside_another_transport_an_address_picks_its_carrier() {
        let net = Net::new();
        let (pc_key, distro_key, far_key) = (key(), key(), key());
        let distro_pipe = Arc::new(PipeTransport::accepting(copy(&distro_key)));
        // The distribution is on no wire the PC's main transport reaches.
        let pc = Beside::new(net.join(copy(&pc_key)), pc(copy(&pc_key), &distro_pipe));
        let far = net.join(copy(&far_key));

        let to_distro = in_wsl(distro_key.public());
        let over_pipe = within("the pipe's connection", async {
            tokio::join!(pc.dial(&to_distro), distro_pipe.accept())
        })
        .await;
        assert!(matches!(over_pipe.0, Ok(Either::Pipe(_))));
        over_pipe.1.unwrap();
        let plain = PeerAddr::new(far_key.public(), Vec::new());
        let over_main = within("the main connection", async {
            tokio::join!(pc.dial(&plain), far.accept())
        })
        .await;
        assert!(matches!(over_main.0, Ok(Either::Main(_))));
        // Without the distribution's name the address is the main
        // transport's, which does not reach it.
        let unnamed = PeerAddr::new(distro_key.public(), Vec::new());
        assert!(pc.dial(&unnamed).await.is_err());

        // A machine that dials the PC arrives through the same accept.
        let to_pc = PeerAddr::new(pc_key.public(), Vec::new());
        let (dialed, accepted) = within("the dial to the PC", async {
            tokio::join!(far.dial(&to_pc), pc.accept())
        })
        .await;
        dialed.unwrap();
        assert!(matches!(accepted, Ok(Either::Main(_))));
    }
}
