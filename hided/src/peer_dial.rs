//! This core's node on a machine it is paired with (PRD device-connect-iroh
//! D-61, D-77): the guest's side of [`crate::guests`].
//!
//! The core opens a paired machine as it opens any device, through its
//! connector, which asks here for the node's link
//! ([`hide_node::ssh::host::peer::PeerDial`]). On the connection the two
//! machines already keep, this asks the host on the management stream to
//! run this build's node, uploads the build when the host lacks it, and
//! opens the node link stream once the host says the node is ready. The
//! management stream stays open for as long as that link does, and on it
//! the node's box of events reaches this core: this side names how far it
//! recorded the box, records each event the node sends after that,
//! acknowledges it and only then applies it (D-66, `hide_peer::inbox`).

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use hide_node::ssh::host::HelperPackages;
use hide_node::ssh::host::guest_build::GuestBuild;
use hide_node::ssh::host::peer::{Opened, PeerDial};
use hide_node_link::device::{Cancel, EstablishError, InstallStop, RemoteError, RemoteStage, Wake};
use hide_peer::inbox::{Applied, Apply, Inbox, Recorded};
use hide_peer::protocol::build::FileMessage;
use hide_peer::protocol::event::Event;
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::manage::{Manage, NodeStop, Release};
use hide_peer::protocol::{PeerKey, StreamKind, frame, launch};
use hide_peer::{PeerEvent, limits, wire};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::runtime::Handle;
use tokio::sync::watch;

use crate::peer_host::{HostPeers, Link, PeerHost, Stream};
use crate::peer_inbox::{self, Events};

/// How long a machine that is not connected is waited for: the connection
/// is kept by the pairing, which dials it again by itself.
const CONNECTED_WITHIN: Duration = Duration::from_secs(10);

/// How long a WSL distribution the operator opened is waited for when it
/// was off: WSL's virtual machine starts cold, then the distribution's
/// core.
const WOKEN_WITHIN: Duration = Duration::from_secs(60);

/// How long a machine just connected has to say the grant it gives: one
/// frame after its hello. A machine that gives none says so too, so only
/// one that never says waits this long.
const GRANT_TOLD_WITHIN: Duration = Duration::from_secs(5);

/// How long the host has to say which platform it is.
const PLATFORM_WITHIN: Duration = Duration::from_secs(30);

/// How long the host has for each stretch that is its own: to say it lacks
/// the build, and, once it has the build, to bring the node up (the node's
/// own start, the folder another node may still hold, and one start again
/// after a crash). The upload between the two is not counted: it is bounded
/// by each of its writes and by [`PLACED_WITHIN`], and a build takes as long
/// as its link is slow.
const READY_WITHIN: Duration = Duration::from_secs(
    launch::READY_WITHIN_SECS
        + launch::FOLDER_HELD_RETRIES as u64 * launch::FOLDER_HELD_RETRY_SECS
        + 30,
);

/// How long the host has to check and place an uploaded build after its
/// last byte.
const PLACED_WITHIN: Duration = Duration::from_secs(120);

/// The bytes of a build sent in one write, each within the write's bound.
const UPLOAD_CHUNK: usize = 1024 * 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn failed(what: &str, error: impl std::fmt::Display) -> EstablishError {
    EstablishError::Helper(format!("{what}: {error}"))
}

/// How long an earlier link's management stream has to end, on both
/// machines, before the next one is opened in its place.
const HELD_ENDS_WITHIN: Duration = Duration::from_secs(2);

/// The management stream held for one host.
struct Held {
    close: watch::Sender<bool>,
    /// Done once the stream ended and the host let its place go.
    ended: tokio::task::JoinHandle<()>,
}

pub struct Dialer {
    /// The endpoint, handed in once it exists: the core is started before
    /// it.
    host: OnceLock<Arc<PeerHost>>,
    runtime: Handle,
    /// This build's version and, when it was published, its release.
    build: String,
    release: Option<Release>,
    /// What ends the management stream held for each host, so a new link
    /// to that host opens the one a connection carries.
    held: Mutex<HashMap<PeerKey, Held>>,
    /// This daemon's state folder, which holds each host's inbox.
    state_dir: PathBuf,
    /// What applies a node's events, handed in once the core exists. With
    /// none, no box is asked for and the node keeps its events.
    events: OnceLock<Arc<dyn Events>>,
}

