//! The in-process transport (PRD D-63): two machines in one process.
//!
//! Pairing, grants and delivery are tested on it, with no network and no
//! timing of one. A [`Net`] is the wire; each machine joins it with its key.
//! Nothing below a loopback connection authenticates, so the first exchange
//! is a challenge each way signed with the machine key ([`crate::proof`]),
//! the same proof a `wsl.exe` carrier uses, admitted under the same caps as
//! any transport's: [`limits::MAX_ADMITTING`] proofs at once, each for
//! [`limits::HELLO_WITHIN`] at most.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use hide_peer_protocol::{Candidate, CloseCode, PathKind, PeerAddr, PeerKey};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::{Semaphore, mpsc, watch};

use crate::gate::{Gate, Guarded};
use crate::key::MachineKey;
use crate::transport::{DialBound, PeerConn, PeerTransport, RelayState, TransportError};
use crate::{limits, proof};

/// The bytes a loopback stream holds before its writer waits.
const STREAM_BUFFER: usize = 64 * 1024;

/// How many dials or streams wait to be accepted before the next one does.
const BACKLOG: usize = 16;

/// The generation a connection's streams run under while it is open; any
/// other ends them, as a closed QUIC connection ends its streams.
const OPEN: u64 = 1;

struct Dialing {
    proof: DuplexStream,
    conn: LoopConn,
}

/// The wire machines of one test share.
#[derive(Default)]
pub struct Net {
    machines: Mutex<HashMap<PeerKey, mpsc::Sender<Dialing>>>,
    next_id: AtomicU64,
    dials: AtomicU64,
}

impl Net {
    pub fn new() -> Arc<Self> {
        Arc::default()
    }

