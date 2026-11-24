//! `hided node serve --guest <dir>`: a guest's node (PRD device-connect-iroh
//! D-61, D-77). Another machine's work runs on this machine in this
//! process, from that machine's own build; this machine's hided starts it,
//! supervises it over its standard streams and joins the guest's
//! connection to its link endpoint.
//!
//! What the two builds agree on is `hide_peer_protocol::launch` and
//! nothing else: the folder, the two endpoints, the ready line, the lines
//! after it and the exit codes. The node lives until its host tells it to
//! end or the host's end of its input closes; a guest's connection comes
//! and goes meanwhile, and the node keeps its pane holders, their records
//! and its ownership of them across every one (D-77 (5)).

pub(crate) mod away;
mod event_box;
mod tap;

use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use hide_host::pane_holders::PaneSlot;
use hide_host::serve::{KitPlace, Services, Terminals as _};
use hide_node::terminal::device::{NodeTerminals, ResidentFolders};
use hide_node_link::holders::PaneHolders;
use hide_peer::protocol::launch::{self, EndMode, FromNode, Ready, ToNode};
use hide_peer::protocol::tap as tap_contract;
use hide_platform::ipc::{LocalListener, LocalStream, ShutdownHandle};
use serde_json::json;

/// How long the node waits at its start for an earlier node of this folder
/// to let its panes go, before it answers that the folder is held.
const OWN_WITHIN: Duration = Duration::from_secs(5);
/// How long the node's own look at every pane's record may take at its
/// start and before it ends every pane.
const SCAN_WITHIN: Duration = Duration::from_secs(10);
/// How long a node that ends waits for its host to have taken the list of
/// the panes that ended with it.
const TAP_FLUSH_WITHIN: Duration = Duration::from_secs(2);
/// How often the node looks again at whether a pane still runs, between
/// the events that tell it to look: a pane whose program ends on its own
/// tells no one while no guest is connected.
const LIVE_CHECK: Duration = Duration::from_secs(5);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // What these locks guard is plain values a panic left whole.
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// One record on the node's standard error, which its host keeps in its own
/// log under the guest's name. A host that is gone reads nothing, and the
/// write's failure is not an event: the node's input ends then.
fn record(kind: &str, extra: serde_json::Value) {
    let mut line = json!({
        "component": "guest_node",
        "kind": kind,
        "at_unix_ms": now_unix_ms(),
    });
    if let (Some(line), Some(extra)) = (line.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            line.insert(key.clone(), value.clone());
        }
    }
    let _ = writeln!(io::stderr().lock(), "{line}");
}

fn exit(code: i32, kind: &str, message: String) -> ! {
    record(kind, json!({"message": message, "exit": code}));
    std::process::exit(code)
}

/// What the node does about a pane whose holder died without saying why.
/// Its own next start ends that pane's record as lost and wakes its agent
/// (`Holders::recover`), so the node ends with [`launch::EXIT_RESTART`] and
/// its host starts it again, every other pane staying in its holder. It
/// ends so at most once in [`launch::HOLDER_LOST_RESTART_SECS`], counted in
/// its own folder across its starts; a pane lost sooner stays unreachable
/// until the next start, and the host's daemon never restarts for it.
fn restart_for_lost_holder(dir: PathBuf) -> Arc<dyn Fn(&str) + Send + Sync> {
    use crate::holder_restart::{Asked, request};
    Arc::new(
        move |pane| match request(&dir, std::time::SystemTime::now(), &|| Ok(())) {
            Ok(Asked::Requested) => exit(
                launch::EXIT_RESTART,
                "guest_node.restart_for_lost_holder",
                format!("the holder of {pane} was lost"),
            ),
            Ok(Asked::Waiting) => record(
                "guest_node.holder_lost",
                json!({"pane": pane, "outcome": "already_asked"}),
            ),
            Err(error) => record(
                "guest_node.holder_lost",
                json!({"pane": pane, "outcome": format!("failed: {error}")}),
            ),
        },
    )
}

