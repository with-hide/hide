//! Other machines' work on this machine (PRD device-connect-iroh D-61,
//! D-77): for each guest, a folder of its own, the guest's build placed in
//! it, and one node process of that build which this daemon starts,
//! supervises ([`node`]) and feeds from the guest's connection.
//!
//! Three kinds of stream reach here, each only under a grant the guest
//! holds: the management stream, on which the guest asks for its node and
//! hears where it stands; the node link, whose bytes this daemon carries
//! to the node's `link.sock` and back without reading them; and a file
//! stream that carries a build this machine does not have.
//!
//! A guest's node is its own build's process, so nothing here speaks to it
//! beyond the launch contract, and one guest's failure stays in that
//! guest's folder and process.

pub mod node;
pub mod tap;

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use hide_kit::layout;
use hide_node::ssh::host::{HelperPackages, guest_build};
use hide_node_link::address::guest_pane_id;
use hide_peer::protocol::build::{self, FileMessage};
use hide_peer::protocol::launch::{EndMode, Position};
use hide_peer::protocol::manage::{Manage, NodeStop};
use hide_peer::protocol::{PeerKey, StreamKind, frame, launch};
use hide_peer::{limits, wire};
use hide_platform::fs::{Access, atomic, private};
use hide_platform::ipc::{self, LocalStream};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Handle;
use tokio::sync::watch;

use self::node::{GuestNode, Phase};
use self::tap::GuestTap;
use crate::core::CoreHandle;
use crate::ending::Ending;
use crate::peer_host::{Incoming, PeerHost, Stream};

/// How long the daemon's end waits for each guest's node to be reaped.
const END_WITHIN: Duration = Duration::from_secs(launch::END_NOW_WAITS_SECS + 1);

/// How long a node that said it is ready is given to take its link.
const LINK_WITHIN: Duration = Duration::from_secs(5);

/// How long a guest's management stream is kept from ending for its node
/// link to end here first. Less than a guest waits for the stream's end
/// before its next link (`peer_dial`, `HELD_ENDS_WITHIN`).
const LINK_ENDS_WITHIN: Duration = Duration::from_secs(1);

/// Connects to an endpoint of a guest's node, waiting `within` for a node
/// that is slow to take it. The wait bounds the connect alone: the stream
/// is one this daemon keeps for as long as the node and the guest do, so
/// the deadline a bounded connect leaves on its stream is taken off.
fn connect_to_node(endpoint: &Path, within: Duration) -> io::Result<LocalStream> {
    let stream = LocalStream::connect_until(endpoint, Instant::now() + within)?;
    stream.clear_deadline();
    Ok(stream)
}

/// The bytes carried in one step between a guest and its node.
const CARRY: usize = 64 * 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A map of guests is whole whatever panicked beside it.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn record(kind: &str, guest: &PeerKey, extra: serde_json::Value) {
    let mut record = json!({
        "component": "guests",
        "kind": kind,
        "guest": guest.short(),
    });
    if let (Some(record), Some(extra)) = (record.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            record.insert(key.clone(), value.clone());
        }
    }
    hide_core::diagnostic!(record);
}

/// Why a guest's node was not started, as its guest is told.
type Refusal = (NodeStop, String);

fn stopped((reason, detail): Refusal) -> Manage {
    Manage::NodeStopped {
        reason,
        retry: false,
        detail: Some(detail),
    }
}

fn told(phase: &Phase) -> Manage {
    match phase {
        Phase::Starting => preparing(),
        Phase::Ready(running) => Manage::NodeReady {
            build: running.build.clone(),
        },
        Phase::Stopped {
            reason,
            retry,
            detail,
        } => Manage::NodeStopped {
            reason: *reason,
            retry: *retry,
            detail: detail.clone(),
        },
    }
}

/// The platform a guest's build has to be for: this machine's.
fn preparing() -> Manage {
    Manage::NodePreparing {
        os: std::env::consts::OS.to_owned(),
        arch: std::env::consts::ARCH.to_owned(),
    }
}

/// Where a guest's panes reach this machine's screens and its core.
pub struct Screens {
    pub core: Arc<CoreHandle>,
}

/// What this host remembers of a guest's node between its own starts
/// (`h/node.json`), so the guest's panes are drawn and typed into again
/// after this daemon restarts with the guest away (B16).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct NodeRecord {
    /// The build the node last ran from.
    build_id: String,
    /// The guest's product device id.
    device: String,
}

/// One guest on this machine: its node, and the tap this machine's screens
/// reach its panes by. Made together and dropped together.
#[derive(Clone)]
struct Hosted {
    node: Arc<GuestNode>,
    tap: Arc<GuestTap>,
    /// The guest was taken off this machine: its folder goes once nothing
    /// of it runs here.
    removed: Arc<AtomicBool>,
    /// How many of the guest's node links are carried here now.
    links: watch::Sender<usize>,
}

/// One node link while it is carried, counted in its guest's
/// [`Hosted::links`].
struct Carried(watch::Sender<usize>);

impl Carried {
    fn begin(links: &watch::Sender<usize>) -> Self {
        links.send_modify(|links| *links += 1);
        Self(links.clone())
    }
}

impl Drop for Carried {
    fn drop(&mut self) {
        self.0.send_modify(|links| *links -= 1);
    }
}

impl Hosted {
    /// Whether the guest is at work here: its node runs, or a pane of it
    /// is left. That long its hook entries are on this account
    /// ([`Guests::unhook_if_idle`]).
    fn at_work(&self) -> bool {
        !self.tap.panes().is_empty() || !self.node.stopped_by(Instant::now())
    }
}

/// What this service was told so far.
#[derive(Default)]
struct Told {
    /// The guests this machine hosted before this start are listed again.
    resumed: bool,
    /// The guests that hold a grant here, as last told.
    granted: Option<Vec<PeerKey>>,
}

/// The word [`layout::guest_removed`] holds for a guest whose panes are
/// to end at once; anything else leaves them until the last one ends.
const END_NOW: &str = "now";

/// How long a node started only to end its panes has to be ready.
const END_START_WITHIN: Duration = Duration::from_secs(launch::READY_WITHIN_SECS + 5);

pub struct Guests {
    state_dir: PathBuf,
    /// The account's home, which every guest's node is started with.
    home: PathBuf,
    /// This daemon's own programs: the build a guest of the same build
    /// runs, placed without a byte crossing the connection.
    packages: HelperPackages,
    /// The name of this daemon's own build for this machine, read once.
    own_build: OnceLock<Option<String>>,
    hosted: Mutex<HashMap<PeerKey, Hosted>>,
    told: Mutex<Told>,
    /// Held while a guest is being taken off: the two ways it is said
    /// (the grant list, the operator's "end it") come on two threads.
    leaving: Mutex<()>,
    /// One guest's node is started at a time, so counting the guests at
    /// work and the start that adds one are a single step.
    starting: Mutex<()>,
    /// None in a test of the node's launch alone, where no screen draws a
    /// guest's panes.
    screens: Option<Screens>,
    runtime: Handle,
}

/// The screens' hub for a daemon with none.
struct NoScreens;

impl hide_node::terminal::OutputSink for NoScreens {
    fn output(&self, _pane: &str, _bytes: &[u8], _full: bool) {}
    fn forget(&self, _pane: &str) {}
    fn clipboard(&self, _pane: &str, _text: &[u8]) {}
}

/// Where one guest's node keeps its state and runs its builds from.
struct Place {
    /// The node's `--guest` folder.
    node: PathBuf,
    /// The folder its builds are placed in.
    install: PathBuf,
    /// Whether they are the device helper's folders, which this guest took
    /// over ([`Guests::take_helper`]), and not folders of the guest's own.
    helper: bool,
}

/// What became of a guest's ask for the device helper's folder on this
/// machine (PRD device-connect-iroh D-49, D-75).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HelperFolder {
    /// The guest's node runs on the helper's panes from now on.
    Taken,
    /// No helper ever ran on this account: there is nothing to take.
    Absent,
    /// The guest's work already runs here from a folder of its own, which
    /// it keeps.
    Kept,
    /// Another paired machine took the folder first.
    HeldBy(PeerKey),
}

impl Guests {
    /// The paired machine that took the device helper's folder over, with
    /// the helper root, as the record in that folder says.
    fn helper_taken(&self) -> Option<(PeerKey, PathBuf)> {
        let taken = layout::HelperGuest::read(&layout::device_node_state(&self.home))?;
        let key = PeerKey::from_hex(&taken.key).ok()?;
        taken.root.is_absolute().then_some((key, taken.root))
    }

    /// The paired machine that took the device helper's folder over.
    pub fn helper_holder(&self) -> Option<PeerKey> {
        self.helper_taken().map(|(key, _)| key)
    }

    /// The helper root `spelled` names on this account (`~/` is its home),
    /// when a device helper was installed there: a folder with a build of
    /// Hide's in it. Any other folder is not one a guest may name as its
    /// install root.
    fn helper_root(&self, spelled: &str) -> Option<PathBuf> {
        let root = layout::expand_home(spelled, &self.home);
        let built = std::fs::read_dir(&root).ok()?.flatten().any(|entry| {
            entry.file_name().to_str().is_some_and(build::is_id)
                && entry
                    .path()
                    .join(format!("hided{}", std::env::consts::EXE_SUFFIX))
                    .is_file()
        });
        (root.is_absolute() && built).then_some(root)
    }

    /// Gives `guest` the device helper's folder when no other paired
    /// machine holds it: the first to ask keeps it, and a later one gets a
    /// folder of its own like any guest (D-75). `paired` says whether a
    /// machine still holds a grant here; the record of one that does not,
    /// and has nothing running here, is a pairing that never finished and
    /// is replaced. Blocking.
    pub fn take_helper(
        &self,
        guest: &PeerKey,
        root: &str,
        paired: impl Fn(&PeerKey) -> bool,
    ) -> HelperFolder {
        match self.helper_for(guest, root, paired) {
            // Free: nobody holds it, and `guest` runs nothing here yet.
            HelperFolder::Kept if !self.runs_here(guest) => {}
            found => return found,
        }
        let Some(root) = self.helper_root(root) else {
            return HelperFolder::Absent;
        };
        let taken = layout::HelperGuest {
            key: guest.to_hex(),
            root,
            panes: None,
        };
        match serde_json::to_vec(&taken)
            .map_err(io::Error::other)
            .and_then(|bytes| {
                atomic::write_file(
                    &layout::device_node_guest(&self.home),
                    &bytes,
                    Access::Private,
                )
            }) {
            Ok(_) => {
                record("guests.helper_taken", guest, json!({}));
                HelperFolder::Taken
            }
            Err(error) => {
                record(
                    "guests.helper_not_taken",
                    guest,
                    json!({"reason": error.to_string()}),
                );
                HelperFolder::Kept
            }
        }
    }

    /// Whether a node of `guest` ever ran on this machine, from a folder
    /// of its own.
    fn runs_here(&self, guest: &PeerKey) -> bool {
        lock(&self.hosted).contains_key(guest) || self.remembered(guest).is_some()
    }