impl Dialer {
    pub fn new(
        runtime: Handle,
        build: String,
        release: Option<Release>,
        state_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            host: OnceLock::new(),
            runtime,
            build,
            release,
            held: Mutex::new(HashMap::new()),
            state_dir,
            events: OnceLock::new(),
        })
    }

    /// What applies the events of this core's nodes. Given once, at the
    /// daemon's start.
    pub fn take_events(&self, events: Arc<dyn Events>) {
        let _ = self.events.set(events);
    }

    /// The endpoint this dialer opens streams on. Given once, at the
    /// daemon's start.
    pub fn attach(&self, host: Arc<PeerHost>) {
        let _ = self.host.set(host);
    }

    fn start(&self, build_id: Option<String>) -> Manage {
        Manage::NodeStart {
            build: self.build.clone(),
            build_id,
            release: self.release.clone(),
        }
    }

    async fn open_link(
        &self,
        host: &Arc<PeerHost>,
        key: PeerKey,
        device: &str,
        packages: HelperPackages,
        wake: Wake,
    ) -> Result<Opened, EstablishError> {
        let peers = host.open().await.map_err(|error| {
            failed(
                "This machine's connection to other Hides did not start",
                error,
            )
        })?;
        // Heard from before the link is read, so the grant its machine
        // tells is not missed between the two.
        let told = peers.events();
        // The operator opened a distribution of this PC: the dial that
        // keeps its connection may start it, and is made now rather than
        // at its next turn (B43). Anything else is only waited for.
        let woken = (wake == Wake::Allowed && peers.link(&key).is_none())
            .then(|| peers.host(&key))
            .flatten()
            .zip(host.wsl())
            .and_then(|(known, carriers)| {
                let named = hide_peer::protocol::PeerAddr::new(key, known.candidates);
                let distro = hide_peer::pipe::distribution(&named)?;
                carriers.may_wake(distro);
                Some(())
            });
        if woken.is_some() {
            peers.network_changed();
        }
        let within = match woken {
            Some(()) => WOKEN_WITHIN,
            None => CONNECTED_WITHIN,
        };
        // A distribution of this PC that does not run is off until the
        // operator opens it; any other machine only waits for its
        // connection, which `hide_peer` keeps dialing, so the core tries its
        // link again on its own schedule rather than leaving it stopped
        // after a sleep longer than this wait (D-34 (2), the fault run).
        let distribution = peers.host(&key).is_some_and(|known| {
            hide_peer::pipe::distribution(&hide_peer::protocol::PeerAddr::new(
                key,
                known.candidates,
            ))
            .is_some()
        });
        let link = connected(&peers, &key, within).await.map_err(|away| {
            if distribution {
                away
            } else {
                not_yet(device, away)
            }
        })?;
        if granted(&link, &key, told).await != GrantState::Active {
            return Err(EstablishError::Stopped(
                "That machine has not opened itself to this one, or paused it".to_owned(),
            ));
        }
        let name = peers
            .host(&key)
            .map_or_else(|| key.short(), |known| known.name);
        // A connection carries one management stream: the one an earlier
        // link to this host held ends before this one opens.
        let earlier = lock(&self.held).remove(&key);
        if let Some(earlier) = earlier {
            earlier.close.send_replace(true);
            // The host takes one such stream at a time, so the next opens
            // only once it let the earlier one go. A host that does not
            // answer in time refuses the next, and the link is tried again.
            let _ = tokio::time::timeout(HELD_ENDS_WITHIN, earlier.ended).await;
        }
        let Stream { send, recv, .. } = link
            .open(StreamKind::Management)
            .await
            .map_err(|error| failed("The management stream did not open", error))?;
        let mut management = Management::new(send, recv);

        management.ask(&self.start(None)).await?;
        let (os, arch) = tokio::time::timeout(PLATFORM_WITHIN, async {
            loop {
                if let Manage::NodePreparing { os, arch } = management.answer().await? {
                    break Ok::<_, EstablishError>((os, arch));
                }
            }
        })
        .await
        .map_err(|_| failed("That machine did not answer", "no platform within the wait"))??;

        // The build is read whole and digested: off the runtime.
        let build = tokio::task::spawn_blocking(move || packages.guest_build(&os, &arch))
            .await
            .map_err(|error| failed("This build could not be read", error))??;
        let id = build.id().to_owned();
        let placed = ready(
            &mut management,
            || self.start(Some(id.clone())),
            || upload(&link, &build),
        )
        .await?;

        let Stream { send, recv, .. } = link
            .open(StreamKind::NodeLink)
            .await
            .map_err(|error| failed("The node link did not open", error))?;
        let (close, closed) = watch::channel(false);
        let taking = self.events.get().map(|events| Taking {
            dir: hide_kit::layout::peer_inbox(&self.state_dir, &key),
            device: device.to_owned(),
            events: Arc::clone(events),
        });
        let ended = tokio::spawn(hold(management, key, closed.clone(), taking));
        lock(&self.held).insert(
            key,
            Held {
                close: close.clone(),
                ended,
            },
        );
        let (writing, _) =
            hide_peer::blocking::blocking(send, tokio::io::empty(), self.runtime.clone());
        Ok(Opened {
            reading: Box::new(LinkReader {
                recv,
                runtime: self.runtime.clone(),
                closed,
            }),
            writing: Box::new(writing),
            close: Box::new(move || {
                close.send_replace(true);
            }),
            name,
            placed,
        })
    }
}

impl PeerDial for Dialer {
    fn open(
        &self,
        key: &str,
        device: &str,
        packages: &HelperPackages,
        wake: Wake,
        cancel: &Cancel,
    ) -> Result<Opened, EstablishError> {
        let ended = || EstablishError::Helper("the connection attempt ended".to_owned());
        let key = PeerKey::from_hex(key)
            .map_err(|error| failed("This device names no paired machine", error))?;
        let host = self.host.get().ok_or_else(|| {
            failed(
                "This machine's connection to other Hides is not started",
                "the daemon is still starting",
            )
        })?;
        // A cancel ends the attempt at whatever it waits on.
        let (stop, mut stopped) = watch::channel(false);
        cancel.arm(move || {
            stop.send_replace(true);
        });
        let opened = self.runtime.block_on(async {
            tokio::select! {
                opened = self.open_link(host, key, device, packages.clone(), wake) => opened,
                _ = stopped.wait_for(|stopped| *stopped) => Err(ended()),
            }
        });
        cancel.disarm();
        match opened {
            Ok(opened) if cancel.is_cancelled() => {
                (opened.close)();
                Err(ended())
            }
            opened => opened,
        }
    }
}

