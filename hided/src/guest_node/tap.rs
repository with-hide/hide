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
use std::time::Duration;

use hide_node::terminal::OutputSink;
use hide_node::terminal::device::NodeTerminals;
use hide_node_link::terminal::{GridSize, KeyTarget, TerminalControl, TerminalNode};
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
    /// The folder each live pane was started in, read once from its
    /// holder's record; `None` for a pane whose record names none.
    folders: BTreeMap<String, Option<String>>,
    /// The connected host's number; 0 is none.
    client: u64,
    clients: u64,
    /// Ends the connected host's stream, so its reader and writer end.
    stream: Option<ShutdownHandle>,
    /// Frames waiting, each with its pane when it is output.
    queue: VecDeque<(Option<String>, Vec<u8>)>,
    /// Bytes of waiting output frames.
    bytes: usize,
    /// A frame left the queue and is being written.
    writing: bool,
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
        self.writing = false;
        self.started.clear();
        self.asked.clear();
        if let Some(stream) = self.stream.take() {
            stream.shutdown();
        }
    }

    /// Drops what waits of `pane`'s output; its next frame for this host
    /// has to be a full one.
    fn unstart(&mut self, pane: &str) {
        self.started.remove(pane);
        self.asked.remove(pane);
        let mut freed = 0;
        self.queue.retain(|(waiting, bytes)| {
            let gone = waiting.as_deref() == Some(pane);
            if gone {
                freed += bytes.len();
            }
            !gone
        });
        self.bytes -= freed;
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

/// Reads the folder a pane was started in, in the spelling paths cross
/// machines in.
type FolderOf = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

#[derive(Default)]
pub(super) struct TapHub {
    state: Mutex<TapState>,
    ready: Condvar,
    node: OnceLock<Weak<NodeTerminals>>,
    terminate: OnceLock<Terminate>,
    /// The node's holders' records, read for a pane's folder (D-55).
    folder_of: OnceLock<FolderOf>,
}

impl TapHub {
    /// The node whose panes the tap types into and asks to draw again,
    /// given once, after the node is made over this hub.
    pub(super) fn serve(&self, node: &Arc<NodeTerminals>, terminate: Terminate) {
        let _ = self.node.set(Arc::downgrade(node));
        let _ = self.terminate.set(terminate);
        let records = Arc::downgrade(node);
        let _ = self.folder_of.set(Box::new(move |pane| {
            records.upgrade().and_then(|node| node.start_folder(pane))
        }));
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

    /// The panes that may still run, as the node's holders' records say:
    /// the list the host is told follows them, a started or adopted pane
    /// joining it and an ended one leaving it, and the node keeps a
    /// session of its own on each so the host draws it whether or not the
    /// guest's core looks at it (D-77 (5)).
    pub(super) fn live(&self, live: &[String]) {
        let started: Vec<String> = {
            let mut state = lock(&self.state);
            let ended: Vec<String> = state
                .panes
                .keys()
                .filter(|pane| !live.contains(pane))
                .cloned()
                .collect();
            for pane in ended {
                state.panes.remove(&pane);
                state.folders.remove(&pane);
                state.unstart(&pane);
                state.fact(ToHost::Gone { pane });
            }
            let started: Vec<String> = live
                .iter()
                .filter(|pane| !state.panes.contains_key(*pane))
                .cloned()
                .collect();
            for pane in &started {
                state.panes.insert(pane.clone(), (0, 0));
                state.fact(ToHost::Pane(PaneSize {
                    pane: pane.clone(),
                    rows: 0,
                    cols: 0,
                }));
            }
            started
        };
        self.tell_folders(live);
        self.ready.notify_all();
        if let Some(node) = self.node() {
            for pane in started {
                node.control(TerminalControl::Attach {
                    pane,
                    size: None,
                    manual: false,
                });
            }
        }
    }

    /// Reads the folder of each live pane whose folder is not read yet and
    /// tells the host (D-55). The records are read under no lock of the
    /// hub's.
    fn tell_folders(&self, live: &[String]) {
        let Some(folder_of) = self.folder_of.get() else {
            return;
        };
        let unread: Vec<String> = {
            let state = lock(&self.state);
            live.iter()
                .filter(|pane| !state.folders.contains_key(*pane))
                .cloned()
                .collect()
        };
        let read: Vec<(String, Option<String>)> = unread
            .into_iter()
            .map(|pane| {
                let folder = folder_of(&pane).filter(|folder| folder.len() <= tap::MAX_FOLDER);
                (pane, folder)
            })
            .collect();
        let mut state = lock(&self.state);
        for (pane, folder) in read {
            // A pane that ended while its record was read has no entry.
            if !state.panes.contains_key(&pane) {
                continue;
            }
            if let Some(folder) = &folder {
                state.fact(ToHost::Folder {
                    pane: pane.clone(),
                    folder: folder.clone(),
                });
            }
            state.folders.insert(pane, folder);
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
            let folders: Vec<ToHost> = state
                .folders
                .iter()
                .filter_map(|(pane, folder)| {
                    Some(ToHost::Folder {
                        pane: pane.clone(),
                        folder: folder.clone()?,
                    })
                })
                .collect();
            for folder in folders {
                state.fact(folder);
            }
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
                    // The frame before this one is with the host.
                    if state.writing {
                        state.writing = false;
                        self.ready.notify_all();
                    }
                    if let Some((pane, bytes)) = state.queue.pop_front() {
                        if pane.is_some() {
                            state.bytes -= bytes.len();
                        }
                        state.writing = true;
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

    /// Waits until the connected host has every frame queued so far, or
    /// `within` passed, or no host is connected: what a node that is about
    /// to exit calls so the host's list is not left with panes that ended.
    pub(super) fn flushed(&self, within: Duration) {
        let state = lock(&self.state);
        drop(
            self.ready
                .wait_timeout_while(state, within, |state| {
                    state.client != 0 && (state.writing || !state.queue.is_empty())
                })
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
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
                // A screen of the host sizes the pane, as a core's resize
                // does: the view alone only names the grid frames must
                // have, and a program left at the guest's grid would have
                // every frame refused as the wrong size (B10, D-74).
                ToNode::Resize { pane, rows, cols } => {
                    let size = GridSize { rows, cols };
                    node.view(&pane, size, false);
                    node.control(TerminalControl::Resize { pane, size });
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

    /// The node's terminal service let the pane's picture go. Whether the
    /// pane still runs is its holder's to say ([`TapHub::live`]): the list
    /// keeps it, and what waited of its output goes.
    fn forget(&self, pane: &str) {
        lock(&self.state).unstart(pane);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer_host::fixture::short_root;

    /// A host on the hub's tap, past its hello.
    fn host(endpoint: &std::path::Path) -> LocalStream {
        let mut host = LocalStream::connect(endpoint).unwrap();
        host.write_all(
            &ToNode::Hello {
                oldest: tap::OLDEST_TAP,
                newest: tap::NEWEST_TAP,
            }
            .encode()
            .unwrap(),
        )
        .unwrap();
        host
    }

    fn told(host: &mut LocalStream) -> ToHost {
        let mut prefix = [0u8; 4];
        host.read_exact(&mut prefix).unwrap();
        let mut body = vec![0u8; tap::frame_length(prefix).unwrap()];
        host.read_exact(&mut body).unwrap();
        ToHost::decode(body[0], &body[1..])
            .unwrap()
            .expect("a frame this build knows")
    }

    fn pane(pane: &str) -> PaneSize {
        PaneSize {
            pane: pane.to_owned(),
            rows: 0,
            cols: 0,
        }
    }

    /// D-55: the host is told the folder each pane was started in, once
    /// when the pane joins the list and again to a host that attaches
    /// later; a pane whose record names none is told without one.
    #[test]
    fn the_host_is_told_the_folder_each_pane_was_started_in() {
        let root = short_root();
        let endpoint = root.path().join("tap.sock");
        let listener = LocalListener::bind(&endpoint).unwrap();
        let hub = Arc::new(TapHub::default());
        let _ = hub.folder_of.set(Box::new(|pane| {
            (pane == "w1:p1").then(|| "/work/shop-api".to_owned())
        }));
        {
            let hub = Arc::clone(&hub);
            std::thread::spawn(move || hub.accept(&listener));
        }
        let mut first = host(&endpoint);
        assert!(matches!(told(&mut first), ToHost::Welcome { .. }));
        hub.live(&["w1:p1".to_owned(), "w1:p2".to_owned()]);
        assert_eq!(told(&mut first), ToHost::Pane(pane("w1:p1")));
        assert_eq!(told(&mut first), ToHost::Pane(pane("w1:p2")));
        let folder = || ToHost::Folder {
            pane: "w1:p1".to_owned(),
            folder: "/work/shop-api".to_owned(),
        };
        assert_eq!(told(&mut first), folder());
        // The same panes again say nothing more; a later host hears the
        // folder after its welcome.
        hub.live(&["w1:p1".to_owned(), "w1:p2".to_owned()]);
        let mut second = host(&endpoint);
        assert!(matches!(told(&mut second), ToHost::Welcome { panes, .. } if panes.len() == 2));
        assert_eq!(told(&mut second), folder());
    }

    /// D-77 (5): which panes the host lists is the holders' records' to
    /// say, not the terminal service's, which lets a pane's picture go
    /// whenever the guest's core releases it.
    #[test]
    fn the_hosts_list_follows_the_panes_that_run_not_the_ones_being_drawn() {
        let root = short_root();
        let endpoint = root.path().join("tap.sock");
        let listener = LocalListener::bind(&endpoint).unwrap();
        let hub = Arc::new(TapHub::default());
        {
            let hub = Arc::clone(&hub);
            std::thread::spawn(move || hub.accept(&listener));
        }
        let mut first = host(&endpoint);
        assert_eq!(
            told(&mut first),
            ToHost::Welcome {
                version: tap::NEWEST_TAP,
                panes: Vec::new(),
            }
        );
        hub.live(&["w1:p1".to_owned(), "w1:p2".to_owned()]);
        assert_eq!(told(&mut first), ToHost::Pane(pane("w1:p1")));
        assert_eq!(told(&mut first), ToHost::Pane(pane("w1:p2")));
        // The service lets one pane's picture go; one pane ends.
        hub.forget("w1:p1");
        hub.live(&["w1:p1".to_owned()]);
        assert_eq!(
            told(&mut first),
            ToHost::Gone {
                pane: "w1:p2".to_owned()
            }
        );
        // A host that connects next is told the pane that still runs.
        let mut next = host(&endpoint);
        assert_eq!(
            told(&mut next),
            ToHost::Welcome {
                version: tap::NEWEST_TAP,
                panes: vec![pane("w1:p1")],
            }
        );
    }
}