    /// What `guest` would find if it asked for the device helper's folder
    /// now, and nothing changes: [`HelperFolder::Kept`] for a folder it
    /// does not hold, which it may take unless its node already ran here
    /// from a folder of its own. Blocking.
    pub fn helper_for(
        &self,
        guest: &PeerKey,
        root: &str,
        paired: impl Fn(&PeerKey) -> bool,
    ) -> HelperFolder {
        let folder = layout::device_node_state(&self.home);
        if !layout::pane_records(&folder).is_dir() || self.helper_root(root).is_none() {
            return HelperFolder::Absent;
        }
        match self.helper_holder() {
            Some(holder) if holder == *guest => HelperFolder::Taken,
            Some(holder)
                if paired(&holder)
                    || lock(&self.hosted).get(&holder).is_some_and(Hosted::at_work) =>
            {
                HelperFolder::HeldBy(holder)
            }
            _ => HelperFolder::Kept,
        }
    }

    /// The folders of `guest`'s node: the device helper's when the guest
    /// took them over, and its own below its guest folder otherwise. The
    /// one place that chooses, so the node, its builds and their removal
    /// agree.
    fn place(&self, guest: &PeerKey) -> Place {
        if let Some((holder, root)) = self.helper_taken()
            && holder == *guest
        {
            return Place {
                node: layout::device_node_state(&self.home),
                install: root,
                helper: true,
            };
        }
        Place {
            node: layout::guest_node_side(&self.state_dir, guest),
            install: layout::guest_install_root(&self.state_dir, guest),
            helper: false,
        }
    }

    /// Called on the daemon's runtime, whose tasks follow each guest's
    /// node.
    pub fn new(
        state_dir: PathBuf,
        home: PathBuf,
        packages: HelperPackages,
        screens: Option<Screens>,
    ) -> Arc<Self> {
        Arc::new(Self {
            state_dir,
            home,
            packages,
            own_build: OnceLock::new(),
            hosted: Mutex::new(HashMap::new()),
            told: Mutex::default(),
            leaving: Mutex::default(),
            starting: Mutex::default(),
            screens,
            runtime: Handle::current(),
        })
    }

    /// Every guest's panes on this machine, as each guest's node last
    /// said; a guest with none is not in the list.
    pub fn panes(&self) -> Vec<hide_core::guest_panes::GuestPanes> {
        let mut guests: Vec<_> = lock(&self.hosted)
            .iter()
            .map(|(key, hosted)| {
                let device = hosted.tap.device();
                hide_core::guest_panes::GuestPanes {
                    device_id: device.to_owned(),
                    key: key.to_hex(),
                    panes: hosted
                        .tap
                        .panes()
                        .into_iter()
                        .map(|pane| hide_core::guest_panes::GuestPane {
                            id: guest_pane_id(device, &pane.pane),
                            pane: pane.pane,
                            since_unix_ms: pane.since_unix_ms,
                        })
                        .collect(),
                }
            })
            .filter(|guest| !guest.panes.is_empty())
            .collect();
        guests.sort_by(|left, right| left.device_id.cmp(&right.device_id));
        guests
    }

    /// Tells the core the guests' panes, whole; the core keeps the list
    /// only when it differs. Called from a tap's own thread or a blocking
    /// one, never under a lock of this module.
    fn tell_core(&self) {
        let Some(screens) = &self.screens else { return };
        let event = json!({
            "schema_version": hide_core::SCHEMA_VERSION,
            "kind": "guest_panes",
            "payload": { "guests": self.panes() },
        });
        if let Err(error) = screens.core.dispatch(event.to_string().into_bytes()) {
            hide_core::diagnostic!(json!({
                "component": "guests",
                "kind": "guests.core_not_told",
                "reason": error,
            }));
        }
    }

    /// The guest's tap: it lists the panes the guest's `h/panes.json`
    /// holds, takes this machine's screens to them from now on, and is
    /// connected whenever `node` is ready.
    fn tap_of(self: &Arc<Self>, guest: &PeerKey, device: &str, node: &GuestNode) -> Arc<GuestTap> {
        let hub: Arc<dyn hide_node::terminal::OutputSink> = match &self.screens {
            Some(screens) => Arc::clone(&screens.core.outputs) as _,
            None => Arc::new(NoScreens),
        };
        let guests = Arc::downgrade(self);
        let listed = *guest;
        let tap = GuestTap::open(
            device,
            &guest.short(),
            layout::guest_panes(&self.state_dir, guest),
            hub,
            Box::new(move || {
                if let Some(guests) = guests.upgrade() {
                    guests.tell_core();
                    // The last pane of a guest whose node is gone ended,
                    // and, of one that was taken off, the guest with it.
                    guests.unhook_if_idle(&listed);
                    guests.forget_if_done(&listed);
                }
            }),
        );
        if let Some(screens) = &self.screens {
            screens
                .core
                .terminals
                .install_guest(device, Arc::clone(&tap) as _);
        }
        let mut phase = node.phase();
        let record = layout::guest_node_record(&self.state_dir, guest);
        let builds = self.place(guest).install;
        let following = Arc::clone(&tap);
        let guests = Arc::downgrade(self);
        let followed = *guest;
        // Ends with the node: its phase has no more to say once it is
        // dropped.
        self.runtime.spawn(async move {
            loop {
                let now = phase.borrow_and_update().clone();
                match now {
                    Phase::Ready(running) => {
                        following.attach(&running.tap);
                        // What the next start of this daemon runs the
                        // guest's node from.
                        let remembered = NodeRecord {
                            build_id: running.build_id,
                            device: following.device().to_owned(),
                        };
                        let record = record.clone();
                        let builds = builds.clone();
                        tokio::task::spawn_blocking(move || {
                            // The build that runs and the one before it
                            // stay; older ones go (D-77 (2)).
                            let before = std::fs::read(&record)
                                .ok()
                                .and_then(|bytes| serde_json::from_slice::<NodeRecord>(&bytes).ok())
                                .map(|earlier| earlier.build_id)
                                .unwrap_or_default();
                            let kept: [&str; launch::BUILDS_KEPT] = [&remembered.build_id, &before];
                            guest_build::prune(&builds, &kept);
                            let written = serde_json::to_vec(&remembered)
                                .map_err(io::Error::other)
                                .and_then(|bytes| {
                                    atomic::write_file(&record, &bytes, Access::Private)
                                });
                            if let Err(error) = written {
                                hide_core::diagnostic!(json!({
                                    "component": "guests",
                                    "kind": "guests.node_not_remembered",
                                    "reason": error.to_string(),
                                }));
                            }
                        });
                    }
                    Phase::Starting => following.detach(),
                    Phase::Stopped { .. } => {
                        following.detach();
                        // The node ended: with no pane left its hook
                        // entries go, and a guest that was taken off with it.
                        if let Some(guests) = guests.upgrade() {
                            tokio::task::spawn_blocking(move || {
                                guests.unhook_if_idle(&followed);
                                guests.forget_if_done(&followed);
                            });
                        }
                    }
                }
                if phase.changed().await.is_err() {
                    break;
                }
            }
        });
        tap
    }

    /// The guests this machine hosted before this start: each one's panes
    /// are listed again from its `h/panes.json`, and a guest that still has
    /// panes gets its node back from the build it last ran, whether or not
    /// the guest is connected (B16). A guest that was taken off this
    /// machine is ended as it was being ended. Blocking: it reads each
    /// guest's folder.
    pub fn resume(self: &Arc<Self>) {
        for (guest, _) in self.folders() {
            let Some(remembered) = self.remembered(&guest) else {
                continue;
            };
            let Ok(node) = self.node_for(&guest, &remembered.device) else {
                continue;
            };
            let has_panes = self
                .known(&guest)
                .is_some_and(|hosted| !hosted.tap.panes().is_empty());
            let removed = std::fs::read_to_string(layout::guest_removed(&self.state_dir, &guest));
            match removed.as_deref().map(str::trim) {
                Ok(END_NOW) => self.leave(&guest, EndMode::Now),
                Ok(_) => self.leave(&guest, EndMode::AfterLastPane),
                Err(_) => {}
            }
            if has_panes && removed.as_deref().map(str::trim).ok() != Some(END_NOW) {
                self.start_remembered(&guest, &node);
            }
            // What a node that ended without this daemon left on the
            // account.
            self.unhook_if_idle(&guest);
            record("guests.resumed", &guest, json!({"panes": has_panes}));
        }
        self.tell_core();
        let granted = {
            let mut told = lock(&self.told);
            told.resumed = true;
            told.granted.clone()
        };
        if let Some(granted) = granted {
            self.sweep(&granted);
        }
    }

    /// Every guest's folder on this machine with the key it holds, at most
    /// twice as many as this machine can have guests.
    fn folders(&self) -> Vec<(PeerKey, PathBuf)> {
        let Ok(folders) = std::fs::read_dir(layout::guests(&self.state_dir)) else {
            return Vec::new();
        };
        folders
            .flatten()
            .take(2 * limits::MAX_GRANTS)
            .filter_map(|folder| {
                let guest = std::fs::read_to_string(folder.path().join(launch::KEY_FILE))
                    .ok()
                    .and_then(|key| PeerKey::from_hex(key.trim()).ok())
                    .filter(|key| folder.file_name().to_str() == Some(key.short().as_str()))?;
                Some((guest, folder.path()))
            })
            .collect()
    }

    fn known(&self, guest: &PeerKey) -> Option<Hosted> {
        lock(&self.hosted).get(guest).cloned()
    }

    fn remembered(&self, guest: &PeerKey) -> Option<NodeRecord> {
        std::fs::read(layout::guest_node_record(&self.state_dir, guest))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    }

    /// Makes room under the guest's install root `root` for the build `id`
    /// before it is placed: every build goes but `id` itself, whose folder
    /// an earlier try may have left to finish, and the build the guest's
    /// node last ran from, [`launch::BUILDS_KEPT`] in all. A build that
    /// never brings a node up so takes the place of the one before it, and
    /// the folder cannot grow by builds that are only ever sent. Blocking.
    fn make_room(&self, guest: &PeerKey, root: &Path, id: &str) {
        let last = self
            .remembered(guest)
            .map(|remembered| remembered.build_id)
            .unwrap_or_default();
        let kept: [&str; launch::BUILDS_KEPT] = [id, &last];
        let removed = guest_build::prune(root, &kept);
        if !removed.is_empty() {
            record("guests.builds_pruned", guest, json!({"removed": removed}));
        }
    }

    /// Starts the guest's node from the build it last ran, when that build
    /// is still placed. Blocking.
    fn start_remembered(&self, guest: &PeerKey, node: &Arc<GuestNode>) -> bool {
        let root = self.place(guest).install;
        let program = self.remembered(guest).and_then(|remembered| {
            guest_build::placed(&root, &remembered.build_id, std::env::consts::OS)
        });
        match program {
            Some(program) => {
                node.start(program);
                true
            }
            None => false,
        }
    }

    /// The guests that hold a grant on this machine now, told at the start
    /// and on every change of the paired machines. A guest that has a
    /// folder here and no grant was taken off this machine (B26, D-77
    /// (6)): its node takes no new link and ends with its last pane, and
    /// what is left of it goes once nothing of it runs here. A guest in
    /// the list that was taken off before is a guest again.
    pub fn granted(self: &Arc<Self>, granted: Vec<PeerKey>) {
        let resumed = {
            let mut told = lock(&self.told);
            told.granted = Some(granted.clone());
            told.resumed
        };
        // Before the guests of the last start are listed again a folder
        // says nothing of what runs from it; `resume` sweeps when it is
        // done.
        if resumed {
            let guests = Arc::clone(self);
            self.runtime.spawn_blocking(move || guests.sweep(&granted));
        }
    }