/// A machine reached over the network that is not connected yet: a
/// connection failure the core tries again, in the same words.
fn not_yet(device: &str, away: EstablishError) -> EstablishError {
    match away {
        EstablishError::Stopped(words) => EstablishError::Connect(RemoteError::new(
            "peer-connection",
            device,
            RemoteStage::Reconnect,
            words,
            true,
            false,
        )),
        other => other,
    }
}

/// The connection to `key`, waited for `within` while the pairing brings
/// it up.
async fn connected(
    peers: &HostPeers,
    key: &PeerKey,
    within: Duration,
) -> Result<Link, EstablishError> {
    let mut events = peers.events();
    if let Some(link) = peers.link(key) {
        return Ok(link);
    }
    let waited = tokio::time::timeout(within, async {
        loop {
            match events.recv().await {
                Ok(PeerEvent::Connected { key: connected, .. }) if connected == *key => break,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    if peers.link(key).is_some() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
    .await;
    match (waited, peers.link(key)) {
        (_, Some(link)) => Ok(link),
        _ => Err(EstablishError::Stopped(
            "That machine is not connected; it connects by itself when it is reachable".to_owned(),
        )),
    }
}

/// The grant `link`'s machine gives this one, once that machine said it. A
/// link starts with none and hears the grant in a frame after the hello,
/// so a dial made as the connection comes up, as this daemon's start makes
/// it, can be ahead of that frame: it waits for the frame rather than read
/// a machine that opened itself as one that did not.
async fn granted(
    link: &Link,
    key: &PeerKey,
    mut told: tokio::sync::broadcast::Receiver<PeerEvent>,
) -> GrantState {
    let _ = tokio::time::timeout(GRANT_TOLD_WITHIN, async {
        while link.granted() == GrantState::None {
            match told.recv().await {
                Ok(PeerEvent::Granted { key: of, .. }) if of == *key => break,
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    })
    .await;
    link.granted()
}

/// A management stream as this side holds it: the half it writes, and the
/// host's messages through one reader, which keeps what it has of a message
/// when a wait on it is dropped (the held stream waits on it beside its own
/// close and its inbox's clock).
struct Management<S, R> {
    send: S,
    messages: wire::MessageReader<R>,
}

impl<S, R> Management<S, R>
where
    S: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    fn new(send: S, recv: R) -> Self {
        Self {
            send,
            messages: wire::MessageReader::new(recv, frame::MAX_MESSAGE),
        }
    }

    async fn ask(&mut self, message: &Manage) -> Result<(), EstablishError> {
        wire::write_message(&mut self.send, message)
            .await
            .map_err(|error| failed("That machine took no management message", error))
    }

    /// The next management message this build knows. A wait on it that is
    /// dropped loses nothing.
    async fn answer(&mut self) -> Result<Manage, EstablishError> {
        loop {
            let body = self
                .messages
                .next()
                .await
                .map_err(|error| failed("The management stream failed", error))?
                .ok_or_else(|| failed("The management stream ended", "that machine closed it"))?;
            match Manage::parse(&body) {
                Ok(Some(message)) => return Ok(message),
                // A message of a later build.
                Ok(None) => {}
                Err(error) => {
                    return Err(failed("That machine sent no management message", error));
                }
            }
        }
    }
}

/// Has the host bring this build's node up: asks with `start`, sends the
/// build with `upload` once when the host says it lacks it, and waits until
/// the host says the node is ready. True when the build was sent.
///
/// The host has [`READY_WITHIN`] for each stretch that is its own, from the
/// ask and again from the end of the upload; the upload's own time is not
/// counted, so a build sent over a slow link is not given up on for the
/// link's speed.
async fn ready<S, R, U>(
    management: &mut Management<S, R>,
    start: impl Fn() -> Manage,
    upload: impl FnOnce() -> U,
) -> Result<bool, EstablishError>
where
    S: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
    U: Future<Output = Result<(), EstablishError>>,
{
    management.ask(&start()).await?;
    let mut upload = Some(upload);
    let mut by = tokio::time::Instant::now() + READY_WITHIN;
    loop {
        let said = tokio::time::timeout_at(by, management.answer())
            .await
            .map_err(|_| {
                failed(
                    "This Hide's node there did not start",
                    "not ready within the wait",
                )
            })??;
        match said {
            Manage::NodeReady { .. } => return Ok(upload.is_none()),
            Manage::NodeStopped {
                reason: NodeStop::BuildMissing,
                detail,
                ..
            } => match upload.take() {
                Some(upload) => {
                    upload().await?;
                    management.ask(&start()).await?;
                    by = tokio::time::Instant::now() + READY_WITHIN;
                }
                // The host lacks a build it just said it placed.
                None => return Err(stopped(NodeStop::BuildMissing, detail)),
            },
            // The host starts it again by itself.
            Manage::NodeStopped { retry: true, .. } => {}
            Manage::NodeStopped { reason, detail, .. } => return Err(stopped(reason, detail)),
            _ => {}
        }
    }
}

/// Why the host does not run the node, as the device row says it.
fn stopped(reason: NodeStop, detail: Option<String>) -> EstablishError {
    let detail = detail.unwrap_or_default();
    match reason {
        NodeStop::UpdateNeeded => EstablishError::Unsupported(format!(
            "That machine's Hide cannot run this Hide's node; update it there. {detail}"
        )),
        NodeStop::FolderRefused => EstablishError::Unsupported(format!(
            "That machine could not make this Hide's folder there. {detail}"
        )),
        NodeStop::BuildMissing => EstablishError::install(
            InstallStop::Refused,
            format!("That machine does not have this Hide's build and did not take it. {detail}"),
        ),
        NodeStop::HostFull => EstablishError::Unsupported(format!(
            "That machine already runs as many other machines' work as it carries at once; end one's work there first. {detail}"
        )),
        NodeStop::Crashed => EstablishError::Helper(format!(
            "This Hide's node there crashed too often and is left stopped. {detail}"
        )),
        NodeStop::Ended | NodeStop::Other => {
            EstablishError::Helper(format!("This Hide's node there is not running. {detail}"))
        }
    }
}

/// Sends the host `build`: the list of its files, then each file's bytes
/// in that order, and waits for the host to say it is placed.
async fn upload(link: &Link, build: &GuestBuild) -> Result<(), EstablishError> {
    // A wait that ran out is the machine answering nothing; any other
    // failure is the connection ending part way.
    let sent = |error: io::Error| {
        let stop = match error.kind() {
            io::ErrorKind::TimedOut => InstallStop::Quiet,
            _ => InstallStop::Ended,
        };
        EstablishError::install(
            stop,
            format!("This build did not reach that machine: {error}"),
        )
    };
    let mut stream = link.open(StreamKind::File).await.map_err(|error| {
        EstablishError::install(
            InstallStop::Ended,
            format!("The file stream did not open: {error}"),
        )
    })?;
    wire::write_message(
        &mut stream.send,
        &FileMessage::Build {
            id: build.id().to_owned(),
            files: build.files().to_vec(),
        },
    )
    .await
    .map_err(sent)?;
    for index in 0..build.files().len() {
        for chunk in build.bytes(index).unwrap_or_default().chunks(UPLOAD_CHUNK) {
            tokio::time::timeout(limits::WRITE_WITHIN, stream.send.write_all(chunk))
                .await
                .map_err(|_| sent(io::ErrorKind::TimedOut.into()))?
                .map_err(sent)?;
        }
    }
    stream.send.flush().await.map_err(sent)?;
    let body = tokio::time::timeout(
        PLACED_WITHIN,
        wire::read_message(&mut stream.recv, frame::MAX_MESSAGE),
    )
    .await
    .map_err(|_| sent(io::ErrorKind::TimedOut.into()))?
    .map_err(sent)?;
    match body.as_deref().map(FileMessage::parse) {
        Some(Ok(Some(FileMessage::BuildPlaced { .. }))) => Ok(()),
        Some(Ok(Some(FileMessage::Refused { reason }))) => Err(EstablishError::install(
            InstallStop::Refused,
            format!("That machine refused this Hide's build: {reason}"),
        )),
        None => Err(EstablishError::install(
            InstallStop::Ended,
            "That machine closed the stream before it said whether it placed this Hide's build",
        )),
        Some(_) => Err(EstablishError::install(
            InstallStop::Refused,
            "That machine did not say whether it placed this Hide's build",
        )),
    }
}

/// What one host's events are recorded in and applied through.
struct Taking {
    /// The host's inbox folder.
    dir: PathBuf,
    /// The device id the core knows the host's panes under.
    device: String,
    events: Arc<dyn Events>,
}

/// One connection's use of a host's inbox. Every step is a durable write
/// or a call into the core, so each runs off the runtime.
struct Taken {
    inbox: Arc<Mutex<Inbox>>,
    device: String,
    events: Arc<dyn Events>,
    /// Until when the recorded events wait for the core to know what they
    /// are for; `None` while none waits.
    waits_until: Option<Instant>,
}

/// [`Events`] as the inbox applies through it, for one pass.
struct Pass<'a> {
    events: &'a dyn Events,
    device: &'a str,
    last: bool,
}

impl Apply for Pass<'_> {
    fn apply(&mut self, event: &Event) -> Applied {
        self.events.apply(self.device, event, self.last)
    }
}

impl Taken {
    /// Opens the host's inbox. What an earlier connection recorded and did
    /// not apply is in it again.
    async fn open(taking: Taking) -> io::Result<Self> {
        let Taking {
            dir,
            device,
            events,
        } = taking;
        let inbox = tokio::task::spawn_blocking(move || Inbox::open(&dir))
            .await
            .map_err(io::Error::other)??;
        Ok(Self {
            inbox: Arc::new(Mutex::new(inbox)),
            device,
            events,
            waits_until: None,
        })
    }

    /// Applies what is recorded, in order, as far as the core can take it
    /// now. An event the core cannot place yet waits, with those after
    /// it, for [`peer_inbox::PANES_WITHIN`] from its first try: the core
    /// knows a pane only once the node's link brought it back.
    async fn apply(&mut self) -> io::Result<()> {
        let (inbox, events, device) = (
            Arc::clone(&self.inbox),
            Arc::clone(&self.events),
            self.device.clone(),
        );
        let last = self
            .waits_until
            .is_some_and(|until| Instant::now() >= until);
        let waiting = tokio::task::spawn_blocking(move || {
            let mut inbox = lock(&inbox);
            let mut pass = Pass {
                events: events.as_ref(),
                device: &device,
                last,
            };
            inbox.apply_recorded(&mut pass)?;
            io::Result::Ok(inbox.unapplied() > 0)
        })
        .await
        .map_err(io::Error::other)??;
        self.waits_until = waiting.then(|| {
            self.waits_until
                .filter(|_| !last)
                .unwrap_or_else(|| Instant::now() + peer_inbox::PANES_WITHIN)
        });
        Ok(())
    }

    async fn record(&self, event: Event) -> io::Result<Recorded> {
        let inbox = Arc::clone(&self.inbox);
        tokio::task::spawn_blocking(move || lock(&inbox).record(event))
            .await
            .map_err(io::Error::other)?
    }

    /// How far the host's box is recorded: what the host is told to send
    /// from.
    fn resume(&self) -> Manage {
        let inbox = lock(&self.inbox);
        let (outbox, seq) = inbox.position();
        Manage::Resume {
            outbox: outbox.to_owned(),
            seq,
        }
    }

    /// Records one event the host sent. What the host is answered: the
    /// acknowledgement, or this side's place again when the event is not
    /// the next one or there is no room for it.
    async fn take(&self, event: Event) -> io::Result<Manage> {
        let outbox = event.outbox.clone();
        Ok(match self.record(event).await? {
            Recorded::New { seq } | Recorded::Again { seq } => Manage::EventAck { outbox, seq },
            Recorded::Gap { .. } | Recorded::Full => self.resume(),
        })
    }
}

/// Keeps a host's management stream open while its node link is, and ends
/// it as the host hears an end: this side stops writing and reads until
/// the host let the stream go, so the host's place for the next one is
/// free when this returns.
async fn hold<S, R>(
    mut management: Management<S, R>,
    key: PeerKey,
    closed: watch::Receiver<bool>,
    taking: Option<Taking>,
) where
    S: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    held(&mut management, key, closed, taking).await;
    let _ = tokio::time::timeout(HELD_ENDS_WITHIN, async {
        let _ = management.send.shutdown().await;
        // Through the same reader: the held wait may have left it part way
        // into a message.
        while let Ok(Some(_)) = management.messages.next().await {}
    })
    .await;
}

/// What a held management stream does: records what the host says of the
/// node, and takes the node's box of events. The link itself ends when
/// the node does; this only says why.
async fn held<S, R>(
    management: &mut Management<S, R>,
    key: PeerKey,
    mut closed: watch::Receiver<bool>,
    taking: Option<Taking>,
) where
    S: AsyncWrite + Unpin,
    R: AsyncRead + Unpin,
{
    let failed = |step: &str, error: &dyn std::fmt::Display| {
        hide_core::diagnostic!(json!({
            "component": "peer_dial",
            "kind": "peer_dial.inbox_failed",
            "host": key.short(),
            "step": step,
            "error": error.to_string(),
        }));
    };
    // An inbox that cannot be read asks for nothing: the node keeps its
    // events for a connection that can record them.
    let mut taken = match taking {
        Some(taking) => Taken::open(taking)
            .await
            .map_err(|error| failed("open", &error))
            .ok(),
        None => None,
    };
    if let Some(open) = &taken
        && management.ask(&open.resume()).await.is_err()
    {
        return;
    }
    // What an earlier connection recorded and did not apply comes first.
    let mut apply = taken.is_some();
    loop {
        if std::mem::take(&mut apply)
            && let Some(open) = taken.as_mut()
            && let Err(error) = open.apply().await
        {
            failed("apply", &error);
            taken = None;
        }
        let waits = taken
            .as_ref()
            .is_some_and(|open| open.waits_until.is_some());
        let message = tokio::select! {
            // Dropped whenever another branch wins: the reader keeps what
            // it has of the message.
            message = management.answer() => message,
            _ = closed.wait_for(|closed| *closed) => break,
            _ = tokio::time::sleep(peer_inbox::LOOK_AGAIN), if waits => {
                apply = true;
                continue;
            }
        };
        let said = match message {
            Ok(Manage::NodeStopped {
                reason,
                retry,
                detail,
            }) => {
                hide_core::diagnostic!(json!({
                    "component": "peer_dial",
                    "kind": "peer_dial.node_stopped",
                    "host": key.short(),
                    "reason": format!("{reason:?}"),
                    "retry": retry,
                    "detail": detail,
                }));
                None
            }
            // A node the host started again knows no one who listens.
            Ok(Manage::NodeReady { .. }) => taken.as_ref().map(Taken::resume),
            Ok(Manage::Event(event)) => match &taken {
                Some(open) => match open.take(event).await {
                    // Applied once the answer is on its way.
                    Ok(said) => {
                        apply = true;
                        Some(said)
                    }
                    Err(error) => {
                        failed("record", &error);
                        taken = None;
                        None
                    }
                },
                None => None,
            },
            Ok(_) => None,
            Err(_) => break,
        };
        if let Some(said) = said
            && management.ask(&said).await.is_err()
        {
            break;
        }
    }
}

/// The node link's receiving half for the blocking reader a link runs on
/// (`hide_node::ssh::host::over_stream`): each read runs the stream's own
/// on the runtime, and the link's close ends a read that waits.
struct LinkReader<R> {
    recv: R,
    runtime: Handle,
    closed: watch::Receiver<bool>,
}

impl<R: AsyncRead + Unpin> Read for LinkReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let (recv, closed) = (&mut self.recv, &mut self.closed);
        self.runtime.block_on(async {
            tokio::select! {
                read = recv.read(buffer) => read,
                _ = closed.wait_for(|closed| *closed) => Ok(0),
            }
        })
    }
}