/// Writes the host one line. A host that cannot be written to hears
/// nothing more of this node, so the node ends.
fn say(line: &impl serde::Serialize) {
    let written = serde_json::to_string(line)
        .map_err(io::Error::other)
        .and_then(|line| {
            let mut out = io::stdout().lock();
            writeln!(out, "{line}")?;
            out.flush()
        });
    if written.is_err() {
        std::process::exit(launch::EXIT_HOST_UNREACHABLE);
    }
}

/// Writes the host one event of the box, as the line it is kept as: a
/// field this build does not know goes with it.
fn say_event(event: &str) {
    let mut out = io::stdout().lock();
    if writeln!(out, "{{\"event\":{event}}}")
        .and_then(|()| out.flush())
        .is_err()
    {
        std::process::exit(launch::EXIT_HOST_UNREACHABLE);
    }
}

/// What a diagnostic of the node's own parts is written to: up the guest's
/// link while there is one, and otherwise the host's log.
fn diagnostic(value: serde_json::Value) {
    hide_node::terminal::device::forward_diagnostic_or(value, |value| {
        let _ = writeln!(io::stderr().lock(), "{value}");
    });
}

/// A node whose `--guest` folder is the account's device helper folder
/// (`hide_kit::layout::device_node_state`): the machine that dialed a
/// helper here was paired, and its node goes on in the helper's places
/// (PRD device-connect-iroh D-49). The panes that already run there name
/// the helper's installation and ask in the account's state folder, so the
/// node keeps both: it leaves the process's state folder as it is, runs its
/// holders from the account's version folders and applies the helper's kit
/// under the helper root. What a guest's node adds, its endpoints, its box
/// and its ownership of the panes across links, sits in that same folder.
struct Helper {
    home: PathBuf,
    /// Whether this program runs from a build folder of the helper root,
    /// where the host placed it: only then is it the helper's installation.
    placed: bool,
    /// The helper root the host's record names.
    root: Option<PathBuf>,
    /// The installation the folder's panes name when it is not the helper
    /// root's: the host's own Hide ran its node here before it was split
    /// from the core this node belongs to (PRD device-connect-iroh D-46).
    panes: Option<hide_kit::layout::PanesInstall>,
}

impl Helper {
    fn of(dir: &Path, program: &Path) -> Option<Self> {
        let home = hide_platform::host::home_dir().ok()?;
        if dir != hide_kit::layout::device_node_state(&home) {
            return None;
        }
        // The host's record of the machine that took the folder names the
        // root its helper ran from.
        let taken = hide_kit::layout::HelperGuest::read(dir);
        let root = taken.as_ref().map(|taken| taken.root.clone());
        let placed = program
            .parent()
            .is_some_and(|build| hide_peer::protocol::build::is_id(&build_name(build)))
            && root.is_some()
            && program.parent().and_then(Path::parent) == root.as_deref();
        Some(Self {
            home,
            placed,
            root,
            panes: taken.and_then(|taken| taken.panes),
        })
    }
}

