//! The host's end of a guest's tap (PRD device-connect-iroh D-77 (5), B10,
//! B16): how this machine's screens draw a guest's panes and type into
//! them, whether or not the guest is connected.
//!
//! One connection to the `tap.sock` the guest's node serves. What the node
//! writes there is all this build reads of a guest's panes: which exist,
//! each one's grid, and its output. The pane list is kept in the guest's
//! `h/panes.json`, written whole on every change, so the panes show after
//! this daemon restarts with the guest away. Output goes to the screens'
//! hub under each pane's guest address
//! ([`hide_node_link::address::guest_pane_id`]); a screen's keys, sizes and
//! redraws come back through the terminal router to this tap.
//!
//! The node is the guest's build, so what it says is bounded here: at most
//! [`MAX_PANES`] panes, each under an id a list carries; a pane past that
//! is not listed and its output goes nowhere.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hide_core::guest_panes::{MAX_PANES, pane_is_listed};
use hide_node::terminal::OutputSink;
use hide_node_link::address::guest_pane_id;
use hide_node_link::terminal::{GridSize, KeyTarget, TerminalControl, TerminalNode};
use hide_peer::protocol::panes::{self, Pane, Panes};
use hide_peer::protocol::tap::{self, PaneSize, ToHost, ToNode};
use hide_platform::fs::{Access, atomic, private};
use hide_platform::ipc::{LocalStream, ShutdownHandle};
use serde_json::json;

/// How long a node that said it is ready is given to take its tap.
const TAP_WITHIN: Duration = Duration::from_secs(5);

/// How long one frame to the node may wait on a node that reads nothing.
const WRITE_WITHIN: Duration = Duration::from_secs(5);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

#[derive(Default)]
struct State {
    /// The guest's panes as the node last said, by the node's own ids.
    panes: BTreeMap<String, Pane>,
    /// The connection to the node's tap: what is written to the node, and
    /// what ends its reader.
    writer: Option<LocalStream>,
    reader: Option<ShutdownHandle>,
    /// The connection the reader of that number serves; a later attach
    /// takes its place.
    connection: u64,
}

impl State {
    /// Lists `pane` at the grid the node said, keeping when it was first
    /// seen. False for a pane the list has no room or no name for.
    fn list(&mut self, pane: PaneSize, now: u64) -> bool {
        if let Some(known) = self.panes.get_mut(&pane.pane) {
            known.rows = pane.rows;
            known.cols = pane.cols;
            return true;
        }
        if self.panes.len() >= MAX_PANES || !pane_is_listed(&pane.pane) {
            return false;
        }
        self.panes.insert(
            pane.pane.clone(),
            Pane {
                pane: pane.pane,
                rows: pane.rows,
                cols: pane.cols,
                since_unix_ms: now,
            },
        );
        true
    }
}

pub struct GuestTap {
    /// The guest's device id, which its panes' addresses carry.
    device: String,
    /// The guest's short key, for the log.
    guest: String,
    file: PathBuf,
    hub: Arc<dyn OutputSink>,
    /// Told after a pane joined or left the guest's list.
    changed: Box<dyn Fn() + Send + Sync>,
    state: Mutex<State>,
}

impl GuestTap {
    /// The tap of the guest whose host-side folder holds `file`, with the
    /// panes that file lists: the ones this machine last saw, which may
    /// still run in their holders.
    pub fn open(
        device: &str,
        guest: &str,
        file: PathBuf,
        hub: Arc<dyn OutputSink>,
        changed: Box<dyn Fn() + Send + Sync>,
    ) -> Arc<Self> {
        let listed = match std::fs::read(&file) {
            Ok(bytes) => match serde_json::from_slice::<Panes>(&bytes) {
                Ok(listed) => listed.panes,
                Err(error) => {
                    hide_core::diagnostic!(json!({
                        "component": "guest_tap",
                        "kind": "guest_tap.panes_unread",
                        "guest": guest,
                        "reason": error.to_string(),
                    }));
                    Vec::new()
                }
            },
            Err(_) => Vec::new(),
        };
        Arc::new(Self {
            device: device.to_owned(),
            guest: guest.to_owned(),
            file,
            hub,
            changed,
            state: Mutex::new(State {
                panes: listed
                    .into_iter()
                    .filter(|pane| pane_is_listed(&pane.pane))
                    .take(MAX_PANES)
                    .map(|pane| (pane.pane.clone(), pane))
                    .collect(),
                ..State::default()
            }),
        })
    }