/// This side's management stream over an in-memory stream. These tests
/// start no program, so they run on every system.
#[cfg(test)]
mod stream_tests {
    use tokio::sync::mpsc;

    use super::*;
    use crate::peer_host::fixture::within;

    fn start() -> Manage {
        Manage::NodeStart {
            build: "9.9.9".to_owned(),
            build_id: Some("0123456789abcdef".to_owned()),
            release: None,
        }
    }

    /// PRD B34: a build crosses a relay at about 2 MiB a second, so sending
    /// one takes longer than a host has to bring a node up. The wait for
    /// the node starts again once the build is placed, so a host that then
    /// takes nearly all of it is waited for, and one that then says nothing
    /// is given up on after that wait, counted from the build's end.
    #[tokio::test(start_paused = true)]
    async fn the_wait_for_a_node_does_not_count_the_time_its_build_took_to_send() {
        let sending = READY_WITHIN * 3;
        let starting = READY_WITHIN - Duration::from_secs(1);
        for answers in [true, false] {
            let (mine, theirs) = tokio::io::duplex(64 * 1024);
            let (recv, send) = tokio::io::split(mine);
            let mut management = Management::new(send, recv);
            let host = tokio::spawn(async move {
                let (recv, send) = tokio::io::split(theirs);
                let mut host = Management::new(send, recv);
                assert_eq!(host.answer().await.ok(), Some(start()));
                let missing = Manage::NodeStopped {
                    reason: NodeStop::BuildMissing,
                    retry: false,
                    detail: None,
                };
                assert!(host.ask(&missing).await.is_ok());
                assert_eq!(host.answer().await.ok(), Some(start()));
                if answers {
                    tokio::time::sleep(starting).await; // wait-ok: the paused clock, for the time a host takes to start a node
                    let up = Manage::NodeReady {
                        build: "9.9.9".to_owned(),
                    };
                    assert!(host.ask(&up).await.is_ok());
                }
                // Holds its end open until this side lets the stream go.
                let _ = host.answer().await;
            });
            let began = tokio::time::Instant::now();
            let sent = ready(&mut management, start, || async move {
                tokio::time::sleep(sending).await; // wait-ok: the paused clock, for the time a build takes over a relay
                Ok(())
            })
            .await;
            let took = began.elapsed();
            let expected = sending + if answers { starting } else { READY_WITHIN };
            assert_eq!(sent.ok(), answers.then_some(true), "{answers}");
            assert!(
                took >= expected && took < expected + Duration::from_secs(1),
                "{answers}: {took:?} for {expected:?}"
            );
            drop(management);
            host.await.unwrap();
        }
    }