    /// The operator took `guest` off this machine and chose to end its
    /// work here at once (B26). It needs no connection to the guest: the
    /// guest's node holds its panes' records.
    pub fn end_work(self: &Arc<Self>, guest: PeerKey) {
        let guests = Arc::clone(self);
        self.runtime
            .spawn_blocking(move || guests.leave(&guest, EndMode::Now));
    }

    /// Brings every guest's place here in line with who holds a grant.
    /// Blocking.
    fn sweep(self: &Arc<Self>, granted: &[PeerKey]) {
        let known: Vec<(PeerKey, bool)> = lock(&self.hosted)
            .iter()
            .map(|(guest, hosted)| (*guest, hosted.removed.load(Ordering::SeqCst)))
            .collect();
        for (guest, removed) in &known {
            match (granted.contains(guest), removed) {
                (false, false) => self.leave(guest, EndMode::AfterLastPane),
                (false, true) => self.forget_if_done(guest),
                (true, true) => self.back(guest),
                (true, false) => {}
            }
        }
        // A folder nothing was started from since this daemon's start: a
        // build placed for a guest that never ran, or what a guest left
        // that was taken off while this daemon was down.
        for (guest, _) in self.folders() {
            if !granted.contains(&guest) {
                let hosted = lock(&self.hosted);
                if !hosted.contains_key(&guest) {
                    self.unhook(&guest);
                    self.remove_folder(&guest);
                }
            }
        }
    }

    /// Marks `guest` as taken off this machine and tells its node how to
    /// end. With `EndMode::Now` a node that is not running is started to
    /// end the panes its holders still run. Blocking.
    fn leave(self: &Arc<Self>, guest: &PeerKey, mode: EndMode) {
        let Some(hosted) = self.known(guest) else {
            return;
        };
        let _one = lock(&self.leaving);
        // The grant list says "taken off" of a guest the operator already
        // ended at once: the stronger word stands.
        if hosted.removed.swap(true, Ordering::SeqCst) && mode == EndMode::AfterLastPane {
            return;
        }
        // What the next start of this daemon goes on with.
        let word = match mode {
            EndMode::Now => END_NOW,
            EndMode::AfterLastPane => "after_last_pane",
        };
        if let Err(error) = atomic::write_file(
            &layout::guest_removed(&self.state_dir, guest),
            word.as_bytes(),
            Access::Private,
        ) {
            record(
                "guests.removal_not_saved",
                guest,
                json!({"reason": error.to_string()}),
            );
        }
        if mode == EndMode::Now && hosted.node.running().is_none() && !hosted.tap.panes().is_empty()
        {
            if self.start_remembered(guest, &hosted.node) {
                let mut phase = hosted.node.phase();
                let ready = self.runtime.block_on(async {
                    tokio::time::timeout(
                        END_START_WITHIN,
                        phase.wait_for(|phase| !matches!(phase, Phase::Starting)),
                    )
                    .await
                    .is_ok_and(|phase| phase.is_ok_and(|phase| matches!(*phase, Phase::Ready(_))))
                });
                if !ready {
                    record("guests.end_not_started", guest, json!({}));
                }
            } else {
                // No build is left to run the node that would end them:
                // the panes stay, and the log says why.
                record(
                    "guests.end_without_build",
                    guest,
                    json!({"panes": hosted.tap.panes().len()}),
                );
            }
        }
        record(
            "guests.removed",
            guest,
            json!({"mode": word, "panes": hosted.tap.panes().len()}),
        );
        hosted.node.end(mode);
        self.forget_if_done(guest);
    }

    /// A guest that was taken off this machine is a guest again: its node
    /// is started again, since the one that runs takes no new link.
    /// Blocking.
    fn back(&self, guest: &PeerKey) {
        let Some(hosted) = self.known(guest) else {
            return;
        };
        if !hosted.removed.swap(false, Ordering::SeqCst) {
            return;
        }
        if let Err(error) = std::fs::remove_file(layout::guest_removed(&self.state_dir, guest))
            && error.kind() != io::ErrorKind::NotFound
        {
            record(
                "guests.removal_not_cleared",
                guest,
                json!({"reason": error.to_string()}),
            );
        }
        hosted.node.restart();
        record("guests.back", guest, json!({}));
    }

    /// Drops what is left of a guest that was taken off this machine once
    /// nothing of it runs here: its place among the guests, the screens'
    /// way to its panes, and its folder. Blocking.
    fn forget_if_done(&self, guest: &PeerKey) {
        let dropped = {
            let mut hosted = lock(&self.hosted);
            let done = hosted.get(guest).is_some_and(|known| {
                known.removed.load(Ordering::SeqCst)
                    && known.tap.panes().is_empty()
                    && known.node.stopped_by(Instant::now())
            });
            if !done {
                return;
            }
            let dropped = hosted.remove(guest);
            // Under the lock: a guest that is paired again makes its
            // folder, and its node its hook entries, only after these are
            // gone.
            self.unhook(guest);
            self.remove_folder(guest);
            dropped
        };
        if let Some(dropped) = dropped {
            dropped.tap.detach();
            if let Some(screens) = &self.screens {
                screens.core.terminals.remove_guest(dropped.tap.device());
            }
        }
        record("guests.forgotten", guest, json!({}));
        self.tell_core();
    }

    /// Takes the hook entries of `guest`'s installation off this account
    /// once nothing of the guest runs here: its node stopped and it has no
    /// pane (D-77 (4)). A guest that comes back has them written again by
    /// its node, which its core asks as it asks any device's kit. Blocking.
    fn unhook_if_idle(&self, guest: &PeerKey) {
        let idle = self.known(guest).is_none_or(|known| {
            known.tap.panes().is_empty() && known.node.stopped_by(Instant::now())
        });
        if idle {
            self.unhook(guest);
        }
    }

    /// Takes what `guest`'s installation wrote into the account's agent
    /// settings: the entries that carry Hide's marker and that
    /// installation's name, its agent pieces and its record (D-77 (5)). The
    /// guest's node wrote them from the guest's own build; this build knows
    /// them by the name alone, which is the install root's
    /// (`hide_kit::remove_guest`), and leaves the account's `hide` command,
    /// its skills and every other installation's entries. Blocking.
    fn unhook(&self, guest: &PeerKey) {
        let root = self.place(guest).install;
        let report = hide_kit::remove_guest(&root, &self.home);
        let outcomes = report
            .components
            .iter()
            .map(|(id, outcome)| (id.code().to_owned(), outcome))
            .chain(
                report
                    .agents
                    .iter()
                    .map(|(piece, outcome)| (piece.clone(), outcome)),
            );
        let mut removed = Vec::new();
        let mut failed = Vec::new();
        for (piece, outcome) in outcomes {
            match outcome {
                hide_kit::RemoveOutcome::Removed => removed.push(piece),
                hide_kit::RemoveOutcome::Failed { reason } => {
                    failed.push(json!({"piece": piece, "reason": reason}));
                }
                hide_kit::RemoveOutcome::Absent | hide_kit::RemoveOutcome::Kept { .. } => {}
            }
        }
        if !removed.is_empty() || !failed.is_empty() {
            record(
                "guests.hooks_removed",
                guest,
                json!({"removed": removed, "failed": failed}),
            );
        }
    }

    fn remove_folder(&self, guest: &PeerKey) {
        let place = self.place(guest);
        // The node of a guest that took the device helper's folder ran
        // there, so that folder and the helper root go with the guest as a
        // folder of its own would. A guest whose pairing never finished ran
        // nothing there: only its record goes, and the helper's panes stay
        // for the machine that still dials them.
        let mut gone = Vec::new();
        if place.helper {
            if self.remembered(guest).is_some() {
                gone.push(place.node);
                gone.push(place.install);
            } else if let Err(error) = std::fs::remove_file(layout::device_node_guest(&self.home))
                && error.kind() != io::ErrorKind::NotFound
            {
                record(
                    "guests.folder_not_removed",
                    guest,
                    json!({"reason": error.to_string()}),
                );
            }
        }
        gone.push(layout::guest_dir(&self.state_dir, guest));
        for folder in gone {
            if let Err(error) = std::fs::remove_dir_all(&folder)
                && error.kind() != io::ErrorKind::NotFound
            {
                record(
                    "guests.folder_not_removed",
                    guest,
                    json!({"reason": error.to_string()}),
                );
            }
        }
    }

    /// Takes the three kinds of stream a guest opens for its node, and ends
    /// every node with the daemon. Called once, before the endpoint opens.
    pub fn serve(self: &Arc<Self>, peers: &PeerHost, shutdown: Ending) {
        let guests = Arc::clone(self);
        peers.handle(StreamKind::Management, move |incoming| {
            Arc::clone(&guests).manage(incoming)
        });
        let guests = Arc::clone(self);
        peers.handle(StreamKind::NodeLink, move |incoming| {
            Arc::clone(&guests).link(incoming)
        });
        let guests = Arc::clone(self);
        peers.handle(StreamKind::File, move |incoming| {
            Arc::clone(&guests).file(incoming)
        });
        let guests = Arc::clone(self);
        tokio::task::spawn_blocking(move || guests.resume());
        let guests = Arc::clone(self);
        tokio::spawn(async move {
            shutdown.ended().await;
            let _ = tokio::task::spawn_blocking(move || guests.stop()).await;
        });
    }

    /// The node of `guest`, when this daemon was asked for one.
    pub fn node(&self, guest: &PeerKey) -> Option<Arc<GuestNode>> {
        lock(&self.hosted)
            .get(guest)
            .map(|hosted| Arc::clone(&hosted.node))
    }

    /// Ends every guest's node and waits, within a bound, until each is
    /// reaped. Their pane holders stay.
    pub fn stop(&self) {
        let hosted: Vec<_> = lock(&self.hosted).values().cloned().collect();
        for guest in &hosted {
            guest.node.stop();
        }
        let deadline = Instant::now() + END_WITHIN;
        for guest in &hosted {
            guest.node.stopped_by(deadline);
            guest.tap.detach();
        }
    }