    pub fn device(&self) -> &str {
        &self.device
    }

    /// The guest's panes on this machine, as last seen.
    pub fn panes(&self) -> Vec<Pane> {
        lock(&self.state).panes.values().cloned().collect()
    }

    fn record(&self, kind: &str, extra: serde_json::Value) {
        let mut record = json!({
            "component": "guest_tap",
            "kind": kind,
            "guest": self.guest,
        });
        if let (Some(record), Some(extra)) = (record.as_object_mut(), extra.as_object()) {
            for (key, value) in extra {
                record.insert(key.clone(), value.clone());
            }
        }
        hide_core::diagnostic!(record);
    }

    /// Connects to the tap the guest's node serves at `endpoint`, in place
    /// of an earlier connection, and reads it on a thread of its own until
    /// it ends. Called when the node says it is ready.
    pub fn attach(self: &Arc<Self>, endpoint: &Path) {
        let endpoint = endpoint.to_path_buf();
        let tap = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name(format!("guest-tap-{}", self.guest))
            .spawn(move || tap.read(&endpoint));
        if let Err(error) = spawned {
            self.record(
                "guest_tap.not_started",
                json!({"reason": error.to_string()}),
            );
        }
    }

    /// Ends the connection to the node's tap. The pane list stays: the
    /// panes run in their holders whether or not their node does.
    pub fn detach(&self) {
        let mut state = lock(&self.state);
        state.connection += 1;
        state.writer = None;
        if let Some(reader) = state.reader.take() {
            reader.shutdown();
        }
    }

    fn read(&self, endpoint: &Path) {
        let mut stream = match super::connect_to_node(endpoint, TAP_WITHIN) {
            Ok(stream) => stream,
            Err(error) => {
                self.record(
                    "guest_tap.unreachable",
                    json!({"reason": error.to_string()}),
                );
                return;
            }
        };
        let writer = stream.duplicate();
        let _ = writer.set_write_timeout(Some(WRITE_WITHIN));
        let connection = {
            let mut state = lock(&self.state);
            state.connection += 1;
            if let Some(earlier) = state.reader.replace(stream.shutdown_handle()) {
                earlier.shutdown();
            }
            state.writer = Some(writer);
            state.connection
        };
        self.send(&ToNode::Hello {
            oldest: tap::OLDEST_TAP,
            newest: tap::NEWEST_TAP,
        });
        self.record("guest_tap.attached", json!({}));
        let ended = loop {
            match read_frame(&mut stream) {
                Ok(Some(frame)) => self.take(frame),
                Ok(None) => {}
                Err(reason) => break reason,
            }
        };
        let mut state = lock(&self.state);
        if state.connection == connection {
            state.writer = None;
            state.reader = None;
        }
        drop(state);
        self.record("guest_tap.ended", json!({"reason": ended}));
    }

    fn take(&self, frame: ToHost) {
        match frame {
            ToHost::Welcome { panes, .. } => {
                let mut state = lock(&self.state);
                let earlier = std::mem::take(&mut state.panes);
                let now = now_unix_ms();
                let mut unlisted = 0usize;
                for pane in panes {
                    let since = earlier
                        .get(&pane.pane)
                        .map_or(now, |known| known.since_unix_ms);
                    if !state.list(pane, since) {
                        unlisted += 1;
                    }
                }
                let gone: Vec<String> = earlier
                    .keys()
                    .filter(|pane| !state.panes.contains_key(*pane))
                    .cloned()
                    .collect();
                let joined = state.panes.keys().any(|pane| !earlier.contains_key(pane));
                self.save(&state);
                drop(state);
                if unlisted > 0 {
                    self.record("guest_tap.panes_unlisted", json!({"panes": unlisted}));
                }
                for pane in &gone {
                    self.hub.forget(&guest_pane_id(&self.device, pane));
                }
                if joined || !gone.is_empty() {
                    (self.changed)();
                }
            }
            ToHost::Pane(pane) => {
                let mut state = lock(&self.state);
                let known = state.panes.contains_key(&pane.pane);
                if !state.list(pane, now_unix_ms()) {
                    drop(state);
                    self.record("guest_tap.panes_unlisted", json!({"panes": 1}));
                    return;
                }
                self.save(&state);
                drop(state);
                if !known {
                    (self.changed)();
                }
            }
            ToHost::Gone { pane } => {
                let mut state = lock(&self.state);
                if state.panes.remove(&pane).is_none() {
                    return;
                }
                self.save(&state);
                drop(state);
                self.hub.forget(&guest_pane_id(&self.device, &pane));
                (self.changed)();
            }
            ToHost::Output {
                pane,
                full,
                rows,
                cols,
                bytes,
            } => {
                {
                    let mut state = lock(&self.state);
                    // Output of a pane the list does not hold is no one's
                    // to draw: the node names a pane before it draws it.
                    let Some(known) = state.panes.get_mut(&pane) else {
                        return;
                    };
                    // A size of zero is a pane not drawn yet, never a grid.
                    if rows > 0 && cols > 0 && (known.rows, known.cols) != (rows, cols) {
                        known.rows = rows;
                        known.cols = cols;
                        self.save(&state);
                    }
                }
                self.hub
                    .output(&guest_pane_id(&self.device, &pane), &bytes, full);
            }
        }
    }