fn build_name(build: &Path) -> String {
    build
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The words after `serve` were `--guest <dir>`: runs the node over `dir`
/// until it is told to end or its input ends.
pub fn run(dir: &str) -> ! {
    // A thread that panicked would leave a node that looks alive and
    // answers nothing; the host counts the exit and starts another.
    std::panic::set_hook(Box::new(|info| {
        record("guest_node.panicked", json!({"message": info.to_string()}));
        std::process::exit(launch::EXIT_HOST_UNREACHABLE);
    }));
    let dir = PathBuf::from(dir);
    if !dir.is_absolute() {
        exit(
            launch::EXIT_FOLDER_REFUSED,
            "guest_node.folder_refused",
            format!("{} is not an absolute path", dir.display()),
        );
    }
    let refused = |error: io::Error| -> ! {
        exit(
            launch::EXIT_FOLDER_REFUSED,
            "guest_node.folder_refused",
            format!("{}: {error}", dir.display()),
        )
    };
    if let Err(error) = hide_platform::fs::private::create_dir_all(&dir)
        .and_then(|()| hide_platform::fs::private::own_dir(&dir).map(drop))
    {
        refused(error);
    }
    // Every endpoint the node binds below its folder fits where the longest
    // one does.
    if let Err(error) =
        hide_platform::ipc::check_path(&dir.join("x".repeat(launch::NODE_SOCKET_TAIL - 1)))
    {
        refused(error);
    }
    let program = std::env::current_exe().unwrap_or_else(|error| refused(error));
    let helper = Helper::of(&dir, &program);
    // Everything below reads the node's state folder from the process, as
    // a device's node does, and a pane inherits it: its `hide` and its
    // hooks then reach this node and no other Hide of the account. A
    // helper's folder keeps the helper's places instead, so the panes that
    // already run there go on reaching their node (D-49).
    if helper.is_none() {
        // SAFETY: no other thread of this process runs yet.
        unsafe {
            std::env::set_var("HIDE_STATE_DIR", &dir);
        }
    }
    // The panes a split left in this folder name the host's own
    // installation, and so does everything this node answers and starts:
    // their `hide` and hooks ask for that name (D-46).
    if let Some(panes) = helper.as_ref().and_then(|helper| helper.panes.as_ref()) {
        let install =
            panes
                .id
                .clone()
                .zip(panes.cli.clone())
                .map(|(id, cli)| hide_host::kit::PaneInstall {
                    id,
                    cli,
                    path_first: None,
                });
        hide_host::kit::answer_as(install);
    }
    hide_node::diagnostics::install(diagnostic);
    let bind = |name: &str| -> LocalListener {
        let path = dir.join(name);
        LocalListener::bind(&path).unwrap_or_else(|error| {
            let (code, kind) = match error.kind() {
                io::ErrorKind::AddrInUse => (launch::EXIT_FOLDER_HELD, "guest_node.folder_held"),
                _ => (launch::EXIT_FOLDER_REFUSED, "guest_node.folder_refused"),
            };
            exit(code, kind, format!("{}: {error}", path.display()))
        })
    };
    let link = bind(launch::LINK_ENDPOINT);
    let taps = bind(launch::TAP_ENDPOINT);
    let away = bind(away::ENDPOINT);
    // A box this build cannot read stays as it is for a build that can:
    // the node runs without one and says so to whoever sends a letter.
    let events = match event_box::EventBox::open(&dir, Box::new(say_event)) {
        Ok(events) => Some(Arc::new(events)),
        Err(error) => {
            record(
                "guest_node.box_unreadable",
                json!({"error": error.to_string()}),
            );
            None
        }
    };

    let install_root = launch::runs_from_install_root(&dir, &program);
    let hub = Arc::new(tap::TapHub::default());
    let terminals = Arc::new(NodeTerminals::resident(
        Arc::clone(&hub) as Arc<dyn hide_node::terminal::OutputSink>,
        ResidentFolders {
            state_dir: dir.clone(),
            holders_home: match &helper {
                // Where the helper's holders already run from.
                Some(helper) => helper.home.clone(),
                // A node that does not run from a guest's install root (a
                // development build) keeps its holders' version folders in
                // its own folder, never in the account's.
                None => install_root.clone().unwrap_or_else(|| dir.clone()),
            },
        },
    ));
    // The node owns its panes from its start to its end: it takes them
    // before any guest connects, so a pane is ended, and a lost holder is
    // seen, with no guest there (D-77 (5), B16).
    let holders = terminals.start_holders().unwrap_or_else(|message| {
        exit(
            launch::EXIT_FOLDER_REFUSED,
            "guest_node.holders_failed",
            message,
        )
    });
    // A holder that died without saying why is settled by a start of this
    // node, which it asks its host for by ending (D-77 (6)).
    terminals.on_holder_lost(restart_for_lost_holder(dir.clone()));
    let slot = PaneSlot::holding(holders);
    let owned = slot.own(OWN_WITHIN).unwrap_or_else(|error| {
        exit(
            launch::EXIT_FOLDER_HELD,
            "guest_node.folder_held",
            format!("another node still owns this folder's panes: {error:?}"),
        )
    });
    let life = Arc::new(Life::default());
    {
        let owned = Arc::clone(&owned);
        let life = Arc::clone(&life);
        let told = Arc::clone(&terminals);
        hub.serve(
            &terminals,
            // The host's window ends a pane the guest's engine owns: the
            // engine hears it ended, as when its program exits, and closes
            // it and its tab. A guest not linked now hears it from the
            // pane's record when it links again (`HolderRecovery::closed`).
            Box::new(move |pane| {
                match owned.terminate(pane) {
                    Ok(()) => told.tell_ended(pane),
                    Err(error) => record(
                        "guest_node.terminate_failed",
                        json!({"pane": pane, "error": format!("{error:?}")}),
                    ),
                }
                life.look();
            }),
        );
    }

    say(&Ready {
        guest_node: launch::GUEST_NODE,
        build: crate::build_order::Release::of_this_build().version,
        build_id: build_id(
            &program,
            install_root.is_some() || helper.as_ref().is_some_and(|helper| helper.placed),
        ),
        pid: std::process::id(),
        link: launch::LINK_ENDPOINT.to_owned(),
        tap: launch::TAP_ENDPOINT.to_owned(),
        tap_versions: [tap_contract::OLDEST_TAP, tap_contract::NEWEST_TAP],
        capabilities: events
            .iter()
            .flat_map(|_| launch::NODE_CAPABILITIES)
            .map(str::to_owned)
            .collect(),
    });

    {
        let life = Arc::clone(&life);
        let owned = Arc::clone(&owned);
        let events = events.clone();
        let hub = Arc::clone(&hub);
        std::thread::spawn(move || read_host(&life, &owned, &hub, events.as_deref()));
    }
    {
        let slot = slot.clone();
        std::thread::spawn(move || away::serve(&away, &slot, events.as_ref()));
    }
    {
        let hub = Arc::clone(&hub);
        std::thread::spawn(move || hub.accept(&taps));
    }
    {
        // The first look is now: the host hears at once whether the node
        // found a pane still running.
        life.look();
        let life = Arc::clone(&life);
        let terminals = Arc::clone(&terminals);
        let hub = Arc::clone(&hub);
        std::thread::spawn(move || watch_life(&life, &terminals, &hub));
    }
    let (sessions, next) = mpsc::sync_channel::<LocalStream>(0);
    {
        let life = Arc::clone(&life);
        std::thread::spawn(move || accept_links(&link, &life, &sessions));
    }

    let worktree_facts = match &helper {
        // Where the helper kept them.
        Some(helper) => hide_kit::layout::worktree_facts(
            &hide_kit::layout::state_dir_from_process(&helper.home),
        ),
        None => hide_kit::layout::worktree_facts(&dir),
    };
    // A node that runs from its guest's install root keeps that
    // installation's hook entries on this account, which its core asks for
    // as it asks any device's kit (D-77 (4)); a copy anywhere else, a
    // development build among them, installs nothing. On a helper's folder
    // the installation is the helper's, kept as the helper kept it.
    let kit = match (
        &helper,
        &install_root,
        program.parent().and_then(Path::file_name),
    ) {
        // The host's own Hide keeps its installation on this account; this
        // node keeps the entries of the build it runs, as a guest's does.
        (Some(helper), _, Some(version)) if helper.placed && helper.panes.is_some() => {
            match &helper.root {
                Some(root) => KitPlace::Guest {
                    root: root.clone(),
                    version: version.to_string_lossy().into_owned(),
                },
                None => KitPlace::Standalone,
            }
        }
        (Some(helper), _, _) if helper.placed => KitPlace::Installed,
        (None, Some(root), Some(version)) => KitPlace::Guest {
            root: root.clone(),
            version: version.to_string_lossy().into_owned(),
        },
        _ => KitPlace::Standalone,
    };
    for stream in next {
        let session = life.begin(stream.shutdown_handle());
        record("guest_node.link_started", json!({"session": session}));
        terminals.relink();
        let reader = BufReader::new(stream.duplicate());
        let served = hide_host::serve::serve_with(
            reader,
            stream,
            Services {
                terminals: Some(&*terminals),
                heartbeat: false,
                // A call the host handed to this guest's `hide` is in no
                // pane: the node proves it by its folder, and the guest's
                // core binds it to its checkout or refuses, finally (D-55).
                checkout_callers: true,
                opened_roots: None,
                browser: None,
                worktree_facts: Some(worktree_facts.clone()),
                kit: kit.clone(),
                panes: Some(slot.clone()),
            },
        );
        life.end();
        record(
            "guest_node.link_ended",
            json!({
                "session": session,
                "error": served.err().map(|error| error.to_string()),
            }),
        );
    }
    exit(
        launch::EXIT_HOST_UNREACHABLE,
        "guest_node.link_closed",
        "the link endpoint closed".to_owned(),
    )
}

/// The build's name in its ready line: its folder's when it runs from a
/// guest's install root, which the host named by the digest of its files,
/// and this program's own hash otherwise.
fn build_id(program: &Path, installed: bool) -> String {
    let folder = program
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .filter(|_| installed);
    match folder {
        Some(folder) => folder.to_owned(),
        None => crate::build_id::of_current_exe().unwrap_or_default(),
    }
}

#[derive(Default)]
struct LifeState {
    /// The guest's link session being served.
    session: Option<Arc<ShutdownHandle>>,
    sessions_started: u64,
    /// No new session, and the node exits with its last pane.
    after_last_pane: bool,
    /// Something changed that the live watch has not looked at.
    look: bool,
}

/// What the node's threads share of its life: the link session, how it was
/// told to end, and the nudge that makes the live watch look now.
#[derive(Default)]
struct Life {
    state: Mutex<LifeState>,
    changed: Condvar,
}

impl Life {
    fn look(&self) {
        lock(&self.state).look = true;
        self.changed.notify_all();
    }

    fn begin(&self, session: ShutdownHandle) -> u64 {
        let mut state = lock(&self.state);
        state.session = Some(Arc::new(session));
        state.sessions_started += 1;
        state.look = true;
        let started = state.sessions_started;
        drop(state);
        self.changed.notify_all();
        started
    }

    fn end(&self) {
        let mut state = lock(&self.state);
        state.session = None;
        state.look = true;
        drop(state);
        self.changed.notify_all();
    }

    /// Whether a session is served and whether the node ends with its last
    /// pane, once something changed or [`LIVE_CHECK`] passed.
    fn wait(&self) -> (bool, bool) {
        let state = lock(&self.state);
        let (mut state, _) = self
            .changed
            .wait_timeout_while(state, LIVE_CHECK, |state| !state.look)
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.look = false;
        (state.session.is_some(), state.after_last_pane)
    }
}

/// Takes each connection to the link endpoint. The guest has one
/// connection, so a new one means the earlier is gone whatever its stream
/// still says: that session is ended and the new one served.
fn accept_links(link: &LocalListener, life: &Life, sessions: &mpsc::SyncSender<LocalStream>) {
    while let Ok(stream) = link.accept() {
        let earlier = {
            let state = lock(&life.state);
            if state.after_last_pane {
                record("guest_node.link_refused", json!({"reason": "ending"}));
                continue;
            }
            state.session.clone()
        };
        if let Some(earlier) = earlier {
            earlier.shutdown();
        }
        if sessions.send(stream).is_err() {
            return;
        }
    }
}

/// Reads what the host says on the node's input until that input ends,
/// which is the host going away: the node exits and leaves its holders.
fn read_host(
    life: &Life,
    owned: &Arc<dyn PaneHolders>,
    tap: &tap::TapHub,
    events: Option<&event_box::EventBox>,
) {
    for line in io::stdin().lock().lines() {
        let Ok(line) = line else { break };
        match ToNode::parse(&line) {
            Some(ToNode::End(EndMode::Now)) => end_now(owned, tap),
            Some(ToNode::End(EndMode::AfterLastPane)) => {
                record("guest_node.end_after_last_pane", json!({}));
                lock(&life.state).after_last_pane = true;
                life.look();
            }
            // The box's lines: a node that named no box is sent none.
            Some(ToNode::Resume(position)) => {
                box_did("resume", events.map(|events| events.resume(&position)));
            }
            Some(ToNode::Ack(position)) => {
                box_did("ack", events.map(|events| events.ack(&position)));
            }
            Some(ToNode::Away) => {
                if let Some(events) = events {
                    events.away();
                }
            }
            // A line of a later host.
            None => record("guest_node.host_line_unread", json!({"bytes": line.len()})),
        }
    }
    record("guest_node.input_ended", json!({}));
    std::process::exit(launch::EXIT_ENDED);
}

/// Records a box step that failed. The guest's core asks again at its next
/// connection, and an event it did not get is still kept.
fn box_did(step: &str, done: Option<io::Result<()>>) {
    if let Some(Err(error)) = done {
        record(
            "guest_node.box_step_failed",
            json!({"step": step, "error": error.to_string()}),
        );
    }
}

/// Ends every pane through the node's own ownership, then exits. A pane
/// that has not ended within the contract's wait is left to its holder,
/// and stays in the host's list: the host hears of every pane that ended
/// before the node goes.
fn end_now(owned: &Arc<dyn PaneHolders>, tap: &tap::TapHub) -> ! {
    let present = owned
        .recovered(SCAN_WITHIN)
        .map(|found| found.present)
        .unwrap_or_default();
    record("guest_node.end_now", json!({"panes": present.len()}));
    let deadline = Instant::now() + Duration::from_secs(launch::END_NOW_WAITS_SECS);
    let (ended, wait) = mpsc::channel();
    let mut left = present.len();
    let mut running: Vec<String> = present.clone();
    for pane in present {
        let ended = ended.clone();
        let owned = Arc::clone(owned);
        // Not joined: a close still under way at the deadline is its
        // holder's to finish.
        std::thread::spawn(move || {
            let closed = owned.terminate(&pane);
            let _ = ended.send((pane, closed));
        });
    }
    drop(ended);
    while left > 0 {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        match wait.recv_timeout(remaining) {
            Ok((pane, Err(error))) => {
                left -= 1;
                record(
                    "guest_node.terminate_failed",
                    json!({"pane": pane, "error": format!("{error:?}")}),
                );
            }
            Ok((pane, Ok(()))) => {
                left -= 1;
                running.retain(|live| *live != pane);
            }
            Err(_) => break,
        }
    }
    if left > 0 {
        record("guest_node.end_now_unfinished", json!({"panes": left}));
    }
    tap.live(&running);
    tap.flushed(TAP_FLUSH_WITHIN);
    std::process::exit(launch::EXIT_ENDED)
}

/// Tells the host when the node becomes idle or stops being idle, and ends
/// a node told to end with its last pane once that pane is gone.
fn watch_life(life: &Life, terminals: &NodeTerminals, tap: &tap::TapHub) {
    let mut told: Option<bool> = None;
    loop {
        let (linked, after_last_pane) = life.wait();
        // A count that cannot be read is not "no pane": the node is not
        // idle, and does not end, on what it could not see.
        let live = match terminals.live_panes() {
            Ok(live) => live,
            Err(error) => {
                record("guest_node.live_unread", json!({"error": error}));
                continue;
            }
        };
        // The host's list of this guest's panes follows the holders'
        // records, whatever the guest's core holds of them.
        tap.live(&live);
        if live.is_empty() && after_last_pane {
            record("guest_node.last_pane_ended", json!({}));
            std::process::exit(launch::EXIT_ENDED);
        }
        let idle = live.is_empty() && !linked;
        if told != Some(idle) {
            say(&FromNode::Idle(idle));
            told = Some(idle);
        }
    }
}
