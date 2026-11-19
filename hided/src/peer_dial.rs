//! This core's node on a machine it is paired with (PRD device-connect-iroh
//! D-61, D-77): the guest's side of [`crate::guests`].
//!
//! The core opens a paired machine as it opens any device, through its
//! connector, which asks here for the node's link
//! ([`hide_node::ssh::host::peer::PeerDial`]). On the connection the two
//! machines already keep, this asks the host on the management stream to
//! run this build's node, uploads the build when the host lacks it, and
//! opens the node link stream once the host says the node is ready. The
//! management stream stays open for as long as that link does.

use std::collections::HashMap;
use std::io::{self, Read};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use hide_node::ssh::host::HelperPackages;
use hide_node::ssh::host::guest_build::GuestBuild;
use hide_node::ssh::host::peer::{Opened, PeerDial};
use hide_node_link::device::{Cancel, EstablishError};
use hide_peer::protocol::build::FileMessage;
use hide_peer::protocol::hello::GrantState;
use hide_peer::protocol::manage::{Manage, NodeStop, Release};
use hide_peer::protocol::{PeerKey, StreamKind, frame, launch};
use hide_peer::{PeerEvent, limits, wire};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Handle;
use tokio::sync::watch;

use crate::peer_host::{HostPeers, Link, PeerHost, Stream};

/// How long a machine that is not connected is waited for: the connection
/// is kept by the pairing, which dials it again by itself.
const CONNECTED_WITHIN: Duration = Duration::from_secs(10);

/// How long the host has to say which platform it is.
const PLATFORM_WITHIN: Duration = Duration::from_secs(30);

/// How long the host has to bring the node up once it has the build: the
/// node's own start, the folder another node may still hold, and one start
/// again after a crash.
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
    held: Mutex<HashMap<PeerKey, watch::Sender<bool>>>,
}

impl Dialer {
    pub fn new(runtime: Handle, build: String, release: Option<Release>) -> Arc<Self> {
        Arc::new(Self {
            host: OnceLock::new(),
            runtime,
            build,
            release,
            held: Mutex::new(HashMap::new()),
        })
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
        packages: HelperPackages,
    ) -> Result<Opened, EstablishError> {
        let peers = host.open().await.map_err(|error| {
            failed(
                "This machine's connection to other Hides did not start",
                error,
            )
        })?;
        let link = connected(&peers, &key).await?;
        if link.granted() != GrantState::Active {
            return Err(EstablishError::Stopped(
                "That machine has not opened itself to this one, or paused it".to_owned(),
            ));
        }
        let name = peers
            .host(&key)
            .map_or_else(|| key.short(), |known| known.name);
        // A connection carries one management stream: the one an earlier
        // link to this host held ends before this one opens.
        if let Some(earlier) = lock(&self.held).remove(&key) {
            earlier.send_replace(true);
        }
        let mut management = link
            .open(StreamKind::Management)
            .await
            .map_err(|error| failed("The management stream did not open", error))?;

        ask(&mut management, &self.start(None)).await?;
        let (os, arch) = tokio::time::timeout(PLATFORM_WITHIN, async {
            loop {
                if let Manage::NodePreparing { os, arch } = answer(&mut management).await? {
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
        ask(&mut management, &self.start(Some(id.clone()))).await?;
        let mut placed = false;
        tokio::time::timeout(READY_WITHIN, async {
            loop {
                match answer(&mut management).await? {
                    Manage::NodeReady { .. } => break Ok(()),
                    Manage::NodeStopped {
                        reason: NodeStop::BuildMissing,
                        ..
                    } if !placed => {
                        upload(&link, &build).await?;
                        placed = true;
                        ask(&mut management, &self.start(Some(id.clone()))).await?;
                    }
                    // The host starts it again by itself.
                    Manage::NodeStopped { retry: true, .. } => {}
                    Manage::NodeStopped { reason, detail, .. } => {
                        break Err(stopped(reason, detail));
                    }
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| {
            failed(
                "This Hide's node there did not start",
                "not ready within the wait",
            )
        })??;

        let Stream { send, recv, .. } = link
            .open(StreamKind::NodeLink)
            .await
            .map_err(|error| failed("The node link did not open", error))?;
        let (close, closed) = watch::channel(false);
        lock(&self.held).insert(key, close.clone());
        tokio::spawn(hold(management, key, closed.clone()));
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
        packages: &HelperPackages,
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
                opened = self.open_link(host, key, packages.clone()) => opened,
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

/// The connection to `key`, waited for while the pairing brings it up.
async fn connected(peers: &HostPeers, key: &PeerKey) -> Result<Link, EstablishError> {
    let mut events = peers.events();
    if let Some(link) = peers.link(key) {
        return Ok(link);
    }
    let waited = tokio::time::timeout(CONNECTED_WITHIN, async {
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

async fn ask(management: &mut Stream, message: &Manage) -> Result<(), EstablishError> {
    wire::write_message(&mut management.send, message)
        .await
        .map_err(|error| failed("That machine took no management message", error))
}

/// The next management message this build knows.
async fn answer(management: &mut Stream) -> Result<Manage, EstablishError> {
    loop {
        let body = wire::read_message(&mut management.recv, frame::MAX_MESSAGE)
            .await
            .map_err(|error| failed("The management stream failed", error))?
            .ok_or_else(|| failed("The management stream ended", "that machine closed it"))?;
        match Manage::parse(&body) {
            Ok(Some(message)) => return Ok(message),
            // A message of a later build.
            Ok(None) => {}
            Err(error) => return Err(failed("That machine sent no management message", error)),
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
        NodeStop::BuildMissing => EstablishError::Install(format!(
            "That machine does not have this Hide's build and did not take it. {detail}"
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
    let sent = |error: io::Error| {
        EstablishError::Install(format!("This build did not reach that machine: {error}"))
    };
    let mut stream = link.open(StreamKind::File).await.map_err(|error| {
        EstablishError::Install(format!("The file stream did not open: {error}"))
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
        Some(Ok(Some(FileMessage::Refused { reason }))) => Err(EstablishError::Install(format!(
            "That machine refused this Hide's build: {reason}"
        ))),
        _ => Err(EstablishError::Install(
            "That machine did not say whether it placed this Hide's build".to_owned(),
        )),
    }
}

/// Keeps a host's management stream open while its node link is, and
/// records what the host says of the node meanwhile. The link itself ends
/// when the node does; this only says why.
async fn hold(mut management: Stream, key: PeerKey, mut closed: watch::Receiver<bool>) {
    loop {
        let message = tokio::select! {
            message = answer(&mut management) => message,
            _ = closed.wait_for(|closed| *closed) => break,
        };
        match message {
            Ok(Manage::NodeStopped {
                reason,
                retry,
                detail,
            }) => hide_core::diagnostic!(json!({
                "component": "peer_dial",
                "kind": "peer_dial.node_stopped",
                "host": key.short(),
                "reason": format!("{reason:?}"),
                "retry": retry,
                "detail": detail,
            })),
            Ok(_) => {}
            Err(_) => break,
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
        let dialer = Dialer::new(Handle::current(), "9.9.9".to_owned(), None);
        dialer.attach(Arc::clone(&guest.host));
        let open = |dialer: Arc<Dialer>, packages: HelperPackages| {
            tokio::task::spawn_blocking(move || {
                dialer.open(&host_key.to_hex(), &packages, &Cancel::default())
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
}