    /// Writes the pane list whole. A list that could not be written is
    /// still the list in memory; the next change writes it again.
    fn save(&self, state: &State) {
        let listed = Panes {
            version: panes::VERSION,
            panes: state.panes.values().cloned().collect(),
        };
        let written = self
            .file
            .parent()
            .map_or(Ok(()), private::create_dir_all)
            .and_then(|()| {
                serde_json::to_vec(&listed)
                    .map_err(std::io::Error::other)
                    .and_then(|bytes| atomic::write_file(&self.file, &bytes, Access::Private))
                    .map(drop)
            });
        if let Err(error) = written {
            self.record(
                "guest_tap.panes_unsaved",
                json!({"reason": error.to_string()}),
            );
        }
    }

    /// Writes the node one frame; a node whose tap is not connected hears
    /// nothing, and one that takes no bytes in time loses the connection.
    fn send(&self, frame: &ToNode) {
        let Ok(bytes) = frame.encode() else { return };
        let mut state = lock(&self.state);
        let written = state.writer.as_mut().is_some_and(|writer| {
            writer
                .write_all(&bytes)
                .and_then(|()| writer.flush())
                .is_ok()
        });
        if !written
            && state.writer.take().is_some()
            && let Some(reader) = state.reader.take()
        {
            reader.shutdown();
        }
    }

    /// Ends the guest's pane for this machine's operator (B10): its node
    /// closes it, whether or not the guest is connected.
    pub fn terminate(&self, pane: &str) {
        self.send(&ToNode::Terminate {
            pane: pane.to_owned(),
        });
    }

    fn resize(&self, pane: &str, size: GridSize) {
        self.send(&ToNode::Resize {
            pane: pane.to_owned(),
            rows: size.rows,
            cols: size.cols,
        });
    }
}

/// The next frame the node wrote: `None` for a kind of a later build, and
/// the reason once the connection ended or wrote something no frame is.
fn read_frame(stream: &mut LocalStream) -> Result<Option<ToHost>, String> {
    let mut prefix = [0u8; 4];
    stream
        .read_exact(&mut prefix)
        .map_err(|error| error.to_string())?;
    let length = tap::frame_length(prefix).map_err(|error| error.to_string())?;
    if length == 0 {
        return Err("a frame without a kind".to_owned());
    }
    let mut body = vec![0u8; length];
    stream
        .read_exact(&mut body)
        .map_err(|error| error.to_string())?;
    ToHost::decode(body[0], &body[1..]).map_err(|error| error.to_string())
}

/// A screen of this machine reaches a guest's panes here, by the node's
/// own pane ids: the terminal router takes the guest's address apart.
impl TerminalNode for GuestTap {
    fn control(&self, control: TerminalControl) {
        match control {
            TerminalControl::Attach { pane, size, .. } => {
                if let Some(size) = size {
                    self.resize(&pane, size);
                }
                self.send(&ToNode::Redraw { pane });
            }
            TerminalControl::Resize { pane, size } => self.resize(&pane, size),
            // A screen that stops looking at the pane changes nothing at
            // the node, which keeps the pane's session for the next one.
            _ => {}
        }
    }

    /// Whether the guest's node is there to take input for a pane it
    /// lists: the tap is connected and the pane is in the list.
    fn input_permit(
        &self,
        pane: &str,
        _bytes: usize,
        reply: hide_node_link::terminal::InputPermitReply,
    ) {
        let taken = {
            let state = lock(&self.state);
            state.writer.is_some() && state.panes.contains_key(pane)
        };
        reply(if taken {
            Ok(())
        } else {
            Err(hide_node_link::terminal::InputPermitError::Unavailable)
        });
    }