    /// The guest's folder, made private with the guest's whole key in it.
    /// A folder that holds another key is another guest's whose key begins
    /// the same: it is refused, never shared.
    fn folder(&self, guest: &PeerKey) -> Result<(), Refusal> {
        let refused = |what: &str, error: &dyn std::fmt::Display| {
            (NodeStop::FolderRefused, format!("{what}: {error}"))
        };
        ipc::check_path(&launch::longest_endpoint(&self.state_dir)).map_err(|error| {
            refused(
                "this machine's state folder is too long a path for a guest's endpoints",
                &error,
            )
        })?;
        // The guest's folder and, in it, what this daemon alone keeps of the
        // guest: the build its node runs from and its pane list are written
        // there from the node's first start.
        private::create_dir_all(&layout::guest_host_side(&self.state_dir, guest))
            .map_err(|error| refused("the guest's folder could not be made", &error))?;
        let key = layout::guest_key(&self.state_dir, guest);
        match std::fs::read_to_string(&key) {
            Ok(held) if held.trim() == guest.to_hex() => Ok(()),
            Ok(_) => Err((
                NodeStop::FolderRefused,
                "another guest whose key begins the same holds this guest's folder".to_owned(),
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                atomic::write_file(&key, guest.to_hex().as_bytes(), Access::Private)
                    .map(drop)
                    .map_err(|error| refused("the guest's key could not be saved", &error))
            }
            Err(error) => Err(refused("the guest's key could not be read", &error)),
        }
    }

    /// The name of this daemon's own build for this machine; `None` when
    /// it lacks a part a guest's node needs.
    fn own_build(&self) -> Option<&str> {
        self.own_build
            .get_or_init(|| {
                self.packages
                    .guest_build(std::env::consts::OS, std::env::consts::ARCH)
                    .ok()
                    .map(|own| own.id().to_owned())
            })
            .as_deref()
    }

    /// The node program of the guest's build `id`: already placed, or
    /// placed now from this daemon's own programs when they are that
    /// build. Blocking: it reads and writes whole programs.
    fn program(&self, guest: &PeerKey, id: &str) -> Result<PathBuf, Refusal> {
        if !build::is_id(id) {
            return Err((NodeStop::Other, format!("{id:?} names no build")));
        }
        self.folder(guest)?;
        let root = self.place(guest).install;
        if let Some(program) = guest_build::placed(&root, id, std::env::consts::OS) {
            return Ok(program);
        }
        if self.own_build() != Some(id) {
            return Err((
                NodeStop::BuildMissing,
                format!("build {id} is not on this machine"),
            ));
        }
        self.make_room(guest, &root, id);
        self.packages
            .guest_build(std::env::consts::OS, std::env::consts::ARCH)
            .map_err(|error| error.to_string())
            .and_then(|own| own.place(&root))
            .map_err(|detail| (NodeStop::FolderRefused, detail))
    }

    /// The guest's node and its tap, made on its first ask. `device` is
    /// the id the guest's panes go by on this machine: one that names no
    /// guest, or that another guest's panes already go by, is refused. The
    /// map holds one entry per guest that ever asked; at the cap the
    /// guests whose node is stopped and who have no pane here are dropped,
    /// and a guest past it is refused.
    fn node_for(
        self: &Arc<Self>,
        guest: &PeerKey,
        device: &str,
    ) -> Result<Arc<GuestNode>, Refusal> {
        let refused = |detail: String| Err((NodeStop::Other, detail));
        if !hide_core::guest_panes::device_names_a_guest(device) {
            return refused(format!("{device:?} is no device id this machine can show"));
        }
        let mut hosted = lock(&self.hosted);
        if let Some(known) = hosted.get(guest) {
            if known.tap.device() != device {
                return refused(format!(
                    "this machine knows this guest as {:?}, not {device:?}; pair the two again",
                    known.tap.device()
                ));
            }
            return Ok(Arc::clone(&known.node));
        }
        if hosted.values().any(|other| other.tap.device() == device) {
            return refused(format!(
                "another guest of this machine already goes by {device:?}"
            ));
        }
        if hosted.len() >= limits::MAX_GRANTS {
            let idle: Vec<PeerKey> = hosted
                .iter()
                .filter(|(_, known)| {
                    known.tap.panes().is_empty() && known.node.stopped_by(Instant::now())
                })
                .map(|(key, _)| *key)
                .collect();
            for key in idle {
                if let (Some(dropped), Some(screens)) = (hosted.remove(&key), &self.screens) {
                    screens.core.terminals.remove_guest(dropped.tap.device());
                }
            }
        }
        if hosted.len() >= limits::MAX_GRANTS {
            return refused(format!(
                "this machine already runs {} guests' nodes",
                limits::MAX_GRANTS
            ));
        }
        let place = self.place(guest);
        let node = GuestNode::new(
            *guest,
            place.node,
            self.home.clone(),
            place.helper.then(|| self.state_dir.clone()),
        );
        let tap = self.tap_of(guest, device, &node);
        hosted.insert(
            *guest,
            Hosted {
                node: Arc::clone(&node),
                tap,
                removed: Arc::default(),
                links: watch::Sender::new(0),
            },
        );
        Ok(node)
    }

    /// Starts the guest's node from its build `id`, placing the build when
    /// this machine has it.
    async fn start(
        self: &Arc<Self>,
        guest: PeerKey,
        device: &str,
        id: String,
    ) -> Result<Arc<GuestNode>, Refusal> {
        let guests = Arc::clone(self);
        let program = tokio::task::spawn_blocking(move || guests.program(&guest, &id))
            .await
            .map_err(|error| (NodeStop::Other, error.to_string()))??;
        let node = self.node_for(&guest, device)?;
        // Only a guest that holds a grant asks: one that was taken off
        // this machine before is back.
        let guests = Arc::clone(self);
        let _ = tokio::task::spawn_blocking(move || guests.back(&guest)).await;
        self.within_the_cap(&guest, || node.start(program))?;
        Ok(node)
    }

    /// Runs `start`, which starts the node of `guest`, unless as many other
    /// guests are at work here as this machine carries at once
    /// ([`launch::MAX_GUEST_NODES`], D-77 (7)). Every start a guest asks
    /// for goes through here, its first and one it asks for again. A guest
    /// already at work is never refused its own node: a restart or a newer
    /// build of it adds no guest.
    fn within_the_cap<T>(&self, guest: &PeerKey, start: impl FnOnce() -> T) -> Result<T, Refusal> {
        let _one = lock(&self.starting);
        let others = lock(&self.hosted)
            .iter()
            .filter(|(key, known)| *key != guest && known.at_work())
            .count();
        if others >= launch::MAX_GUEST_NODES {
            return Err((
                NodeStop::HostFull,
                format!(
                    "{} other machines' work runs here, as many as this machine carries at once",
                    launch::MAX_GUEST_NODES
                ),
            ));
        }
        Ok(start())
    }

    /// One guest's management stream: its asks for its node, and where the
    /// node stands, told on every change from the first ask on.
    async fn manage(self: Arc<Self>, incoming: Incoming) {
        let guest = incoming.link.key();
        let device = incoming.link.their_hello().device.clone();
        let Stream { mut send, recv, .. } = incoming.stream;
        // The read is one branch of a wait that loops: what it has of a
        // message stays in the reader when another branch wins.
        let mut messages = wire::MessageReader::new(recv, frame::MAX_MESSAGE);
        let mut phase: Option<watch::Receiver<Phase>> = None;
        let mut events: Option<Events> = None;
        loop {
            let answer = tokio::select! {
                message = messages.next() => {
                    let Ok(Some(body)) = message else { break };
                    let asked = match Manage::parse(&body) {
                        Ok(Some(asked)) => asked,
                        // A message of a later build.
                        Ok(None) => continue,
                        Err(_) => break,
                    };
                    match self.asked(guest, &device, asked).await {
                        Asked::Answer(answer) => serde_json::to_value(answer).ok(),
                        Asked::Events(asked) => {
                            // A stream asks for one node's box at a time.
                            if let Some(earlier) = events.replace(asked) {
                                earlier.node.away(earlier.listener);
                            }
                            None
                        }
                        Asked::Watch(node) => {
                            // The node's standing now is the first answer.
                            let mut watched = node.phase();
                            watched.mark_changed();
                            phase = Some(watched);
                            None
                        }
                        Asked::Nothing => None,
                    }
                }
                changed = async {
                    match phase.as_mut() {
                        Some(phase) => phase.changed().await.map(|()| told(&phase.borrow_and_update())),
                        None => std::future::pending().await,
                    }
                } => match changed {
                    Ok(answer) => serde_json::to_value(answer).ok(),
                    Err(_) => break,
                },
                event = async {
                    match events.as_mut() {
                        Some(events) => events.events.recv().await,
                        None => std::future::pending().await,
                    }
                } => match event {
                    Some(event) => Some(event),
                    // Another stream asked for the box after this one.
                    None => {
                        events = None;
                        None
                    }
                },
            };
            // A guest that takes nothing ends its stream: the node's
            // events wait in its box, not here.
            if let Some(answer) = answer
                && !matches!(
                    tokio::time::timeout(
                        limits::WRITE_WITHIN,
                        wire::write_message(&mut send, &answer)
                    )
                    .await,
                    Ok(Ok(()))
                )
            {
                break;
            }
        }
        if let Some(events) = events {
            events.node.away(events.listener);
        }
        // The guest opens its next link once it sees this stream end, and
        // a connection carries one node link as it carries one management
        // stream. The node link this guest had ends here first, so its
        // place is free by then: the guest let both go together, and a
        // link that outlives the wait is refused its successor as before.
        if let Some(hosted) = self.known(&guest) {
            let mut links = hosted.links.subscribe();
            let _ =
                tokio::time::timeout(LINK_ENDS_WITHIN, links.wait_for(|links| *links == 0)).await;
        }
        // The reading half goes first, so this machine's place for a
        // management stream is free when the guest sees the end.
        drop(messages);
        drop(send);
    }

    async fn asked(self: &Arc<Self>, guest: PeerKey, device: &str, asked: Manage) -> Asked {
        match asked {
            Manage::NodeStart { build_id: None, .. } => Asked::Answer(preparing()),
            Manage::NodeStart {
                build,
                build_id: Some(id),
                ..
            } => match self.start(guest, device, id.clone()).await {
                Ok(node) => {
                    record(
                        "guests.node_asked",
                        &guest,
                        json!({"build": build, "build_id": id}),
                    );
                    Asked::Watch(node)
                }
                Err(refusal) => {
                    record(
                        "guests.node_refused",
                        &guest,
                        json!({"build": build, "build_id": id, "reason": refusal.1}),
                    );
                    Asked::Answer(stopped(refusal))
                }
            },
            Manage::NodeRetry => {
                // This daemon was never asked for a build: the guest names
                // one, as it does on its first ask.
                let name_one = || {
                    Asked::Answer(Manage::NodeStopped {
                        reason: NodeStop::Ended,
                        retry: false,
                        detail: None,
                    })
                };
                let Some(node) = self.node(&guest) else {
                    return name_one();
                };
                // A node left stopped is no guest at work, so its start
                // again is one more guest here like any other.
                match self.within_the_cap(&guest, || node.retry()) {
                    Ok(true) => Asked::Watch(node),
                    Ok(false) => name_one(),
                    Err(refusal) => {
                        record(
                            "guests.node_refused",
                            &guest,
                            json!({"retry": true, "reason": refusal.1}),
                        );
                        Asked::Answer(stopped(refusal))
                    }
                }
            }
            Manage::NodeEnd { mode } => {
                if let Some(node) = self.node(&guest) {
                    node.end(mode);
                }
                Asked::Nothing
            }
            // The guest's core names its place in its node's box: the node
            // sends what follows on this stream (D-66).
            Manage::Resume { outbox, seq } => self
                .node(&guest)
                .and_then(|node| {
                    let (listener, events) = node.resume(Position { outbox, seq })?;
                    Some(Asked::Events(Events {
                        node,
                        listener,
                        events,
                    }))
                })
                .unwrap_or(Asked::Nothing),
            Manage::EventAck { outbox, seq } => {
                if let Some(node) = self.node(&guest) {
                    node.ack(Position { outbox, seq });
                }
                Asked::Nothing
            }
            // What only a host writes.
            Manage::Event(_)
            | Manage::NodeReady { .. }
            | Manage::NodeStopped { .. }
            | Manage::NodePreparing { .. } => Asked::Nothing,
        }
    }

    /// One guest's node link: its bytes go to the guest's node and the
    /// node's come back, until either side ends. A guest whose node is not
    /// ready gets a closed stream; it opens the link when it hears ready.
    async fn link(self: Arc<Self>, incoming: Incoming) {
        let guest = incoming.link.key();
        let running = self.known(&guest).and_then(|hosted| {
            let running = hosted.node.running()?;
            Some((hosted, running))
        });
        let Some((hosted, running)) = running else {
            record("guests.link_without_node", &guest, json!({}));
            return;
        };
        // Counted until the stream's two halves are gone, which is when its
        // place on the connection is free.
        let _carried = Carried::begin(&hosted.links);
        let runtime = Handle::current();
        let carried =
            tokio::task::spawn_blocking(move || carry(incoming.stream, &running.link, &runtime))
                .await;
        let ended = match carried {
            Ok(Ok(())) => "ended".to_owned(),
            Ok(Err(error)) => error.to_string(),
            Err(error) => error.to_string(),
        };
        record("guests.link_ended", &guest, json!({"reason": ended}));
    }

    /// A build the guest uploads: the list of its files, then each file's
    /// bytes in that order. Every file is checked against the list before
    /// it is written, and the answer says whether the build is placed.
    async fn file(self: Arc<Self>, incoming: Incoming) {
        let guest = incoming.link.key();
        let Stream {
            mut send, mut recv, ..
        } = incoming.stream;
        let Ok(Some(body)) = wire::read_message(&mut recv, frame::MAX_MESSAGE).await else {
            return;
        };
        let (id, files) = match FileMessage::parse(&body) {
            Ok(Some(FileMessage::Build { id, files })) => (id, files),
            // What only a host writes, or a message of a later build.
            Ok(_) => {
                let _ = wire::write_message(
                    &mut send,
                    &FileMessage::Refused {
                        reason: "this build takes no such file message".to_owned(),
                    },
                )
                .await;
                return;
            }
            Err(_) => return,
        };
        let guests = Arc::clone(&self);
        let named = id.clone();
        let mut upload = Upload {
            recv,
            runtime: Handle::current(),
        };
        let placed = tokio::task::spawn_blocking(move || {
            guests.folder(&guest).map_err(|refusal| refusal.1)?;
            let root = guests.place(&guest).install;
            guests.make_room(&guest, &root, &named);
            guest_build::place(&root, &named, &files, &mut upload)
        })
        .await
        .unwrap_or_else(|error| Err(error.to_string()));
        let answer = match placed {
            Ok(_) => {
                record("guests.build_placed", &guest, json!({"build_id": id}));
                FileMessage::BuildPlaced { id }
            }
            Err(reason) => {
                record(
                    "guests.build_refused",
                    &guest,
                    json!({"build_id": id, "reason": reason}),
                );
                FileMessage::Refused { reason }
            }
        };
        let _ = wire::write_message(&mut send, &answer).await;
        let _ = send.shutdown().await;
    }
}

/// How long an upload may bring nothing before it is given up. Each read
/// of the stream has this long, whatever it brings: a build is never too
/// slow to arrive, only silent. (A relay carries about 2 MiB a second, so a
/// whole file within one such wait was more than it could do.)
const UPLOAD_QUIET_WITHIN: Duration = limits::WRITE_WITHIN;

/// Some of an upload's next bytes, waited for at most
/// [`UPLOAD_QUIET_WITHIN`].
async fn upload_read<R>(recv: &mut R, buffer: &mut [u8]) -> io::Result<usize>
where
    R: AsyncRead + Unpin,
{
    tokio::time::timeout(UPLOAD_QUIET_WITHIN, recv.read(buffer))
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::TimedOut))?
}