    /// What the core takes, for a test: it can place no event yet, as while
    /// a node's panes are still being read, and tells of each try.
    struct NotYet(mpsc::UnboundedSender<()>);

    impl Events for NotYet {
        fn apply(&self, _device: &str, _event: &Event, _last: bool) -> Applied {
            let _ = self.0.send(());
            Applied::Later
        }
    }

    fn event(seq: u64) -> Event {
        Event {
            v: hide_peer::protocol::event::BOX_VERSION,
            outbox: "box".to_owned(),
            seq,
            kind: "letter".to_owned(),
            key: format!("intent-{seq}"),
            at_unix_ms: 0,
            body: json!({}),
        }
    }

    /// While an event waits for the core, the held stream looks at its
    /// inbox again on a clock, beside its wait for the host's next message.
    /// A message whose first half arrived before such a look and whose rest
    /// came after it is read whole: its event is recorded and acknowledged,
    /// and the stream ends at a message's edge when the link closes.
    #[tokio::test(start_paused = true)]
    async fn a_message_that_arrives_in_two_parts_across_a_look_at_the_inbox_is_read_whole() {
        let dir = tempfile::tempdir().unwrap();
        let (mine, theirs) = tokio::io::duplex(64 * 1024);
        let (tries, mut tried) = mpsc::unbounded_channel();
        let (close, closed) = watch::channel(false);
        let taking = Taking {
            dir: dir.path().join("inbox"),
            device: "windows-pc".to_owned(),
            events: Arc::new(NotYet(tries)),
        };
        let key = PeerKey::from_hex(&"07".repeat(32)).unwrap();
        let holding = tokio::spawn(async move {
            let (recv, send) = tokio::io::split(mine);
            hold(Management::new(send, recv), key, closed, Some(taking)).await;
        });
        let (recv, send) = tokio::io::split(theirs);
        let mut host = Management::new(send, recv);
        let acked = |seq: u64| {
            Some(Manage::EventAck {
                outbox: "box".to_owned(),
                seq,
            })
        };
        let place = within("this side's place in the box", host.answer()).await;
        assert!(matches!(place, Ok(Manage::Resume { seq: 0, .. })));
        assert!(host.ask(&Manage::Event(event(1))).await.is_ok());
        assert_eq!(
            within("the first event's acknowledgement", host.answer())
                .await
                .ok(),
            acked(1)
        );
        within("the first event's first try", tried.recv())
            .await
            .unwrap();

        let mut frame = Vec::new();
        wire::write_message(&mut frame, &Manage::Event(event(2)))
            .await
            .unwrap();
        let (first, rest) = frame.split_at(frame.len() / 2);
        host.send.write_all(first).await.unwrap();
        host.send.flush().await.unwrap();
        // Two more looks at the inbox: the wait that held the first half of
        // the message was dropped for a look in between.
        for _ in 0..2 {
            within("a look at the inbox", tried.recv()).await.unwrap();
        }
        host.send.write_all(rest).await.unwrap();
        host.send.flush().await.unwrap();
        assert_eq!(
            within("the second event's acknowledgement", host.answer())
                .await
                .ok(),
            acked(2)
        );

        close.send(true).unwrap();
        drop(host);
        within("the held stream's end", holding).await.unwrap();
    }
}

