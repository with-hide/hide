//! The node's end of the tap (PRD device-connect-iroh D-77 (5)): how the
//! host's window draws a guest's panes and types into them, over
//! `tap.sock`, whether or not the guest is connected.
//!
//! One host connection at a time; a new one replaces the old and is sent
//! every pane whole. The tap is lossy: at most
//! [`tap::CAP_BYTES`](hide_peer::protocol::tap::CAP_BYTES) of output waits
//! for a slow host, and when a partial update would pass that, the waiting
//! output goes and each pane in it is drawn again from a full frame.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};

use hide_node::terminal::OutputSink;
use hide_node::terminal::device::NodeTerminals;
use hide_node_link::terminal::{GridSize, KeyTarget, TerminalNode};
use hide_peer::protocol::tap::{self, PaneSize, ToHost, ToNode};
use hide_platform::ipc::{LocalListener, LocalStream, ShutdownHandle};
use serde_json::json;

use super::{lock, now_unix_ms, record};

/// Ends a pane for the host's window, through the node's own ownership.
pub(super) type Terminate = Box<dyn Fn(&str) + Send + Sync>;

#[derive(Default)]
struct TapState {
    /// The panes whose output passed here, each with the grid it is drawn
    /// at; `(0, 0)` until the service says.
    panes: BTreeMap<String, (u16, u16)>,
    /// The connected host's number; 0 is none.
    client: u64,
    clients: u64,
    /// Ends the connected host's stream, so its reader and writer end.
    stream: Option<ShutdownHandle>,
    /// Frames waiting, each with its pane when it is output.
    queue: VecDeque<(Option<String>, Vec<u8>)>,
    /// Bytes of waiting output frames.
    bytes: usize,
    /// Panes whose picture this client has from a full frame.
    started: HashSet<String>,
    /// Panes asked to draw again whole and not yet drawn.
    asked: HashSet<String>,
    dropped: u64,
}

impl TapState {
    /// Forgets the connected host and ends its stream.
    fn clear_client(&mut self) {
        self.client = 0;
        self.queue.clear();
        self.bytes = 0;
        self.started.clear();
        self.asked.clear();
        if let Some(stream) = self.stream.take() {
            stream.shutdown();
        }
    }

    fn fact(&mut self, frame: ToHost) {
        if self.client == 0 {
            return;
        }
        // A pane id a frame cannot carry has no frame; the host's list is
        // then without it, which the node's own records still hold.
        if let Ok(bytes) = frame.encode() {
            self.queue.push_back((None, bytes));
        }
    }
}

#[derive(Default)]
pub(super) struct TapHub {
    state: Mutex<TapState>,
    ready: Condvar,
    node: OnceLock<Weak<NodeTerminals>>,
    terminate: OnceLock<Terminate>,
}

impl TapHub {
    /// The node whose panes the tap types into and asks to draw again,
    /// given once, after the node is made over this hub.
    pub(super) fn serve(&self, node: &Arc<NodeTerminals>, terminate: Terminate) {
        let _ = self.node.set(Arc::downgrade(node));
        let _ = self.terminate.set(terminate);
    }

    fn node(&self) -> Option<Arc<NodeTerminals>> {
        self.node.get().and_then(Weak::upgrade)
    }

    fn redraw(&self, panes: Vec<String>) {
        if panes.is_empty() {
            return;
        }
        if let Some(node) = self.node() {
            for pane in panes {
                node.redraw(&pane);
            }
        }
    }

    /// Takes each host connection in turn until the listener closes.
    pub(super) fn accept(self: &Arc<Self>, taps: &LocalListener) {
        while let Ok(stream) = taps.accept() {
            let hub = Arc::clone(self);
            std::thread::spawn(move || hub.attach(stream));
        }
    }

    /// Reads the host's hello, then replaces the connected host with this
    /// one and sends it every pane whole.
    fn attach(self: Arc<Self>, mut stream: LocalStream) {
        let version = match read_frame(&mut stream) {
            Some(ToNode::Hello { oldest, newest })
                if oldest <= tap::NEWEST_TAP && newest >= tap::OLDEST_TAP =>
            {
                newest.min(tap::NEWEST_TAP)
            }
            other => {
                record(
                    "guest_node.tap_refused",
                    json!({"hello": matches!(other, Some(ToNode::Hello { .. }))}),
                );
                return;
            }
        };
        let (client, redraws) = {
            let mut state = lock(&self.state);
            state.clients += 1;
            let client = state.clients;
            state.clear_client();
            state.client = client;
            state.stream = Some(stream.shutdown_handle());
            state.asked = state.panes.keys().cloned().collect();
            let panes = state
                .panes
                .iter()
                .map(|(pane, (rows, cols))| PaneSize {
                    pane: pane.clone(),
                    rows: *rows,
                    cols: *cols,
                })
                .collect();
            state.fact(ToHost::Welcome { version, panes });
            (client, state.panes.keys().cloned().collect::<Vec<_>>())
        };
        record("guest_node.tap_attached", json!({"client": client}));
        self.ready.notify_all();
        let writer = stream.duplicate();
        {
            let hub = Arc::clone(&self);
            std::thread::spawn(move || hub.write(client, writer));
        }
        self.redraw(redraws);
        self.read(client, stream);
    }