/// An upload's bytes for the blocking code that places them
/// ([`guest_build::place`]): each read is the stream's own, on the runtime
/// ([`upload_read`]).
struct Upload<R> {
    recv: R,
    runtime: Handle,
}

impl<R: AsyncRead + Unpin> io::Read for Upload<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.runtime.block_on(upload_read(&mut self.recv, buffer))
    }
}

enum Asked {
    Answer(Manage),
    /// The node's standing is told from now on.
    Watch(Arc<GuestNode>),
    /// The node's box is sent from now on.
    Events(Events),
    Nothing,
}

/// One node's events on their way to the stream that asked for its box.
struct Events {
    node: Arc<GuestNode>,
    /// The name the node knows this stream's ask by.
    listener: u64,
    events: tokio::sync::mpsc::Receiver<serde_json::Value>,
}

/// Carries a guest's link stream to its node's endpoint and back until one
/// side ends, which ends the other. Blocking: one thread per direction,
/// this one and one it starts and joins.
fn carry(stream: Stream, endpoint: &Path, runtime: &Handle) -> io::Result<()> {
    let Stream {
        mut send, mut recv, ..
    } = stream;
    let node = connect_to_node(endpoint, LINK_WITHIN)?;
    let mut to_node = node.duplicate();
    let mut from_node = node;
    let closer = from_node.shutdown_handle();
    // The node's bytes to the guest. It ends when the node closes, when the
    // guest takes no bytes in time, and when the other direction shuts the
    // node's stream down.
    let (ended, gone) = tokio::sync::oneshot::channel::<()>();
    let back = {
        let runtime = runtime.clone();
        std::thread::Builder::new()
            .name("guest-link".to_owned())
            .spawn(move || {
                let _ended = ended;
                let mut buffer = vec![0u8; CARRY];
                loop {
                    let read = match from_node.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => read,
                    };
                    let sent = runtime.block_on(async {
                        tokio::time::timeout(limits::WRITE_WITHIN, async {
                            send.write_all(&buffer[..read]).await?;
                            send.flush().await
                        })
                        .await
                    });
                    if !matches!(sent, Ok(Ok(()))) {
                        break;
                    }
                }
                let _ = runtime.block_on(send.shutdown());
            })?
    };
    // The guest's bytes to the node, on this thread, until the guest's
    // stream ends, the node takes no more, or the other direction ended.
    let mut gone = gone;
    let mut buffer = vec![0u8; CARRY];
    let result = loop {
        let read = runtime.block_on(async {
            tokio::select! {
                read = recv.read(&mut buffer) => Some(read),
                _ = &mut gone => None,
            }
        });
        match read {
            None | Some(Ok(0)) => break Ok(()),
            Some(Ok(read)) => {
                if let Err(error) = to_node
                    .write_all(&buffer[..read])
                    .and_then(|()| to_node.flush())
                {
                    break Err(error);
                }
            }
            Some(Err(error)) => break Err(error),
        }
    };
    closer.shutdown();
    let _ = back.join();
    result
}

/// A build whose node is a stand-in, for tests on both sides of a guest's
/// node.
#[cfg(all(test, unix))]
pub(crate) mod stand_in {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use hide_peer::protocol::launch;

    /// Lays out in `folder` the programs of "another build" whose node is a
    /// few lines of shell: all a host knows of a guest's node is the launch
    /// contract, which they keep. They say they are ready, name the link
    /// endpoint a test listens on, and end when told to or when their input
    /// ends.
    pub(crate) fn programs(folder: &Path) {
        write(folder, &node("[]", ""));
    }

