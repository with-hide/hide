//! The in-process transport (PRD D-63): two machines in one process.
//!
//! Pairing, grants and delivery are tested on it, with no network and no
//! timing of one. A [`Net`] is the wire; each machine joins it with its key.
//! Nothing below a loopback connection authenticates, so the first exchange
//! is a challenge each way signed with the machine key ([`crate::key`]), the
//! same proof a `wsl.exe` carrier uses.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

use hide_peer_protocol::{Candidate, CloseCode, PathKind, PeerAddr, PeerKey};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf, WriteHalf};
use tokio::sync::{mpsc, watch};

use crate::gate::{Gate, Guarded};
use crate::key::{self, CHALLENGE_BYTES, MachineKey, PROOF_BYTES};
use crate::transport::{PeerConn, PeerTransport, RelayState, TransportError};

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
            conns: Mutex::new(Vec::new()),
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
    conns: Mutex<Vec<Weak<Shared>>>,
}

struct Shared {
    id: u64,
    /// Ends every stream of the connection when it closes.
    open: Arc<Gate>,
    closed: watch::Sender<Option<CloseCode>>,
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

    async fn dial(&self, to: &PeerAddr) -> Result<LoopConn, TransportError> {
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
        let (mut proof, their_proof) = tokio::io::duplex(256);
        machine
            .send(Dialing {
                proof: their_proof,
                conn: theirs,
            })
            .await
            .map_err(|_| unreachable("that machine left the wire"))?;
        self.net.dials.fetch_add(1, Ordering::AcqRel);

        // The dialer's half of the proof: its key and a challenge, then the
        // acceptor's key, challenge and proof, then its own proof.
        let asked = key::challenge().map_err(proof_failed)?;
        proof
            .write_all(self.key().as_bytes())
            .await
            .map_err(proof_failed)?;
        proof.write_all(&asked).await.map_err(proof_failed)?;
        let mut their_key = [0u8; 32];
        let mut their_challenge = [0u8; CHALLENGE_BYTES];
        let mut their_proof = [0u8; PROOF_BYTES];
        proof
            .read_exact(&mut their_key)
            .await
            .map_err(proof_failed)?;
        proof
            .read_exact(&mut their_challenge)
            .await
            .map_err(proof_failed)?;
        proof
            .read_exact(&mut their_proof)
            .await
            .map_err(proof_failed)?;
        let their_key = PeerKey::from_bytes(their_key);
        if their_key != to.key || !key::verify(&their_key, &asked, &self.key(), &their_proof) {
            return Err(unreachable(
                "the machine did not prove the key that was dialed",
            ));
        }
        proof
            .write_all(&key::prove(&self.key, &their_challenge, &their_key))
            .await
            .map_err(proof_failed)?;
        // The acceptor's verdict: one byte once it verified ours.
        proof
            .read_exact(&mut [0u8; 1])
            .await
            .map_err(proof_failed)?;
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
            // never finishes holds up no other. One that fails its proof is
            // dropped; it never reaches the caller.
            let key = Arc::clone(&self.key);
            let proved = self.proved.clone();
            tokio::spawn(async move {
                if let Ok(Some(conn)) = prove_dialer(&key, dialing).await {
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

/// The acceptor's half of the proof: the dialer's key and challenge, then
/// this machine's key, challenge and proof, then the dialer's proof. `None`
/// when the dialer did not prove the key it sent.
async fn prove_dialer(key: &MachineKey, dialing: Dialing) -> io::Result<Option<LoopConn>> {
    let Dialing {
        mut proof,
        mut conn,
    } = dialing;
    let mut their_key = [0u8; 32];
    let mut their_challenge = [0u8; CHALLENGE_BYTES];
    proof.read_exact(&mut their_key).await?;
    proof.read_exact(&mut their_challenge).await?;
    let their_key = PeerKey::from_bytes(their_key);
    let asked = key::challenge()?;
    proof.write_all(key.public().as_bytes()).await?;
    proof.write_all(&asked).await?;
    proof
        .write_all(&key::prove(key, &their_challenge, &their_key))
        .await?;
    let mut their_proof = [0u8; PROOF_BYTES];
    proof.read_exact(&mut their_proof).await?;
    if !key::verify(&their_key, &asked, &key.public(), &their_proof) {
        return Ok(None);
    }
    proof.write_all(&[1]).await?;
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
        PathKind::Local
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

    #[tokio::test]
    async fn a_dialer_that_never_finishes_its_proof_holds_up_no_other() {
        let net = Net::new();
        let host = net.join(MachineKey::generate().unwrap());
        let guest = net.join(MachineKey::generate().unwrap());

        // A dial whose dialer sends nothing of its proof, and keeps its end
        // open.
        let (_silent, their_proof) = tokio::io::duplex(256);
        let (opened, _never_accepted) = mpsc::channel(BACKLOG);
        let (_never_opened, accepting) = mpsc::channel(BACKLOG);
        let stuck = LoopConn {
            shared: Arc::new(Shared {
                id: net.next_id.fetch_add(1, Ordering::AcqRel) + 1,
                open: Gate::new(OPEN),
                closed: watch::channel(None).0,
            }),
            peer: guest.key(),
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

        let to = PeerAddr::new(host.key(), Vec::new());
        let (dialed, accepted) = tokio::time::timeout(Duration::from_secs(20), async {
            tokio::join!(guest.dial(&to), host.accept())
        })
        .await
        .expect("the second dialer connected beside the unfinished proof");
        assert_eq!(accepted.unwrap().peer(), guest.key());
        dialed.unwrap();
    }
}