    fn key(&self, target: KeyTarget, bytes: Vec<u8>, _typed_at_unix_ms: u64) {
        if let KeyTarget::Pane(pane) = target {
            self.send(&ToNode::Input { pane, bytes });
        }
    }

    fn view(&self, pane: &str, size: GridSize, new_view: bool) {
        self.resize(pane, size);
        if new_view {
            self.redraw(pane);
        }
    }

    fn redraw(&self, pane: &str) {
        self.send(&ToNode::Redraw {
            pane: pane.to_owned(),
        });
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::mpsc::{self, Receiver, Sender};

    use hide_platform::ipc::LocalListener;

    use super::*;
    use crate::peer_host::fixture::{WITHIN, short_root};

    #[derive(Debug, PartialEq, Eq)]
    enum Drawn {
        Output(String, Vec<u8>, bool),
        Forgot(String),
    }

    /// The screens' hub, as what reached it in order.
    struct Hub(Mutex<Sender<Drawn>>);

    impl OutputSink for Hub {
        fn output(&self, pane: &str, bytes: &[u8], full: bool) {
            let _ = lock(&self.0).send(Drawn::Output(pane.to_owned(), bytes.to_vec(), full));
        }
        fn forget(&self, pane: &str) {
            let _ = lock(&self.0).send(Drawn::Forgot(pane.to_owned()));
        }
        fn clipboard(&self, _pane: &str, _text: &[u8]) {}
    }

    /// A tap over `file` whose hub and list changes a test reads.
    fn tap(file: &Path) -> (Arc<GuestTap>, Receiver<Drawn>, Receiver<()>) {
        let (drawn, drawn_here) = mpsc::channel();
        let (changed, changed_here) = mpsc::channel();
        let changed = Mutex::new(changed);
        let tap = GuestTap::open(
            "macbook",
            "0123456789abcdef",
            file.to_path_buf(),
            Arc::new(Hub(Mutex::new(drawn))),
            Box::new(move || {
                let _ = lock(&changed).send(());
            }),
        );
        (tap, drawn_here, changed_here)
    }

    /// The next frame the host wrote to the node this test plays.
    pub(crate) fn heard(node: &mut LocalStream) -> ToNode {
        let mut prefix = [0u8; 4];
        node.read_exact(&mut prefix).unwrap();
        let mut body = vec![0u8; tap::frame_length(prefix).unwrap()];
        node.read_exact(&mut body).unwrap();
        ToNode::decode(body[0], &body[1..])
            .unwrap()
            .expect("a frame this build knows")
    }

    fn say(node: &mut LocalStream, frame: &ToHost) {
        node.write_all(&frame.encode().unwrap()).unwrap();
        node.flush().unwrap();
    }

    fn size(pane: &str, rows: u16, cols: u16) -> PaneSize {
        PaneSize {
            pane: pane.to_owned(),
            rows,
            cols,
        }
    }

    fn saved(file: &Path) -> Vec<(String, u16, u16)> {
        let listed: Panes = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert_eq!(listed.version, panes::VERSION);
        listed
            .panes
            .into_iter()
            .map(|pane| (pane.pane, pane.rows, pane.cols))
            .collect()
    }

    #[test]
    fn a_guests_panes_are_listed_drawn_and_typed_into_and_the_list_outlives_the_tap() {
        let root = short_root();
        let file = root.path().join("h").join(panes::FILE);
        let endpoint = root.path().join("tap.sock");
        let listener = LocalListener::bind(&endpoint).unwrap();
        let (tap, drawn, changed) = tap(&file);
        assert!(tap.panes().is_empty());

        tap.attach(&endpoint);
        let mut node = listener.accept().unwrap();
        assert_eq!(
            heard(&mut node),
            ToNode::Hello {
                oldest: tap::OLDEST_TAP,
                newest: tap::NEWEST_TAP,
            }
        );
        say(
            &mut node,
            &ToHost::Welcome {
                version: tap::NEWEST_TAP,
                panes: vec![size("w1:p1", 24, 80)],
            },
        );
        changed.recv_timeout(WITHIN).expect("the list changed");
        assert_eq!(saved(&file), [("w1:p1".to_owned(), 24, 80)]);
        let first_seen = tap.panes()[0].since_unix_ms;

        // Output of a pane the list never held goes nowhere; a listed
        // pane's reaches the hub under the guest's address, and the grid it
        // was drawn at is kept.
        say(
            &mut node,
            &ToHost::Output {
                pane: "w9:p9".to_owned(),
                full: true,
                rows: 24,
                cols: 80,
                bytes: b"stray".to_vec(),
            },
        );
        say(
            &mut node,
            &ToHost::Output {
                pane: "w1:p1".to_owned(),
                full: true,
                rows: 30,
                cols: 100,
                bytes: b"hello".to_vec(),
            },
        );
        assert_eq!(
            drawn.recv_timeout(WITHIN).unwrap(),
            Drawn::Output(
                "guest:macbook:pane:w1:p1".to_owned(),
                b"hello".to_vec(),
                true
            )
        );
        assert_eq!(saved(&file), [("w1:p1".to_owned(), 30, 100)]);
        assert!(
            changed.try_recv().is_err(),
            "a grid is no change of the list"
        );

        // A send from this machine's CLI asks first whether the node is
        // there for the pane.
        let permitted = |pane: &str| {
            hide_node_link::terminal::wait_for_input_permit(tap.as_ref(), pane, 3).is_ok()
        };
        assert!(permitted("w1:p1"));
        assert!(!permitted("w9:p9"), "a pane the guest does not have");

        // What a screen of this machine does reaches the node by the
        // node's own pane id.
        tap.key(KeyTarget::Pane("w1:p1".to_owned()), b"ls\n".to_vec(), 0);
        assert_eq!(
            heard(&mut node),
            ToNode::Input {
                pane: "w1:p1".to_owned(),
                bytes: b"ls\n".to_vec(),
            }
        );
        tap.view(
            "w1:p1",
            GridSize {
                rows: 40,
                cols: 120,
            },
            true,
        );
        assert_eq!(
            heard(&mut node),
            ToNode::Resize {
                pane: "w1:p1".to_owned(),
                rows: 40,
                cols: 120,
            }
        );
        assert_eq!(
            heard(&mut node),
            ToNode::Redraw {
                pane: "w1:p1".to_owned()
            }
        );
        tap.terminate("w1:p1");
        assert_eq!(
            heard(&mut node),
            ToNode::Terminate {
                pane: "w1:p1".to_owned()
            }
        );

        say(&mut node, &ToHost::Pane(size("w1:p2", 0, 0)));
        changed.recv_timeout(WITHIN).expect("a pane joined");
        say(
            &mut node,
            &ToHost::Gone {
                pane: "w1:p1".to_owned(),
            },
        );
        assert_eq!(
            drawn.recv_timeout(WITHIN).unwrap(),
            Drawn::Forgot("guest:macbook:pane:w1:p1".to_owned())
        );
        changed.recv_timeout(WITHIN).expect("a pane left");
        assert_eq!(saved(&file), [("w1:p2".to_owned(), 0, 0)]);

        // The node ends; the list is the file's, for the next start of
        // this daemon.
        tap.detach();
        assert!(
            !hide_node_link::terminal::wait_for_input_permit(tap.as_ref(), "w1:p2", 3).is_ok(),
            "a pane whose node is away takes no input"
        );
        let (again, _drawn, _changed) = self::tap(&file);
        let listed = again.panes();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].pane, "w1:p2");
        assert!(listed[0].since_unix_ms >= first_seen);
    }

    #[test]
    fn a_node_that_names_more_panes_than_a_list_holds_has_the_rest_unlisted() {
        let root = short_root();
        let file = root.path().join("h").join(panes::FILE);
        let endpoint = root.path().join("tap.sock");
        let listener = LocalListener::bind(&endpoint).unwrap();
        let (tap, _drawn, changed) = tap(&file);
        tap.attach(&endpoint);
        let mut node = listener.accept().unwrap();
        heard(&mut node);

        // The frame itself refuses a pane id too long to list; the count
        // is this end's to hold.
        say(
            &mut node,
            &ToHost::Welcome {
                version: tap::NEWEST_TAP,
                panes: (0..=MAX_PANES)
                    .map(|index| size(&format!("w1:p{index}"), 24, 80))
                    .collect(),
            },
        );
        changed.recv_timeout(WITHIN).expect("the list changed");
        assert_eq!(tap.panes().len(), MAX_PANES);
        say(&mut node, &ToHost::Pane(size("w2:p1", 24, 80)));
        say(
            &mut node,
            &ToHost::Gone {
                pane: "w1:p0".to_owned(),
            },
        );
        changed.recv_timeout(WITHIN).expect("a pane left");
        let listed = saved(&file);
        assert_eq!(listed.len(), MAX_PANES - 1);
        assert!(
            listed.iter().all(|(pane, ..)| pane != "w2:p1"),
            "a pane named while the list was full is not listed"
        );
    }
}