    /// The same programs with a node that keeps a box: it names the
    /// capability, sends one event each time it is asked for its box and
    /// one more for the first acknowledgement, and each event says the
    /// place it was last asked from and how many acknowledgements and how
    /// many "the guest is away" it heard before it. Every event carries a
    /// field this build does not know.
    pub(crate) fn programs_with_a_box(folder: &Path) {
        let event = r#"seq=$((seq+1)); printf '{"event":{"v":1,"outbox":"box","seq":%s,"kind":"letter","key":"k%s","at_unix_ms":7,"body":{"from":%s,"acked":%s,"away":%s},"later":"kept"}}\n' "$seq" "$seq" "$from" "$acked" "$away""#;
        let lines = format!(
            "    *'\"resume\"'*) from=$(printf '%s' \"$line\" | sed 's/.*\"seq\":\\([0-9]*\\).*/\\1/'); {event} ;;\n    *'\"ack\"'*) acked=$((acked+1)); if [ \"$acked\" = 1 ]; then {event}; fi ;;\n    *'\"away\"'*) away=$((away+1)) ;;\n"
        );
        write(folder, &node(r#"["events"]"#, &lines));
    }

    /// The node's script: its ready line, then what it does with each line
    /// of its host (`cases`, the arms of a `case "$line"`), ending when
    /// told to end now or when its input ends. Told to end with its last
    /// pane it stays, as a node whose panes still run does.
    fn node(capabilities: &str, cases: &str) -> String {
        format!(
            "#!/bin/sh\nseq=0; from=0; acked=0; away=0\nprintf '%s\\n' '{{\"guest_node\":{},\"build\":\"9.9.9\",\"build_id\":\"0000000000000000\",\"pid\":'\"$$\"',\"link\":\"{}\",\"tap\":\"{}\",\"tap_versions\":[{},{}],\"capabilities\":{capabilities}}}'\nwhile read -r line; do\n  case \"$line\" in\n{cases}    *'\"end\":\"now\"'*) exit 0 ;;\n  esac\ndone\nexit 0\n",
            launch::GUEST_NODE,
            launch::LINK_ENDPOINT,
            launch::TAP_ENDPOINT,
            hide_peer::protocol::tap::OLDEST_TAP,
            hide_peer::protocol::tap::NEWEST_TAP,
        )
    }

    fn write(folder: &Path, node: &str) {
        for (name, bytes) in [
            ("hided", node.as_bytes()),
            ("hide", b"hide".as_slice()),
            ("hide-agent-hooks", b"hooks".as_slice()),
            ("hide-holder", b"holder".as_slice()),
        ] {
            let path = folder.join(name);
            std::fs::write(&path, bytes).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use hide_peer::protocol::build::BuildFile;
    use hide_peer::protocol::launch::EndMode;

    use super::*;
    use crate::peer_host::Link;
    use crate::peer_host::fixture::{Machine, paired, short_root, within};

    /// A stream to a guest's node is kept for as long as the node runs: the
    /// wait its connect was given is over long before the link and the tap
    /// are, and a bounded connect leaves that wait on every later read and
    /// write of its stream.
    #[test]
    fn a_stream_to_a_node_is_kept_past_the_wait_its_connect_was_given() {
        use std::io::{Read, Write};
        let root = short_root();
        let endpoint = root.path().join("link");
        let listener = hide_platform::ipc::LocalListener::bind(&endpoint).unwrap();
        let wait = Duration::from_millis(200);
        let asked = Instant::now();
        let mut ours = connect_to_node(&endpoint, wait).unwrap();
        let mut theirs = listener.accept().unwrap();
        while asked.elapsed() <= wait {
            std::thread::park_timeout(wait); // wait-ok: the connect's own wait to be over on the clock
        }
        let mut read = [0u8; 4];
        theirs.write_all(b"late").unwrap();
        ours.read_exact(&mut read).unwrap();
        assert_eq!(&read, b"late");
        ours.write_all(b"back").unwrap();
        theirs.read_exact(&mut read).unwrap();
        assert_eq!(&read, b"back");
    }

    /// A host that takes guests, and one guest paired to it and connected.
    struct Pair {
        host: Machine,
        guest: Machine,
        link: Link,
        _home: tempfile::TempDir,
    }

    impl Pair {
        async fn new() -> Self {
            let host = Machine::at("windows-pc", short_root());
            let guest = Machine::new("macbook");
            let home = tempfile::tempdir().unwrap();
            // The host's own build has no programs: every build a guest
            // names is one this machine lacks.
            Guests::new(
                host.dir.path().to_path_buf(),
                home.path().to_path_buf(),
                HelperPackages::new(None, crate::cli::RELEASE),
                None,
            )
            .serve(&host.host, host.shutdown.clone());
            let link = paired(&host, &guest).await;
            Self {
                host,
                guest,
                link,
                _home: home,
            }
        }

        fn guest_key(&self) -> PeerKey {
            self.guest.host.peers().unwrap().key()
        }
    }

    async fn ask(stream: &mut Stream, message: &Manage) {
        wire::write_message(&mut stream.send, message)
            .await
            .unwrap();
    }

    async fn answer(stream: &mut Stream) -> Manage {
        let body = within(
            "a management message",
            wire::read_message(&mut stream.recv, frame::MAX_MESSAGE),
        )
        .await
        .unwrap()
        .expect("the management stream stays open");
        Manage::parse(&body)
            .unwrap()
            .expect("a message this build knows")
    }

    fn start(build_id: Option<&str>) -> Manage {
        Manage::NodeStart {
            build: "9.9.9".to_owned(),
            build_id: build_id.map(str::to_owned),
            release: None,
        }
    }

    /// Uploads `files` as the build `id`, each file's bytes from `bytes`,
    /// and answers what the host said.
    async fn upload(link: &Link, id: &str, files: &[BuildFile], bytes: &[&[u8]]) -> FileMessage {
        let mut stream = link.open(StreamKind::File).await.unwrap();
        wire::write_message(
            &mut stream.send,
            &FileMessage::Build {
                id: id.to_owned(),
                files: files.to_vec(),
            },
        )
        .await
        .unwrap();
        for bytes in bytes {
            stream.send.write_all(bytes).await.unwrap();
        }
        stream.send.flush().await.unwrap();
        let body = within(
            "the host's answer to the build",
            wire::read_message(&mut stream.recv, frame::MAX_MESSAGE),
        )
        .await
        .unwrap()
        .expect("an answer before the stream ends");
        FileMessage::parse(&body).unwrap().expect("a known answer")
    }

    /// The next management message as its fields, whatever this build
    /// knows of them.
    async fn fields(stream: &mut Stream) -> serde_json::Value {
        let body = within(
            "a management message",
            wire::read_message(&mut stream.recv, frame::MAX_MESSAGE),
        )
        .await
        .unwrap()
        .expect("the management stream stays open");
        serde_json::from_slice(&body).unwrap()
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_nodes_box_reaches_its_guest_as_the_node_wrote_it_and_the_node_hears_how_far_it_was_recorded()
     {
        let pair = Pair::new().await;
        let programs = tempfile::tempdir().unwrap();
        stand_in::programs_with_a_box(programs.path());
        let build = HelperPackages::new(Some(programs.path().to_path_buf()), crate::cli::RELEASE)
            .guest_build(std::env::consts::OS, std::env::consts::ARCH)
            .unwrap_or_else(|error| panic!("the stand-in build is whole: {error}"));
        let bytes: Vec<&[u8]> = (0..build.files().len())
            .map(|index| build.bytes(index).unwrap())
            .collect();
        upload(&pair.link, build.id(), build.files(), &bytes).await;
        let mut management = pair.link.open(StreamKind::Management).await.unwrap();
        ask(&mut management, &start(Some(build.id()))).await;
        loop {
            match answer(&mut management).await {
                Manage::NodePreparing { .. } => {}
                Manage::NodeReady { .. } => break,
                other => panic!("the node starts: {other:?}"),
            }
        }

        // The guest names its place, and the node's event comes as the
        // node wrote it: a field this host's build does not know is in it.
        let resume = |outbox: &str, seq| Manage::Resume {
            outbox: outbox.to_owned(),
            seq,
        };
        ask(&mut management, &resume("", 0)).await;
        let first = fields(&mut management).await;
        assert_eq!(
            first,
            json!({"t": "event", "v": 1, "outbox": "box", "seq": 1, "kind": "letter",
                   "key": "k1", "at_unix_ms": 7, "later": "kept",
                   "body": {"from": 0, "acked": 0, "away": 0}})
        );
        // What the guest recorded reaches the node.
        ask(
            &mut management,
            &Manage::EventAck {
                outbox: "box".to_owned(),
                seq: 1,
            },
        )
        .await;
        let second = fields(&mut management).await;
        assert_eq!(
            (&second["seq"], &second["body"]["acked"]),
            (&json!(2), &json!(1))
        );

        // The guest goes: the node is told before the host lets go of the
        // stream, so its next event waits for whoever asks next.
        management.send.shutdown().await.unwrap();
        while let Ok(Some(_)) = within(
            "the stream's end",
            wire::read_message(&mut management.recv, frame::MAX_MESSAGE),
        )
        .await
        {}
        let mut again = pair.link.open(StreamKind::Management).await.unwrap();
        ask(&mut again, &resume("box", 2)).await;
        let third = fields(&mut again).await;
        assert_eq!(
            (&third["seq"], &third["body"]),
            (&json!(3), &json!({"from": 2, "acked": 1, "away": 1}))
        );
        ask(&mut again, &Manage::NodeEnd { mode: EndMode::Now }).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_guest_hears_the_platform_to_build_for_and_a_build_that_is_not_its_list_is_refused() {
        let pair = Pair::new().await;
        let mut management = pair.link.open(StreamKind::Management).await.unwrap();

        ask(&mut management, &start(None)).await;
        assert_eq!(
            answer(&mut management).await,
            Manage::NodePreparing {
                os: std::env::consts::OS.to_owned(),
                arch: std::env::consts::ARCH.to_owned(),
            }
        );

        let files = vec![BuildFile {
            path: "hided".to_owned(),
            size: 4,
            sha256: "0".repeat(64),
            executable: true,
        }];
        let id = build::id_of(&files);
        ask(&mut management, &start(Some(&id))).await;
        assert!(
            matches!(
                answer(&mut management).await,
                Manage::NodeStopped {
                    reason: NodeStop::BuildMissing,
                    retry: false,
                    ..
                }
            ),
            "a build this machine lacks is asked of the guest"
        );

        // Four bytes that are not the file the list names.
        let refused = upload(&pair.link, &id, &files, &[b"node"]).await;
        assert!(
            matches!(refused, FileMessage::Refused { .. }),
            "{refused:?}"
        );
        let root = layout::guest_install_root(pair.host.dir.path(), &pair.guest_key());
        assert_eq!(
            guest_build::placed(&root, &id, std::env::consts::OS),
            None,
            "a build with a wrong file never counts as placed"
        );
        ask(&mut management, &start(Some(&id))).await;
        assert!(matches!(
            answer(&mut management).await,
            Manage::NodeStopped {
                reason: NodeStop::BuildMissing,
                ..
            }
        ));
    }

    fn key(byte: u8) -> PeerKey {
        PeerKey::from_hex(&format!("{byte:02x}").repeat(32)).unwrap()
    }

    /// D-49, D-75: the folder a device helper ran in goes to the first
    /// paired machine that asks for it with the helper's root, a later one
    /// is told who holds it and keeps a folder of its own, and a holder
    /// whose pairing never finished is replaced.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_first_paired_machine_to_ask_takes_the_helpers_folder_and_a_later_one_is_told_who_holds_it()
     {
        let state = short_root();
        let home = tempfile::tempdir().unwrap();
        let guests = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        let root = "~/.hide/host-helper";
        let paired = |_: &PeerKey| true;
        assert_eq!(
            guests.take_helper(&key(1), root, paired),
            HelperFolder::Absent,
            "no helper ever ran on this account"
        );
        // What an SSH device left: its node's folder with its panes'
        // records, and a build under the root its core was allowed.
        let node = layout::device_node_state(home.path());
        std::fs::create_dir_all(layout::pane_records(&node)).unwrap();
        let build = home.path().join(".hide/host-helper/0123456789abcdef");
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(
            build.join(format!("hided{}", std::env::consts::EXE_SUFFIX)),
            b"",
        )
        .unwrap();
        assert_eq!(
            guests.take_helper(&key(1), "~/elsewhere", paired),
            HelperFolder::Absent,
            "a folder with no build of Hide's is no helper root"
        );

        assert_eq!(
            guests.take_helper(&key(1), root, paired),
            HelperFolder::Taken
        );
        assert_eq!(
            guests.take_helper(&key(1), root, paired),
            HelperFolder::Taken,
            "the same machine asks again"
        );
        let place = guests.place(&key(1));
        assert!(place.helper);
        assert_eq!(place.node, node);
        assert_eq!(place.install, home.path().join(".hide/host-helper"));

        assert_eq!(
            guests.helper_for(&key(2), root, paired),
            HelperFolder::HeldBy(key(1))
        );
        assert_eq!(
            guests.take_helper(&key(2), root, paired),
            HelperFolder::HeldBy(key(1)),
            "a later machine changes nothing"
        );
        assert!(guests.place(&key(1)).helper);
        let own = guests.place(&key(2));
        assert!(!own.helper);
        assert!(own.node.starts_with(state.path()), "{:?}", own.node);

        // The holder holds no grant here and runs nothing: its pairing
        // never finished, and the next machine takes the folder.
        assert_eq!(
            guests.take_helper(&key(2), root, |_| false),
            HelperFolder::Taken
        );
        assert!(guests.place(&key(2)).helper);
        assert!(!guests.place(&key(1)).helper);
    }

    /// A copy of a released state folder as the release left it: every file
    /// and folder private to the account, as a core writes them.
    fn copy_private(from: &Path, to: &Path) {
        private::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_private(&entry.path(), &target);
            } else {
                private::create_new_file(&target)
                    .unwrap()
                    .write_all(&std::fs::read(entry.path()).unwrap())
                    .unwrap();
            }
        }
    }

    /// B16 over the state folders released builds wrote
    /// (`hide-core/tests/fixtures/release-state/`): what this host keeps of
    /// a guest (`h/node.json`) names the guest by its device id, here the
    /// machine the release itself dialed, which then gave this one its
    /// work. The start's conversions, which rename the devices an earlier
    /// build dialed in every store they know, leave a guest's folder as it
    /// was, and the daemon that starts next reads the same guest, device
    /// and build.
    #[tokio::test]
    async fn a_guests_record_keeps_its_device_through_a_start_over_a_released_state_folder() {
        use hide_core::node_migration::{CORE_STATE, MARKER_FILE, convert, name_devices};

        let released =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../hide-core/tests/fixtures/release-state");
        let mut folders: Vec<PathBuf> = std::fs::read_dir(&released)
            .unwrap_or_else(|error| panic!("{}: {error}", released.display()))
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.is_dir())
            .collect();
        folders.sort();
        assert!(!folders.is_empty(), "no released state folder was found");
        for folder in folders {
            let tag = folder.file_name().unwrap().to_string_lossy().into_owned();
            let root = short_root();
            let state = root.path().join("s");
            copy_private(&folder, &state);
            let stored = |name: &str| -> serde_json::Value {
                serde_json::from_slice(&std::fs::read(state.join(name)).unwrap()).unwrap()
            };
            let node =
                hide_core::node::NodeId::parse(stored(MARKER_FILE)["node"].as_str().unwrap())
                    .unwrap_or_else(|error| panic!("{tag}: {error}"));
            // The upgrade's own start, which names the release's device.
            convert(&state, &node).unwrap_or_else(|refusal| panic!("{tag}: {refusal:?}"));
            name_devices(&state, &node).unwrap_or_else(|refusal| panic!("{tag}: {refusal:?}"));
            let device = stored(CORE_STATE)["device_registrations"][0]["id"]
                .as_str()
                .unwrap_or_else(|| panic!("{tag}: the release's device is not in the store"))
                .to_owned();

            let home = tempfile::tempdir().unwrap();
            let guests = || {
                Guests::new(
                    state.clone(),
                    home.path().to_path_buf(),
                    HelperPackages::new(None, crate::cli::RELEASE),
                    None,
                )
            };
            let guest = key(7);
            let record = NodeRecord {
                build_id: "1.2.3-main.0123abcd".to_owned(),
                device,
            };
            guests()
                .folder(&guest)
                .unwrap_or_else(|refusal| panic!("{tag}: {refusal:?}"));
            atomic::write_file(
                &layout::guest_node_record(&state, &guest),
                &serde_json::to_vec(&record).unwrap(),
                Access::Private,
            )
            .unwrap();
            let kept = |path: PathBuf| std::fs::read(&path).unwrap();
            let before = (
                kept(layout::guest_key(&state, &guest)),
                kept(layout::guest_node_record(&state, &guest)),
            );

            // The next start.
            convert(&state, &node).unwrap_or_else(|refusal| panic!("{tag}: {refusal:?}"));
            let named =
                name_devices(&state, &node).unwrap_or_else(|refusal| panic!("{tag}: {refusal:?}"));
            assert_eq!(named, Default::default(), "{tag}");
            assert_eq!(
                (
                    kept(layout::guest_key(&state, &guest)),
                    kept(layout::guest_node_record(&state, &guest)),
                ),
                before,
                "{tag}: the start rewrote the guest's folder"
            );
            let next = guests();
            let known: Vec<PeerKey> = next.folders().into_iter().map(|(key, _)| key).collect();
            assert_eq!(known, [guest], "{tag}");
            assert_eq!(next.remembered(&guest), Some(record), "{tag}");
        }
    }

    /// PRD B34: a relay carries about 2 MiB a second, so the largest build
    /// (`build::MAX_BYTES`) takes two minutes to arrive, longer than any
    /// one wait of the link. Its bytes are read to the end, because each
    /// read has its own wait, and a stream that then brings nothing is
    /// given up after one such wait.
    #[tokio::test(start_paused = true)]
    async fn a_build_arriving_at_a_relays_speed_is_read_whole_and_a_silent_stream_is_given_up() {
        const A_SECOND: usize = 2 * 1024 * 1024;
        let total = build::MAX_BYTES as usize;
        let (mut send, mut recv) = tokio::io::duplex(64 * 1024);
        let relay = tokio::spawn(async move {
            let piece = vec![7u8; A_SECOND];
            let mut sent = 0;
            while sent < total {
                send.write_all(&piece).await.unwrap();
                sent += piece.len();
                tokio::time::sleep(Duration::from_secs(1)).await; // wait-ok: the paused clock, for a relay's speed
            }
            // Open and silent from here.
            send
        });
        let began = tokio::time::Instant::now();
        let mut buffer = vec![0u8; 64 * 1024];
        let mut read = 0;
        while read < total {
            read += upload_read(&mut recv, &mut buffer)
                .await
                .unwrap_or_else(|error| panic!("after {read} bytes: {error}"));
        }
        assert_eq!(read, total);
        let took = began.elapsed();
        assert!(
            took >= Duration::from_secs((total / A_SECOND) as u64 - 1)
                && took > 4 * UPLOAD_QUIET_WITHIN,
            "{took:?}"
        );

        let quiet = tokio::time::Instant::now();
        let silent = upload_read(&mut recv, &mut buffer).await;
        assert_eq!(
            silent.map_err(|error| error.kind()),
            Err(io::ErrorKind::TimedOut)
        );
        let waited = quiet.elapsed();
        assert!(
            waited >= UPLOAD_QUIET_WITHIN && waited < UPLOAD_QUIET_WITHIN + Duration::from_secs(1),
            "{waited:?}"
        );
        // The guest lets the stream go: the upload ends there.
        drop(relay.await.unwrap());
        assert_eq!(upload_read(&mut recv, &mut buffer).await.ok(), Some(0));
    }

    /// A guest keeps [`launch::BUILDS_KEPT`] builds on this machine however
    /// many it sends: room is made before a build is placed, so one that
    /// never brought a node up goes when the next arrives, and the build
    /// the guest's node last ran from stays for the node to start from.
    #[tokio::test]
    async fn room_is_made_for_a_build_before_it_is_placed() {
        let state = short_root();
        let home = tempfile::tempdir().unwrap();
        let guests = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        let guest = key(7);
        guests.folder(&guest).unwrap();
        let root = layout::guest_install_root(state.path(), &guest);
        let ran = "0000000000000001";
        atomic::write_file(
            &layout::guest_node_record(state.path(), &guest),
            &serde_json::to_vec(&NodeRecord {
                build_id: ran.to_owned(),
                device: "macbook".to_owned(),
            })
            .unwrap(),
            Access::Private,
        )
        .unwrap();
        let builds = || -> Vec<String> {
            let mut builds: Vec<String> = std::fs::read_dir(&root)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            builds.sort();
            builds
        };
        let place = |id: &str| std::fs::create_dir_all(root.join(id).join("x")).unwrap();
        place(ran);
        for sent in ["0000000000000002", "0000000000000003", "0000000000000004"] {
            guests.make_room(&guest, &root, sent);
            assert_eq!(builds(), [ran], "before {sent} is placed");
            place(sent);
            assert_eq!(builds(), [ran, sent]);
            assert!(builds().len() <= launch::BUILDS_KEPT);
        }
        // The same build sent again keeps what its earlier try left.
        guests.make_room(&guest, &root, "0000000000000004");
        assert_eq!(builds(), [ran, "0000000000000004"]);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_guests_panes_go_by_a_device_id_that_is_its_own_on_this_machine() {
        let state = short_root();
        let home = tempfile::tempdir().unwrap();
        let guests = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        let refused = |guest: u8, device: &str| {
            guests
                .node_for(&key(guest), device)
                .err()
                .map(|(reason, _)| reason)
        };
        assert_eq!(refused(1, "macbook"), None);
        assert_eq!(refused(1, "macbook"), None, "the same guest asks again");
        assert_eq!(
            refused(2, "macbook"),
            Some(NodeStop::Other),
            "another guest under the same id would share its panes' addresses"
        );
        assert_eq!(
            refused(1, "mini"),
            Some(NodeStop::Other),
            "a guest's panes keep the id they were given"
        );
        assert_eq!(refused(3, "mac:book"), Some(NodeStop::Other));
        assert_eq!(refused(3, ""), Some(NodeStop::Other));
        assert_eq!(refused(3, "mini"), None);
    }

    /// D-77 (7): a host has at most [`launch::MAX_GUEST_NODES`] guests at
    /// work at once. The guest past that is told the host is full, a guest
    /// already at work is never one more, and a place opens when a guest's
    /// work ends.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_guest_past_the_cap_is_told_the_host_is_full_until_anothers_work_ends() {
        let state = short_root();
        let home = tempfile::tempdir().unwrap();
        let programs = tempfile::tempdir().unwrap();
        stand_in::programs(programs.path());
        let packages =
            HelperPackages::new(Some(programs.path().to_path_buf()), crate::cli::RELEASE);
        let build = packages
            .guest_build(std::env::consts::OS, std::env::consts::ARCH)
            .unwrap_or_else(|error| panic!("the stand-in build is whole: {error}"))
            .id()
            .to_owned();
        let guests = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            packages,
            None,
        );
        let start = |guest: u8| {
            let (guests, build) = (Arc::clone(&guests), build.clone());
            async move {
                guests
                    .start(key(guest), &format!("device-{guest}"), build)
                    .await
                    .map(|_| ())
                    .map_err(|(reason, detail)| (reason, detail.contains("as many")))
            }
        };
        let last = u8::try_from(launch::MAX_GUEST_NODES).unwrap();
        for guest in 1..=last {
            assert_eq!(start(guest).await, Ok(()), "guest {guest}");
        }
        assert_eq!(start(last + 1).await, Err((NodeStop::HostFull, true)));
        assert!(guests.node(&key(last + 1)).unwrap().running().is_none());
        assert_eq!(start(1).await, Ok(()), "a guest at work asks again");

        let first = guests.node(&key(1)).unwrap();
        first.end(EndMode::Now);
        until("the first guest's node to end", || {
            first.stopped_by(Instant::now())
        })
        .await;
        assert_eq!(
            start(last + 1).await,
            Ok(()),
            "the place is the next guest's"
        );
        assert_eq!(start(1).await, Err((NodeStop::HostFull, true)));
        // Asking for its stopped node again is one more guest all the same
        // (B70's retry goes through the same count), and starts nothing.
        let again = guests.asked(key(1), "device-1", Manage::NodeRetry).await;
        assert!(
            matches!(
                &again,
                Asked::Answer(Manage::NodeStopped {
                    reason: NodeStop::HostFull,
                    retry: false,
                    ..
                })
            ),
            "a retry past the cap is told the host is full"
        );
        assert!(first.stopped_by(Instant::now()));
        assert!(first.running().is_none());
    }

    /// What an earlier run of this daemon left of `guest`: its folder, its
    /// build (a stand-in node), the build its node ran from and the one
    /// pane it had.
    #[cfg(unix)]
    fn left_by_an_earlier_run(state: &Path, home: &Path, guest: &PeerKey) {
        use hide_peer::protocol::panes::{self, Pane, Panes};

        let programs = tempfile::tempdir().unwrap();
        stand_in::programs(programs.path());
        let build = HelperPackages::new(Some(programs.path().to_path_buf()), crate::cli::RELEASE)
            .guest_build(std::env::consts::OS, std::env::consts::ARCH)
            .unwrap_or_else(|error| panic!("the stand-in build is whole: {error}"));
        let earlier = Guests::new(
            state.to_path_buf(),
            home.to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        earlier.folder(guest).unwrap();
        build
            .place(&layout::guest_install_root(state, guest))
            .unwrap();
        let write = |path: PathBuf, bytes: Vec<u8>| {
            private::create_dir_all(path.parent().unwrap()).unwrap();
            atomic::write_file(&path, &bytes, Access::Private).unwrap();
        };
        write(
            layout::guest_node_record(state, guest),
            serde_json::to_vec(&NodeRecord {
                build_id: build.id().to_owned(),
                device: "macbook".to_owned(),
            })
            .unwrap(),
        );
        write(
            layout::guest_panes(state, guest),
            serde_json::to_vec(&Panes {
                version: panes::VERSION,
                panes: vec![Pane {
                    pane: "w1:p1".to_owned(),
                    rows: 24,
                    cols: 80,
                    since_unix_ms: 5,
                }],
            })
            .unwrap(),
        );
        // The hook entries the guest's node wrote into this account's
        // Claude Code settings, beside another Hide's.
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        for install in [guest_install(state, guest), "app".to_owned()] {
            // An entry whose helper is gone is no installation's any more.
            let helper = home.join("helper");
            std::fs::write(&helper, b"").unwrap();
            hide_agent_hooks::install::install(
                hide_agent_hooks::AgentRuntime::ClaudeCode,
                home,
                hide_agent_hooks::route::HookTarget::new(&helper, state)
                    .and_then(|target| target.of_install(&install))
                    .unwrap(),
            )
            .unwrap();
        }
    }

    /// The name the hook entries of `guest`'s installation carry.
    fn guest_install(state: &Path, guest: &PeerKey) -> String {
        layout::install_of_root(&layout::guest_install_root(state, guest))
    }

    /// Whether this account's Claude Code settings carry entries of the
    /// installation `install`.
    fn hooked(home: &Path, install: &str) -> bool {
        !matches!(
            hide_agent_hooks::install::status(
                hide_agent_hooks::AgentRuntime::ClaudeCode,
                home,
                Some(install),
            ),
            hide_agent_hooks::HookStatus::NotInstalled
        )
    }

    /// Waits for `state` without a timed pause: it is looked at again each
    /// time the runtime has run what else was ready.
    #[cfg(unix)]
    async fn until(what: &str, state: impl Fn() -> bool) {
        within(what, async {
            while !state() {
                tokio::task::yield_now().await;
            }
        })
        .await;
    }

    /// B26: the host takes a guest off, and ends its work here or not.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_guest_taken_off_keeps_its_panes_until_the_host_ends_them_and_then_nothing_of_it_is_left()
     {
        use hide_peer::protocol::tap::{PaneSize, ToHost, ToNode};

        let state = short_root();
        let home = tempfile::tempdir().unwrap();
        let guest = key(7);
        left_by_an_earlier_run(state.path(), home.path(), &guest);
        // This test is the tap of the node the stand-in says is ready.
        let node_dir = layout::guest_node_side(state.path(), &guest);
        std::fs::create_dir_all(&node_dir).unwrap();
        let taps = ipc::LocalListener::bind(&node_dir.join(launch::TAP_ENDPOINT)).unwrap();
        let guests = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        {
            let guests = Arc::clone(&guests);
            tokio::task::spawn_blocking(move || guests.resume())
                .await
                .unwrap();
        }
        let mut node = within(
            "the tap of the guest's node",
            tokio::task::spawn_blocking(move || {
                let mut node = taps.accept().unwrap();
                assert!(matches!(tap::tests::heard(&mut node), ToNode::Hello { .. }));
                node
            }),
        )
        .await
        .unwrap();
        let say = |node: &mut LocalStream, frame: ToHost| {
            node.write_all(&frame.encode().unwrap()).unwrap();
        };
        say(
            &mut node,
            ToHost::Welcome {
                version: hide_peer::protocol::tap::NEWEST_TAP,
                panes: vec![PaneSize {
                    pane: "w1:p1".to_owned(),
                    rows: 24,
                    cols: 80,
                }],
            },
        );

        // The guest holds no grant any more, with no word about its work:
        // the work stays. Its node is told to end with its last pane, and
        // the pane is still this machine's to show.
        guests.granted(Vec::new());
        let removed = layout::guest_removed(state.path(), &guest);
        until("the guest to be marked as taken off", || removed.exists()).await;
        assert_eq!(guests.panes().len(), 1, "its pane stays on this screen");
        assert!(guests.node(&guest).unwrap().running().is_some());
        // Its agents still run here, so its hook entries stay.
        assert!(hooked(home.path(), &guest_install(state.path(), &guest)));

        // The operator ends its work: the node is told to end every pane
        // now, with the guest away. The pane ends, then the node, and then
        // nothing of the guest is left here.
        guests.end_work(guest);
        say(
            &mut node,
            ToHost::Gone {
                pane: "w1:p1".to_owned(),
            },
        );
        let folder = layout::guest_dir(state.path(), &guest);
        until("the guest's folder to go", || !folder.exists()).await;
        assert!(guests.node(&guest).is_none());
        assert!(guests.panes().is_empty());
        // Its hook entries went with it, and no other Hide's did.
        assert!(!hooked(home.path(), &guest_install(state.path(), &guest)));
        assert!(hooked(home.path(), "app"));
    }

    /// A removal that was ending a guest's work when this daemon stopped
    /// goes on at its next start.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_guests_work_that_was_being_ended_when_the_host_stopped_is_ended_at_its_next_start() {
        let state = short_root();
        let home = tempfile::tempdir().unwrap();
        let guest = key(7);
        left_by_an_earlier_run(state.path(), home.path(), &guest);
        atomic::write_file(
            &layout::guest_removed(state.path(), &guest),
            END_NOW.as_bytes(),
            Access::Private,
        )
        .unwrap();
        let guests = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        // The node is started only to end the panes: `resume` returns once
        // it was ready and told, and the stand-in ends when told so.
        {
            let guests = Arc::clone(&guests);
            tokio::task::spawn_blocking(move || guests.resume())
                .await
                .unwrap();
        }
        let node = guests
            .node(&guest)
            .expect("the guest is known until its panes are gone");
        let mut phase = node.phase();
        let ended = within(
            "the node's end",
            phase.wait_for(|phase| {
                matches!(
                    phase,
                    Phase::Stopped {
                        reason: NodeStop::Ended,
                        ..
                    }
                )
            }),
        )
        .await
        .map(|phase| phase.clone());
        assert!(ended.is_ok(), "{ended:?}");
    }

    /// B16: the host's daemon starts again with the guest away.
    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_host_that_starts_again_lists_a_guests_panes_and_runs_its_node_with_the_guest_away() {
        use hide_peer::protocol::tap::ToNode;

        let state = short_root();
        let home = tempfile::tempdir().unwrap();
        let guest = key(7);
        left_by_an_earlier_run(state.path(), home.path(), &guest);
        // This test is the tap of the node the stand-in says is ready.
        let node_dir = layout::guest_node_side(state.path(), &guest);
        std::fs::create_dir_all(&node_dir).unwrap();
        let taps = ipc::LocalListener::bind(&node_dir.join(launch::TAP_ENDPOINT)).unwrap();

        let guests = Guests::new(
            state.path().to_path_buf(),
            home.path().to_path_buf(),
            HelperPackages::new(None, crate::cli::RELEASE),
            None,
        );
        {
            let guests = Arc::clone(&guests);
            tokio::task::spawn_blocking(move || guests.resume())
                .await
                .unwrap();
        }
        assert_eq!(
            guests.panes(),
            [hide_core::guest_panes::GuestPanes {
                device_id: "macbook".to_owned(),
                key: guest.to_hex(),
                panes: vec![hide_core::guest_panes::GuestPane {
                    id: "guest:macbook:pane:w1:p1".to_owned(),
                    pane: "w1:p1".to_owned(),
                    since_unix_ms: 5,
                }],
            }],
            "the panes show before the node is back"
        );
        let hello = within(
            "the tap of the node that was started again",
            tokio::task::spawn_blocking(move || {
                let mut node = taps.accept().unwrap();
                tap::tests::heard(&mut node)
            }),
        )
        .await
        .unwrap();
        assert!(matches!(hello, ToNode::Hello { .. }), "{hello:?}");
        assert!(
            guests.node(&guest).unwrap().running().is_some(),
            "the guest's node runs with no guest connected"
        );
        tokio::task::spawn_blocking(move || guests.stop())
            .await
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_guest_uploads_its_build_and_its_node_runs_from_it_carries_its_link_and_ends_when_asked()
     {
        let pair = Pair::new().await;
        let guest = pair.guest_key();
        let state = pair.host.dir.path();
        let programs = tempfile::tempdir().unwrap();
        stand_in::programs(programs.path());
        let build = HelperPackages::new(Some(programs.path().to_path_buf()), crate::cli::RELEASE)
            .guest_build(std::env::consts::OS, std::env::consts::ARCH)
            .unwrap_or_else(|error| panic!("the stand-in build is whole: {error}"));
        let bytes: Vec<&[u8]> = (0..build.files().len())
            .map(|index| build.bytes(index).unwrap())
            .collect();
        assert_eq!(
            upload(&pair.link, build.id(), build.files(), &bytes).await,
            FileMessage::BuildPlaced {
                id: build.id().to_owned()
            }
        );
        assert_eq!(
            std::fs::read_to_string(layout::guest_key(state, &guest)).unwrap(),
            guest.to_hex(),
            "the guest's folder holds its whole key"
        );

        // The stand-in names the node's link endpoint and this test is the
        // node behind it.
        let node_dir = layout::guest_node_side(state, &guest);
        std::fs::create_dir_all(&node_dir).unwrap();
        let listener = ipc::LocalListener::bind(&node_dir.join(launch::LINK_ENDPOINT)).unwrap();
        let node = tokio::task::spawn_blocking(move || {
            let mut stream = listener.accept().unwrap();
            let mut heard = [0u8; 5];
            stream.read_exact(&mut heard).unwrap();
            stream.write_all(b"world").unwrap();
            heard
        });

        let mut management = pair.link.open(StreamKind::Management).await.unwrap();
        ask(&mut management, &start(Some(build.id()))).await;
        let ready = loop {
            match answer(&mut management).await {
                Manage::NodePreparing { .. } => {}
                other => break other,
            }
        };
        assert_eq!(
            ready,
            Manage::NodeReady {
                build: "9.9.9".to_owned()
            }
        );
        // What this daemon's next start runs the guest's node from (B16) is
        // saved at the node's first start.
        until("the node's build to be remembered", || {
            std::fs::read(layout::guest_node_record(state, &guest))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<NodeRecord>(&bytes).ok())
                .is_some_and(|record| record.build_id == build.id())
        })
        .await;

        let mut link = pair.link.open(StreamKind::NodeLink).await.unwrap();
        link.send.write_all(b"hello").await.unwrap();
        link.send.flush().await.unwrap();
        let mut back = [0u8; 5];
        within("the node's bytes", link.recv.read_exact(&mut back))
            .await
            .unwrap();
        assert_eq!(&back, b"world");
        assert_eq!(&within("the node's read", node).await.unwrap(), b"hello");
        // The node closed its end: the guest's stream ends with it.
        let mut rest = Vec::new();
        let ended = within("the link's end", link.recv.read_to_end(&mut rest)).await;
        assert!(matches!(ended, Ok(0) | Err(_)), "{ended:?}");

        ask(&mut management, &Manage::NodeEnd { mode: EndMode::Now }).await;
        assert_eq!(
            answer(&mut management).await,
            Manage::NodeStopped {
                reason: NodeStop::Ended,
                retry: false,
                detail: None,
            }
        );
    }
}