    /// Joins the wire as `key`'s machine. A machine that joins again under
    /// the same key takes the place of the earlier one, as a restarted
    /// daemon does.
    pub fn join(self: &Arc<Self>, key: MachineKey) -> Loopback {
        let (dialed, incoming) = mpsc::channel(BACKLOG);
        let (proved, proving) = mpsc::channel(BACKLOG);
        self.machines
            .lock()
            .expect("the wire's lock")
            .insert(key.public(), dialed);
        Loopback {
            net: Arc::clone(self),
            key: Arc::new(key),
            incoming: tokio::sync::Mutex::new((incoming, proving)),
            proved,
            admitting: Arc::new(Semaphore::new(limits::MAX_ADMITTING)),
            conns: Mutex::new(Vec::new()),
            starts_on: Mutex::new(PathKind::Local),
            unreachable: std::sync::atomic::AtomicBool::new(false),
            silent: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// How many dials reached a machine on this wire.
    pub fn dials(&self) -> u64 {
        self.dials.load(Ordering::Acquire)
    }
}

pub struct Loopback {
    net: Arc<Net>,
    key: Arc<MachineKey>,
    /// The dials that arrived, and the connections whose dialer proved its
    /// key, each proof in a task of its own.
    incoming: tokio::sync::Mutex<(mpsc::Receiver<Dialing>, mpsc::Receiver<LoopConn>)>,
    proved: mpsc::Sender<LoopConn>,
    /// The proofs that run. At the cap the next dial is dropped before any
    /// of its bytes are read.
    admitting: Arc<Semaphore>,
    conns: Mutex<Vec<Weak<Shared>>>,
    /// The path the connections this machine dials start on.
    starts_on: Mutex<PathKind>,
    /// This machine's dials reach nothing, as when no network carries them.
    unreachable: std::sync::atomic::AtomicBool,
    /// This machine's dials neither connect nor fail, as a dial whose
    /// packets are dropped.
    silent: std::sync::atomic::AtomicBool,
}

struct Shared {
    id: u64,
    /// Ends every stream of the connection when it closes.
    open: Arc<Gate>,
    closed: watch::Sender<Option<CloseCode>>,
    /// The path both ends read: [`PathKind::Local`] unless a test moved it.
    path: watch::Sender<PathKind>,
}

impl Shared {
    fn close(&self, code: CloseCode) {
        // The first close decides the code both sides read.
        self.closed.send_if_modified(|closed| {
            if closed.is_some() {
                return false;
            }
            *closed = Some(code);
            true
        });
        self.open.set(OPEN + 1);
    }

    fn code(&self) -> Option<CloseCode> {
        *self.closed.borrow()
    }
}

#[derive(Clone)]
pub struct LoopConn {
    shared: Arc<Shared>,
    peer: PeerKey,
    dialed: bool,
    opened: mpsc::Sender<DuplexStream>,
    accepting: Arc<tokio::sync::Mutex<mpsc::Receiver<DuplexStream>>>,
}

fn unreachable(why: impl Into<String>) -> TransportError {
    TransportError::Unreachable(why.into())
}

fn proof_failed(error: io::Error) -> TransportError {
    unreachable(format!("the key proof did not finish: {error}"))
}

impl Loopback {
    /// Moves every live connection of this machine onto `path`, as a
    /// transport does when a direct path opens beside the relay; both ends
    /// read it.
    pub fn move_path(&self, path: PathKind) {
        let conns = self.conns.lock().expect("the connection list's lock");
        for conn in conns.iter().filter_map(Weak::upgrade) {
            conn.path.send_replace(path);
        }
    }

    /// The connections this machine dials from now start on `path`, as one
    /// a transport proves over the relay before a direct path opens.
    pub fn dial_on(&self, path: PathKind) {
        *self.starts_on.lock().expect("the start path's lock") = path;
    }

    /// Whether this machine's dials reach nothing from now.
    pub fn set_unreachable(&self, unreachable: bool) {
        self.unreachable.store(unreachable, Ordering::Release);
    }

    /// Whether this machine's dials, from now, never answer.
    pub fn set_silent(&self, silent: bool) {
        self.silent.store(silent, Ordering::Release);
    }

    fn track(&self, conn: &LoopConn) {
        let mut conns = self.conns.lock().expect("the connection list's lock");
        conns.retain(|known| known.strong_count() > 0);
        conns.push(Arc::downgrade(&conn.shared));
    }
}

impl PeerTransport for Loopback {
    type Conn = LoopConn;

    fn key(&self) -> PeerKey {
        self.key.public()
    }

    fn candidates(&self) -> Vec<Candidate> {
        Vec::new()
    }

    async fn dial(&self, to: &PeerAddr, _bound: DialBound) -> Result<LoopConn, TransportError> {
        if self.unreachable.load(Ordering::Acquire) {
            return Err(unreachable("no network carries this machine's dials"));
        }
        if self.silent.load(Ordering::Acquire) {
            return std::future::pending().await;
        }
        let machine = self
            .net
            .machines
            .lock()
            .expect("the wire's lock")
            .get(&to.key)
            .cloned()
            .ok_or_else(|| unreachable("no machine with that key is on the wire"))?;
        let shared = Arc::new(Shared {
            id: self.net.next_id.fetch_add(1, Ordering::AcqRel) + 1,
            open: Gate::new(OPEN),
            closed: watch::channel(None).0,
            path: watch::channel(*self.starts_on.lock().expect("the start path's lock")).0,
        });
        let (to_them, from_us) = mpsc::channel(BACKLOG);
        let (to_us, from_them) = mpsc::channel(BACKLOG);
        let ours = LoopConn {
            shared: Arc::clone(&shared),
            peer: to.key,
            dialed: true,
            opened: to_them,
            accepting: Arc::new(tokio::sync::Mutex::new(from_them)),
        };
        let theirs = LoopConn {
            shared,
            peer: self.key(),
            dialed: false,
            opened: to_us,
            accepting: Arc::new(tokio::sync::Mutex::new(from_us)),
        };
        let (proof, their_proof) = tokio::io::duplex(256);
        machine
            .send(Dialing {
                proof: their_proof,
                conn: theirs,
            })
            .await
            .map_err(|_| unreachable("that machine left the wire"))?;
        self.net.dials.fetch_add(1, Ordering::AcqRel);
        let (mut recv, mut send) = tokio::io::split(proof);
        if proof::dialer(&self.key, &to.key, &mut send, &mut recv)
            .await
            .map_err(proof_failed)?
            != proof::Proof::Proved
        {
            return Err(unreachable(
                "the machine did not prove the key that was dialed",
            ));
        }
        self.track(&ours);
        Ok(ours)
    }

    async fn accept(&self) -> Result<LoopConn, TransportError> {
        let mut incoming = self.incoming.lock().await;
        let (dials, proving) = &mut *incoming;
        loop {
            let dialing = tokio::select! {
                conn = proving.recv() => {
                    // This transport holds a sender, so this is a connection.
                    let conn = conn.ok_or(TransportError::Closed)?;
                    self.track(&conn);
                    return Ok(conn);
                }
                dialing = dials.recv() => dialing.ok_or(TransportError::Closed)?,
            };
            // Each dialer proves its key in a task of its own, so one that
            // never finishes holds up no other, and is given up after
            // `HELLO_WITHIN`. One that fails its proof, or arrives while as
            // many run as may, is dropped; it never reaches the caller.
            let Ok(admitted) = Arc::clone(&self.admitting).try_acquire_owned() else {
                continue;
            };
            let key = Arc::clone(&self.key);
            let proved = self.proved.clone();
            tokio::spawn(async move {
                let _admitted = admitted;
                let proving = prove_dialer(&key, dialing);
                if let Ok(Ok(Some(conn))) =
                    tokio::time::timeout(limits::HELLO_WITHIN, proving).await
                {
                    let _ = proved.send(conn).await;
                }
            });
        }
    }

    async fn relay(&self) -> RelayState {
        RelayState::None
    }

    async fn close(&self) {
        self.net
            .machines
            .lock()
            .expect("the wire's lock")
            .retain(|key, _| *key != self.key());
        self.incoming.lock().await.0.close();
        let conns = std::mem::take(&mut *self.conns.lock().expect("the connection list's lock"));
        for conn in conns.iter().filter_map(Weak::upgrade) {
            conn.close(CloseCode::Shutdown);
        }
    }
}

/// The acceptor's half of the proof. `None` when the dialer did not prove
/// the key it sent.
async fn prove_dialer(key: &MachineKey, dialing: Dialing) -> io::Result<Option<LoopConn>> {
    let Dialing { proof, mut conn } = dialing;
    let (mut recv, mut send) = tokio::io::split(proof);
    let Some(their_key) = proof::acceptor(key, &mut send, &mut recv).await? else {
        return Ok(None);
    };
    // The key the dialer proved, not the one it was built with.
    conn.peer = their_key;
    Ok(Some(conn))
}

type Halves = (
    Guarded<WriteHalf<DuplexStream>>,
    Guarded<ReadHalf<DuplexStream>>,
);

impl LoopConn {
    fn halves(&self, stream: DuplexStream) -> Halves {
        let (recv, send) = tokio::io::split(stream);
        let open = &self.shared.open;
        (
            Guarded::new(send, Some(open.pass()), None),
            Guarded::new(recv, Some(open.pass()), None),
        )
    }

    fn over(&self) -> TransportError {
        TransportError::Ended(self.shared.code().unwrap_or(CloseCode::Lost))
    }
}

impl PeerConn for LoopConn {
    type Send = Guarded<WriteHalf<DuplexStream>>;
    type Recv = Guarded<ReadHalf<DuplexStream>>;

    fn peer(&self) -> PeerKey {
        self.peer
    }

    fn id(&self) -> u64 {
        self.shared.id
    }

    fn dialed(&self) -> bool {
        self.dialed
    }

    fn path(&self) -> PathKind {
        *self.shared.path.borrow()
    }

    async fn path_changed(&self, from: PathKind) -> PathKind {
        let mut path = self.shared.path.subscribe();
        loop {
            let now = *path.borrow_and_update();
            if now != from {
                return now;
            }
            if path.changed().await.is_err() {
                return std::future::pending().await;
            }
        }
    }

    async fn open(&self, _priority: i32) -> Result<Halves, TransportError> {
        if self.shared.code().is_some() {
            return Err(self.over());
        }
        let (ours, theirs) = tokio::io::duplex(STREAM_BUFFER);
        self.opened.send(theirs).await.map_err(|_| self.over())?;
        Ok(self.halves(ours))
    }

    async fn accept(&self) -> Result<Halves, TransportError> {
        let mut closed = self.shared.closed.subscribe();
        let mut accepting = self.accepting.lock().await;
        tokio::select! {
            stream = accepting.recv() => match stream {
                Some(stream) if self.shared.code().is_none() => Ok(self.halves(stream)),
                _ => Err(self.over()),
            },
            _ = closed.wait_for(Option::is_some) => Err(self.over()),
        }
    }

    fn close(&self, code: CloseCode) {
        self.shared.close(code);
    }

    fn ended(&self) -> Option<CloseCode> {
        self.shared.code()
    }

    async fn closed(&self) -> CloseCode {
        let mut closed = self.shared.closed.subscribe();
        match closed.wait_for(Option::is_some).await {
            Ok(code) => code.unwrap_or(CloseCode::Lost),
            Err(_) => CloseCode::Lost,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A dial whose dialer sends nothing of its proof; the end it keeps
    /// open is answered with it.
    async fn silent_dial(net: &Arc<Net>, host: &Loopback, as_key: PeerKey) -> DuplexStream {
        let (silent, their_proof) = tokio::io::duplex(256);
        let (opened, _never_accepted) = mpsc::channel(BACKLOG);
        let (_never_opened, accepting) = mpsc::channel(BACKLOG);
        let stuck = LoopConn {
            shared: Arc::new(Shared {
                id: net.next_id.fetch_add(1, Ordering::AcqRel) + 1,
                open: Gate::new(OPEN),
                closed: watch::channel(None).0,
                path: watch::channel(PathKind::Local).0,
            }),
            peer: as_key,
            dialed: false,
            opened,
            accepting: Arc::new(tokio::sync::Mutex::new(accepting)),
        };
        let to_host = net.machines.lock().unwrap()[&host.key()].clone();
        to_host
            .send(Dialing {
                proof: their_proof,
                conn: stuck,
            })
            .await
            .expect("the host is on the wire");
        silent
    }

    #[tokio::test]
    async fn a_dialer_that_never_finishes_its_proof_holds_up_no_other() {
        let net = Net::new();
        let host = net.join(MachineKey::generate().unwrap());
        let guest = net.join(MachineKey::generate().unwrap());
        let _silent = silent_dial(&net, &host, guest.key()).await;

        let to = PeerAddr::new(host.key(), Vec::new());
        let (dialed, accepted) = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::join!(guest.dial(&to, DialBound::start()), host.accept())
        })
        .await
        .expect("the second dialer connected beside the unfinished proof");
        assert_eq!(accepted.unwrap().peer(), guest.key());
        dialed.unwrap();
    }

    /// As many proofs run at once as may and no more: the dial past the cap
    /// is dropped unread, and each silent one is given up after
    /// `HELLO_WITHIN`, which frees its place.
    #[tokio::test(start_paused = true)]
    async fn proofs_are_admitted_under_a_count_and_a_time() {
        let net = Net::new();
        let host = Arc::new(net.join(MachineKey::generate().unwrap()));
        let guest = net.join(MachineKey::generate().unwrap());
        let accepting = {
            let host = Arc::clone(&host);
            tokio::spawn(async move { host.accept().await })
        };
        let mut silent = Vec::new();
        for _ in 0..limits::MAX_ADMITTING {
            silent.push(silent_dial(&net, &host, guest.key()).await);
        }
        // Every place is taken: an honest dialer is dropped, not queued.
        let to = PeerAddr::new(host.key(), Vec::new());
        while host.admitting.available_permits() > 0 {
            tokio::task::yield_now().await;
        }
        assert!(matches!(
            guest.dial(&to, DialBound::start()).await,
            Err(TransportError::Unreachable(_))
        ));
        // The silent ones are given up in time, and the next dial is taken:
        // the test's clock is moved past the bound, and every place comes
        // back.
        tokio::time::advance(limits::HELLO_WITHIN + Duration::from_secs(1)).await;
        while host.admitting.available_permits() < limits::MAX_ADMITTING {
            tokio::task::yield_now().await;
        }
        guest.dial(&to, DialBound::start()).await.unwrap();
        assert_eq!(accepting.await.unwrap().unwrap().peer(), guest.key());
    }
}