#[cfg(test)]
mod away_tests {
    use super::*;

    /// A machine that is not connected after a long sleep is tried again by
    /// the core: before, it stayed stopped although its connection came
    /// back (the release A fault run, D-34 (2)).
    #[test]
    fn a_network_machine_not_connected_yet_is_a_failure_the_core_tries_again() {
        let away = EstablishError::Stopped("not connected".to_owned());
        match not_yet("mini", away) {
            EstablishError::Connect(error) => {
                assert!(error.diagnostic().retryable);
                assert_eq!(error.diagnostic().reason, "not connected");
                assert_eq!(error.diagnostic().target, "mini");
            }
            other => panic!("{other:?}"),
        }
        let refused = EstablishError::Unsupported("old".to_owned());
        assert!(matches!(
            not_yet("mini", refused),
            EstablishError::Unsupported(_)
        ));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use std::io::Write;

    use hide_kit::layout;
    use hide_platform::ipc::LocalListener;

    use super::*;
    use crate::guests::{Guests, stand_in};
    use crate::peer_host::fixture::{Machine, paired, short_root, within};

    /// The two halves against each other: a guest's dialer and the host's
    /// guests, over two endpoints on loopback. The node is a stand-in whose
    /// link endpoint this test listens on.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_core_has_a_paired_machine_run_its_node_uploads_the_build_it_lacks_and_gets_the_link()
    {
        let host = Machine::at("windows-pc", short_root());
        let guest = Machine::new("macbook");
        let home = tempfile::tempdir().unwrap();
        Guests::new(
            host.dir.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        )
        .serve(&host.host, host.shutdown.clone());
        let _link = paired(&host, &guest).await;
        let host_key = host.host.peers().unwrap().key();
        let guest_key = guest.host.peers().unwrap().key();

        let node_dir = layout::guest_node_side(host.dir.path(), &guest_key);
        std::fs::create_dir_all(&node_dir).unwrap();
        let listener = LocalListener::bind(&node_dir.join(launch::LINK_ENDPOINT)).unwrap();
        let node = tokio::task::spawn_blocking(move || {
            let mut stream = listener.accept().unwrap();
            let mut heard = [0u8; 5];
            stream.read_exact(&mut heard).unwrap();
            stream.write_all(b"world").unwrap();
            (heard, listener)
        });

        let programs = tempfile::tempdir().unwrap();
        stand_in::programs(programs.path());
        let packages =
            HelperPackages::new(Some(programs.path().to_path_buf()), crate::cli::RELEASE);
        let dialer = Dialer::new(
            Handle::current(),
            "9.9.9".to_owned(),
            None,
            guest.dir.path().to_path_buf(),
        );
        dialer.attach(Arc::clone(&guest.host));
        let open = |dialer: Arc<Dialer>, packages: HelperPackages| {
            tokio::task::spawn_blocking(move || {
                dialer.open(
                    &host_key.to_hex(),
                    "windows-pc",
                    &packages,
                    Wake::NotAllowed,
                    &Cancel::default(),
                )
            })
        };
        let opened = within(
            "the node's link",
            open(Arc::clone(&dialer), packages.clone()),
        )
        .await
        .unwrap()
        .unwrap_or_else(|error| panic!("the host runs the node: {error}"));
        assert!(opened.placed, "the host lacked the build and was sent it");
        assert_eq!(opened.name, "windows-pc");

        let Opened {
            mut reading,
            mut writing,
            close,
            ..
        } = opened;
        let carried = tokio::task::spawn_blocking(move || {
            writing.write_all(b"hello").unwrap();
            writing.flush().unwrap();
            let mut back = [0u8; 5];
            reading.read_exact(&mut back).unwrap();
            (back, reading)
        });
        let (back, mut reading) = within("the node's bytes", carried).await.unwrap();
        assert_eq!(&back, b"world");
        let (heard, listener) = within("the node's read", node).await.unwrap();
        assert_eq!(&heard, b"hello");

        // The link's close ends a read that waits on it.
        close();
        let ended = within(
            "the closed link's read",
            tokio::task::spawn_blocking(move || reading.read(&mut [0u8; 1])),
        )
        .await
        .unwrap();
        assert!(matches!(ended, Ok(0) | Err(_)), "{ended:?}");

        // The next link finds the build placed and the node running.
        let accepted = tokio::task::spawn_blocking(move || listener.accept().map(drop));
        let again = within("the second link", open(dialer, packages))
            .await
            .unwrap()
            .unwrap_or_else(|error| panic!("the host still runs the node: {error}"));
        assert!(!again.placed, "a build the host has is not sent again");
        within("the node's second link", accepted)
            .await
            .unwrap()
            .unwrap();
        (again.close)();
    }

    /// What the core takes, for a test: the first event's pane is not
    /// known at its first try, as while a node's link is still being read.
    struct Taken {
        tries: std::sync::atomic::AtomicUsize,
        applied: tokio::sync::mpsc::UnboundedSender<(String, Event)>,
    }

    impl Events for Taken {
        fn apply(&self, device: &str, event: &Event, _last: bool) -> Applied {
            let first_try = self.tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0;
            if first_try {
                return Applied::Later;
            }
            let _ = self.applied.send((device.to_owned(), event.clone()));
            Applied::Done
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_core_names_its_place_in_its_nodes_box_and_applies_each_event_once_in_order() {
        let host = Machine::at("windows-pc", short_root());
        let guest = Machine::new("macbook");
        let home = tempfile::tempdir().unwrap();
        Guests::new(
            host.dir.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        )
        .serve(&host.host, host.shutdown.clone());
        let _link = paired(&host, &guest).await;
        let host_key = host.host.peers().unwrap().key();
        let guest_key = guest.host.peers().unwrap().key();

        // The stand-in node keeps a box; this test is its link endpoint,
        // and keeps the node's end of each link open in `links` until it
        // lets it go, as a node keeps its own while it runs.
        let node_dir = layout::guest_node_side(host.dir.path(), &guest_key);
        std::fs::create_dir_all(&node_dir).unwrap();
        let listener = LocalListener::bind(&node_dir.join(launch::LINK_ENDPOINT)).unwrap();
        let (accepted, links) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(stream) = listener.accept() {
                // A link taken after the test let them go is closed here.
                if accepted.send(stream).is_err() {
                    break;
                }
            }
        });
        let programs = tempfile::tempdir().unwrap();
        stand_in::programs_with_a_box(programs.path());
        let packages =
            HelperPackages::new(Some(programs.path().to_path_buf()), crate::cli::RELEASE);
        let dialer = Dialer::new(
            Handle::current(),
            "9.9.9".to_owned(),
            None,
            guest.dir.path().to_path_buf(),
        );
        dialer.attach(Arc::clone(&guest.host));
        let (applied, mut heard) = tokio::sync::mpsc::unbounded_channel();
        dialer.take_events(Arc::new(Taken {
            tries: Default::default(),
            applied,
        }));
        let open = |dialer: Arc<Dialer>, packages: HelperPackages| {
            tokio::task::spawn_blocking(move || {
                dialer.open(
                    &host_key.to_hex(),
                    "windows-pc",
                    &packages,
                    Wake::NotAllowed,
                    &Cancel::default(),
                )
            })
        };
        let opened = within(
            "the node's link",
            open(Arc::clone(&dialer), packages.clone()),
        )
        .await
        .unwrap()
        .unwrap_or_else(|error| panic!("the host runs the node: {error}"));

        // The core has seen no box: it asks from the start. The node's
        // second event exists only because the first one's acknowledgement
        // reached it, and the first is applied before the second though
        // the core could not place it at once.
        let (device, first) = within("the first event", heard.recv()).await.unwrap();
        assert_eq!(device, "windows-pc");
        assert_eq!(
            (first.seq, &first.body),
            (1, &json!({"from": 0, "acked": 0, "away": 0}))
        );
        let (_, second) = within("the second event", heard.recv()).await.unwrap();
        assert_eq!((second.seq, &second.body["acked"]), (2, &json!(1)));

        // The next connection names the place this one recorded, so the
        // node sends nothing it already gave.
        // The core closes a link and lets its stream go, which frees the
        // host's place for the next one.
        let Opened {
            reading,
            writing,
            close,
            ..
        } = opened;
        close();
        drop((reading, writing));
        let again = within("the second link", open(dialer, packages))
            .await
            .unwrap()
            .unwrap_or_else(|error| panic!("the host still runs the node: {error}"));
        let (_, third) = within("the third event", heard.recv()).await.unwrap();
        assert_eq!((third.seq, &third.body["from"]), (3, &json!(2)));

        // The node's end of the link goes, as when a node ends, and the
        // link ends on this machine with it. The host carries a link on a
        // thread of its own until one of its ends closes, and a test's
        // runtime waits for that thread when it is dropped, without running
        // what would bring it this machine's close: left to that, the test
        // hung whenever its end outran the close (seen in CI, run
        // 44545668117).
        drop(links);
        let Opened {
            mut reading,
            writing,
            close,
            ..
        } = again;
        let ended = within(
            "the second link's end",
            tokio::task::spawn_blocking(move || {
                let mut rest = Vec::new();
                reading.read_to_end(&mut rest).map(|_| rest)
            }),
        )
        .await
        .unwrap();
        assert!(matches!(ended.as_deref(), Ok([]) | Err(_)), "{ended:?}");
        close();
        drop(writing);
    }
}