    fn write(&self, client: u64, mut stream: LocalStream) {
        loop {
            let next = {
                let mut state = lock(&self.state);
                loop {
                    if state.client != client {
                        return;
                    }
                    if let Some((pane, bytes)) = state.queue.pop_front() {
                        if pane.is_some() {
                            state.bytes -= bytes.len();
                        }
                        break bytes;
                    }
                    state = self
                        .ready
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            };
            if stream
                .write_all(&next)
                .and_then(|()| stream.flush())
                .is_err()
            {
                self.detach(client);
                return;
            }
        }
    }

    fn read(&self, client: u64, mut stream: LocalStream) {
        while let Some(frame) = read_frame(&mut stream) {
            if lock(&self.state).client != client {
                return;
            }
            let Some(node) = self.node() else { break };
            match frame {
                ToNode::Hello { .. } => {}
                ToNode::Input { pane, bytes } => {
                    node.key(KeyTarget::Pane(pane), bytes, now_unix_ms());
                }
                ToNode::Resize { pane, rows, cols } => {
                    node.view(&pane, GridSize { rows, cols }, false);
                }
                ToNode::Terminate { pane } => {
                    if let Some(terminate) = self.terminate.get() {
                        terminate(&pane);
                    }
                }
                ToNode::Redraw { pane } => node.redraw(&pane),
            }
        }
        self.detach(client);
    }

    fn detach(&self, client: u64) {
        let mut state = lock(&self.state);
        if state.client == client {
            state.clear_client();
            record("guest_node.tap_ended", json!({"client": client}));
        }
        drop(state);
        self.ready.notify_all();
    }
}

/// The next frame the host wrote that this build knows; `None` once the
/// connection ended or wrote something no frame is.
fn read_frame(stream: &mut LocalStream) -> Option<ToNode> {
    loop {
        let mut prefix = [0u8; 4];
        stream.read_exact(&mut prefix).ok()?;
        let length = tap::frame_length(prefix).ok()?;
        let mut body = vec![0u8; length];
        stream.read_exact(&mut body).ok()?;
        match ToNode::decode(body[0], &body[1..]) {
            Ok(Some(frame)) => return Some(frame),
            // A kind of a later build.
            Ok(None) => {}
            Err(_) => return None,
        }
    }
}

impl OutputSink for TapHub {
    fn output(&self, pane: &str, bytes: &[u8], full: bool) {
        let mut redraws = Vec::new();
        {
            let mut state = lock(&self.state);
            let (rows, cols) = match state.panes.get(pane) {
                Some(size) => *size,
                None => {
                    state.panes.insert(pane.to_owned(), (0, 0));
                    state.fact(ToHost::Pane(PaneSize {
                        pane: pane.to_owned(),
                        rows: 0,
                        cols: 0,
                    }));
                    (0, 0)
                }
            };
            if state.client == 0 {
                return;
            }
            if !full && !state.started.contains(pane) {
                // A partial update of a picture this host never had.
                if state.asked.insert(pane.to_owned()) {
                    redraws.push(pane.to_owned());
                }
            } else if let Ok(framed) = (ToHost::Output {
                pane: pane.to_owned(),
                full,
                rows,
                cols,
                bytes: bytes.to_vec(),
            })
            .encode()
            {
                let mut take = true;
                if full {
                    if framed.len() > tap::CAP_BYTES {
                        state.started.remove(pane);
                        take = false;
                    } else {
                        // A full frame replaces what of its pane still waits.
                        let mut freed = 0;
                        state.queue.retain(|(waiting, bytes)| {
                            let replaced = waiting.as_deref() == Some(pane);
                            if replaced {
                                freed += bytes.len();
                            }
                            !replaced
                        });
                        state.bytes -= freed;
                        state.started.insert(pane.to_owned());
                        state.asked.remove(pane);
                    }
                }
                if take && state.bytes + framed.len() > tap::CAP_BYTES {
                    // The host fell behind: its backlog goes, and each pane
                    // in it is drawn again from a full frame.
                    let mut behind: HashSet<String> = state
                        .queue
                        .iter()
                        .filter_map(|(waiting, _)| waiting.clone())
                        .collect();
                    behind.insert(pane.to_owned());
                    state.queue.retain(|(waiting, _)| waiting.is_none());
                    state.bytes = 0;
                    state.dropped += 1;
                    for waiting in behind {
                        state.started.remove(&waiting);
                        if state.asked.insert(waiting.clone()) {
                            redraws.push(waiting);
                        }
                    }
                    take = full;
                    if take {
                        state.started.insert(pane.to_owned());
                        state.asked.remove(pane);
                        redraws.retain(|waiting| waiting != pane);
                    }
                    record(
                        "guest_node.tap_dropped",
                        json!({"dropped": state.dropped, "cap": tap::CAP_BYTES}),
                    );
                }
                if take {
                    state.bytes += framed.len();
                    state.queue.push_back((Some(pane.to_owned()), framed));
                }
            }
        }
        self.ready.notify_all();
        self.redraw(redraws);
    }

    fn forget(&self, pane: &str) {
        {
            let mut state = lock(&self.state);
            if state.panes.remove(pane).is_none() {
                return;
            }
            state.started.remove(pane);
            state.asked.remove(pane);
            let mut freed = 0;
            state.queue.retain(|(waiting, bytes)| {
                let gone = waiting.as_deref() == Some(pane);
                if gone {
                    freed += bytes.len();
                }
                !gone
            });
            state.bytes -= freed;
            state.fact(ToHost::Gone {
                pane: pane.to_owned(),
            });
        }
        self.ready.notify_all();
    }

    /// A program's clipboard write is the screen's that looks at the pane
    /// from the guest's own machine; the tap carries none.
    fn clipboard(&self, _pane: &str, _text: &[u8]) {}

    fn grid(&self, pane: &str, rows: u16, cols: u16) {
        {
            let mut state = lock(&self.state);
            let known = state.panes.insert(pane.to_owned(), (rows, cols));
            if known.is_none() {
                state.fact(ToHost::Pane(PaneSize {
                    pane: pane.to_owned(),
                    rows,
                    cols,
                }));
            }
        }
        self.ready.notify_all();
    }
}
