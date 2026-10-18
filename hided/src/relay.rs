//! The core's side of a linked node's screens (PRD core-host-node-remote-core
//! D-05, D-06, D-10, D-20): `/relay`, reached only through the node's own SSH
//! connection and only with the grant the core handed that node's link.
//!
//! A node's screen connects to the node's hided, which opens one relay per
//! screen (`mode=screen`) for the core traffic, and one per node
//! (`mode=terminals`) for the output of every pane that is not the node's
//! own. The node's own panes never come back over it: the node draws them
//! from its own terminals, and this core sees their output only while a
//! screen of its own looks at them.
//!
//! The terminals relay is fed by [`ScreenOutputs`], which hands every
//! pane's output to the hub and to each relay's [`RelayTap`]. A tap holds at
//! most [`TAP_CAP_BYTES`] of output for its node, full frames included:
//! output that would pass it drops the backlog, and a full frame that would
//! pass it drops the waiting output of the panes that waited longest, until
//! it fits. Each pane that lost output is drawn again from a full frame,
//! asked for when the relay next sends, and nothing of that pane goes out
//! before the full frame does, so the node's hub never holds a broken stream
//! (B17).

use std::collections::{HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use hide_node::ssh::RemoteHost;
use hide_node::terminal::OutputSink;
use hide_node_link::terminal::{
    TerminalDown, TerminalLine, TerminalNode, TerminalOutput, TerminalUp, decode_base64,
    device_pane_prefix, encode_base64,
};
use serde_json::json;

use crate::owned_task::AbortOnDrop;
use crate::terminal_hub::TerminalHub;

/// How long a relay that ended waits for its writer to hand back the
/// socket for the close frame.
const WRITER_HANDBACK: std::time::Duration = std::time::Duration::from_millis(100);
/// The most output one node's terminals relay holds unsent, full frames
/// included: a few screens' full frames and their output after them, while
/// a node that reads nothing for longer than that is drawn again rather
/// than queued for.
pub const TAP_CAP_BYTES: usize = 4 * 1024 * 1024;
/// Output items one relay message carries at most.
const MESSAGE_ITEMS: usize = 256;
/// The text after which a relay message takes no more items.
const MESSAGE_BYTES: usize = 1024 * 1024;
/// The longest line a node's terminals relay may send: a paste goes up as
/// one key, as large as a screen of this machine may send one.
const MAX_DOWN_LINE: usize = 16 * 1024 * 1024;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Every pane's output on this machine, as the screens here and the
/// terminals relays of linked nodes read it.
pub struct ScreenOutputs {
    hub: Arc<TerminalHub>,
    /// Which screen each pane's size follows; a pane leaves it when it is
    /// forgotten here.
    pane_sizes: Arc<crate::pane_sizes::PaneSizes>,
    /// The taps, replaced whole on every add and remove, so each chunk of
    /// output takes one reference to the list rather than a copy of it.
    taps: Mutex<Arc<Vec<Arc<RelayTap>>>>,
    /// How many taps there are, read without the lock: with none, output
    /// costs one load beyond the hub.
    tapped: AtomicUsize,
}

impl ScreenOutputs {
    pub fn new(hub: Arc<TerminalHub>) -> Arc<Self> {
        Arc::new(Self {
            hub,
            pane_sizes: Arc::default(),
            taps: Mutex::default(),
            tapped: AtomicUsize::new(0),
        })
    }

    pub fn pane_sizes(&self) -> Arc<crate::pane_sizes::PaneSizes> {
        Arc::clone(&self.pane_sizes)
    }

    /// Adds a node's tap, ending the one its node had: a node keeps one
    /// terminals relay, so a second replaces the first rather than double
    /// what the core sends it.
    fn add(&self, tap: Arc<RelayTap>) {
        let mut taps = lock(&self.taps);
        let mut next: Vec<_> = taps
            .iter()
            .filter(|held| {
                if held.node == tap.node {
                    held.close();
                    held.wake.notify_one();
                    false
                } else {
                    true
                }
            })
            .cloned()
            .collect();
        next.push(tap);
        self.tapped.store(next.len(), Ordering::SeqCst);
        *taps = Arc::new(next);
    }

    fn remove(&self, tap: &Arc<RelayTap>) {
        let mut taps = lock(&self.taps);
        let next: Vec<_> = taps
            .iter()
            .filter(|held| !Arc::ptr_eq(held, tap))
            .cloned()
            .collect();
        self.tapped.store(next.len(), Ordering::SeqCst);
        *taps = Arc::new(next);
    }

    fn taps(&self) -> Option<Arc<Vec<Arc<RelayTap>>>> {
        if self.tapped.load(Ordering::SeqCst) == 0 {
            return None;
        }
        Some(Arc::clone(&lock(&self.taps)))
    }
}

impl OutputSink for ScreenOutputs {
    fn output(&self, pane: &str, bytes: &[u8], full: bool) {
        self.hub.output(pane, bytes, full);
        for tap in self.taps().iter().flat_map(|taps| taps.iter()) {
            tap.output(pane, bytes, full);
        }
    }

    fn forget(&self, pane: &str) {
        self.hub.forget(pane);
        self.pane_sizes.forget(pane);
        for tap in self.taps().iter().flat_map(|taps| taps.iter()) {
            tap.forget(pane);
        }
    }
}

enum TapItem {
    Output(TerminalOutput),
    /// The pane is gone; the node forgets what it kept of it.
    Forget(String),
}

impl TapItem {
    /// What it holds of the tap's byte bound.
    fn size(&self) -> usize {
        match self {
            Self::Output(output) => output.data.len(),
            Self::Forget(pane) => pane.len(),
        }
    }
}

#[derive(Default)]
struct TapState {
    items: VecDeque<TapItem>,
    bytes: usize,
    /// Panes whose next item must be a full frame: the relay just started,
    /// or the pane's backlog was dropped.
    awaiting_full: HashSet<String>,
    /// Every pane awaits a full frame until it has sent one: the relay's
    /// node holds nothing of any pane yet.
    started_whole: HashSet<String>,
    /// Panes whose redraw this tap asked for and the relay has not sent on.
    redraws: Vec<String>,
    dropped: u64,
    closed: bool,
}

/// One linked node's view of this machine's pane output: every pane except
/// the node's own.
pub struct RelayTap {
    node: String,
    own_prefix: String,
    state: Mutex<TapState>,
    wake: tokio::sync::Notify,
}

impl RelayTap {
    fn new(node: &str) -> Arc<Self> {
        Arc::new(Self {
            node: node.to_owned(),
            own_prefix: device_pane_prefix(node),
            state: Mutex::default(),
            wake: tokio::sync::Notify::new(),
        })
    }

    /// Whether the tap keeps a chunk of `pane`, decided before its bytes are
    /// encoded: a closed tap and a pane awaiting its full frame take none,
    /// and a pane the node holds nothing of asks one full frame, once.
    fn takes(&self, pane: &str, full: bool) -> bool {
        let mut state = lock(&self.state);
        if state.closed {
            return false;
        }
        if full {
            return true;
        }
        if !state.started_whole.contains(pane) {
            let asked = state.awaiting_full.insert(pane.to_owned());
            if asked {
                state.redraws.push(pane.to_owned());
            }
            drop(state);
            if asked {
                self.wake.notify_one();
            }
            return false;
        }
        !state.awaiting_full.contains(pane)
    }

    fn output(&self, pane: &str, bytes: &[u8], full: bool) {
        if pane.starts_with(&self.own_prefix) || !self.takes(pane, full) {
            return;
        }
        // Encoded between the tap's locks, which every pane's output takes;
        // what changed in between is decided again below.
        let data = encode_base64(bytes);
        let mut state = lock(&self.state);
        if state.closed {
            return;
        }
        if full && data.len() > TAP_CAP_BYTES {
            // No backlog makes room for it. The pane waits for a full frame
            // that fits and asks none, since its next is likely as long.
            state.awaiting_full.insert(pane.to_owned());
            drop(state);
            herdr_core::diagnostic!(json!({
                "component": "node_relay",
                "kind": "relay.terminals_frame_too_long",
                "node": self.node,
                "bytes": data.len(),
                "cap_bytes": TAP_CAP_BYTES,
            }));
            return;
        }
        if !state.started_whole.contains(pane) {
            if !full {
                // The node holds nothing of the pane: ask a full frame once.
                if state.awaiting_full.insert(pane.to_owned()) {
                    state.redraws.push(pane.to_owned());
                    drop(state);
                    self.wake.notify_one();
                }
                return;
            }
            state.started_whole.insert(pane.to_owned());
        }
        if full {
            state.awaiting_full.remove(pane);
            // A full frame draws over whatever of the pane still waits.
            let mut freed = 0;
            state.items.retain(|item| match item {
                TapItem::Output(output) if output.pane == pane => {
                    freed += output.data.len();
                    false
                }
                _ => true,
            });
            state.bytes -= freed;
        } else if state.awaiting_full.contains(pane) {
            return;
        }
        let output = TerminalOutput {
            pane: pane.to_owned(),
            data,
            full,
        };
        let size = output.data.len();
        if state.bytes + size > TAP_CAP_BYTES && full {
            self.make_room(&mut state, size);
        } else if state.bytes + size > TAP_CAP_BYTES {
            self.overflow(&mut state);
            state.awaiting_full.insert(pane.to_owned());
            if !state.redraws.iter().any(|asked| asked == pane) {
                state.redraws.push(pane.to_owned());
            }
            drop(state);
            self.wake.notify_one();
            return;
        }
        state.bytes += size;
        state.items.push_back(TapItem::Output(output));
        drop(state);
        self.wake.notify_one();
    }

    /// Room for a full frame of `size` bytes, made by the panes whose output
    /// waited longest: each loses what of it waits and is drawn again from a
    /// full frame, asked for when the relay next sends, so a node that reads
    /// nothing is asked no redraw meanwhile. A pane's end is never dropped.
    fn make_room(&self, state: &mut TapState, size: usize) {
        let mut panes = 0;
        while state.bytes + size > TAP_CAP_BYTES {
            let Some(oldest) = state.items.iter().find_map(|item| match item {
                TapItem::Output(output) => Some(output.pane.clone()),
                TapItem::Forget(_) => None,
            }) else {
                break;
            };
            let mut freed = 0;
            state.items.retain(|item| match item {
                TapItem::Output(output) if output.pane == oldest => {
                    freed += output.data.len();
                    false
                }
                _ => true,
            });
            state.bytes -= freed;
            if state.awaiting_full.insert(oldest.clone()) {
                state.redraws.push(oldest);
            }
            panes += 1;
        }
        state.dropped += 1;
        herdr_core::diagnostic!(json!({
            "component": "node_relay",
            "kind": "relay.terminals_overflow",
            "node": self.node,
            "dropped": "oldest_panes",
            "panes": panes,
            "cap_bytes": TAP_CAP_BYTES,
            "overflows": state.dropped,
        }));
    }

    /// The node fell behind: its backlog goes, and each pane in it is drawn
    /// again from a full frame.
    fn overflow(&self, state: &mut TapState) {
        let mut panes = HashSet::new();
        let mut forgotten = VecDeque::new();
        for item in state.items.drain(..) {
            match item {
                TapItem::Output(output) => {
                    panes.insert(output.pane);
                }
                // A pane's end is kept: the node must still forget it.
                forget @ TapItem::Forget(_) => forgotten.push_back(forget),
            }
        }
        state.bytes = forgotten.iter().map(TapItem::size).sum();
        state.items = forgotten;
        state.dropped += 1;
        herdr_core::diagnostic!(json!({
            "component": "node_relay",
            "kind": "relay.terminals_overflow",
            "node": self.node,
            "dropped": "backlog",
            "panes": panes.len(),
            "cap_bytes": TAP_CAP_BYTES,
            "overflows": state.dropped,
        }));
        for pane in panes {
            if state.awaiting_full.insert(pane.clone()) {
                state.redraws.push(pane);
            }
        }
    }

    fn forget(&self, pane: &str) {
        if pane.starts_with(&self.own_prefix) {
            return;
        }
        let mut state = lock(&self.state);
        if state.closed {
            return;
        }
        // Its output goes; a Forget already queued for it stays, so a
        // second forget before the relay sends still forgets it once.
        let mut freed = 0;
        state.items.retain(|item| {
            if matches!(item, TapItem::Output(output) if output.pane == pane) {
                freed += item.size();
                false
            } else {
                true
            }
        });
        state.bytes -= freed;
        state.awaiting_full.remove(pane);
        state.redraws.retain(|asked| asked != pane);
        // A pane the node was sent is forgotten there too, so its hub keeps
        // no pane the core no longer has.
        if state.started_whole.remove(pane) {
            state.bytes += pane.len();
            state.items.push_back(TapItem::Forget(pane.to_owned()));
            drop(state);
            self.wake.notify_one();
        }
    }

    /// The next message for the node, and the panes to draw again.
    fn take(&self) -> (Option<String>, Vec<String>) {
        let mut state = lock(&self.state);
        let redraws = std::mem::take(&mut state.redraws);
        if state.items.is_empty() {
            return (None, redraws);
        }
        let mut taken = Vec::new();
        let mut size = 0;
        while taken.len() < MESSAGE_ITEMS && (taken.is_empty() || size < MESSAGE_BYTES) {
            let Some(item) = state.items.pop_front() else {
                break;
            };
            state.bytes -= item.size();
            size += item.size();
            taken.push(item);
        }
        let more = !state.items.is_empty();
        drop(state);
        if more {
            self.wake.notify_one();
        }
        // Serialized outside the tap's lock.
        let mut text = String::new();
        for item in taken {
            let terminal = match item {
                TapItem::Output(output) => TerminalUp::Output(output),
                TapItem::Forget(pane) => TerminalUp::Forget { pane },
            };
            if let Ok(encoded) = serde_json::to_string(&TerminalLine { terminal }) {
                text.push_str(&encoded);
                text.push('\n');
            }
        }
        (Some(text), redraws)
    }

    fn is_closed(&self) -> bool {
        lock(&self.state).closed
    }

    fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        state.items.clear();
        state.bytes = 0;
    }
}

/// Answers one linked node's screens may wait on at once (D-20): file reads
/// and folder listings of a device's checkout, which run beside the screen
/// and answer later. Past it a new one is refused with `relay_busy`.
pub const MAX_RELAY_REQUESTS: usize = 64;

/// What one linked node's screens wait on this core for. Each answer that
/// runs beside a screen holds a slot until it has answered.
#[derive(Debug, Default)]
pub struct RelayRequests {
    waiting: AtomicUsize,
    refused: AtomicU64,
    /// The node's screens relayed now.
    screens: AtomicUsize,
}

/// The screens one linked node may relay at once: fewer than this core's
/// client cap, so a node's windows never leave the core's own machine
/// without a place for its own.
pub const MAX_RELAY_SCREENS: usize = crate::state_file::MAX_CLIENTS - 2;

/// One relayed screen's place among [`MAX_RELAY_SCREENS`].
#[derive(Debug)]
pub struct RelayScreenSlot(Arc<RelayRequests>);

impl Drop for RelayScreenSlot {
    fn drop(&mut self) {
        self.0.screens.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One waiting answer's place among [`MAX_RELAY_REQUESTS`], given back
/// however the answer ends.
#[derive(Debug)]
pub struct RelaySlot(Arc<RelayRequests>);

impl Drop for RelaySlot {
    fn drop(&mut self) {
        self.0.waiting.fetch_sub(1, Ordering::AcqRel);
    }
}

impl RelayRequests {
    /// A place for one more of `node`'s screens, or `None` at the cap,
    /// which is logged.
    pub fn take_screen(self: &Arc<Self>, node: &str) -> Option<RelayScreenSlot> {
        let taken = self
            .screens
            .try_update(Ordering::AcqRel, Ordering::Acquire, |screens| {
                (screens < MAX_RELAY_SCREENS).then_some(screens + 1)
            })
            .is_ok();
        if taken {
            return Some(RelayScreenSlot(Arc::clone(self)));
        }
        herdr_core::diagnostic!(json!({
            "component": "node_relay",
            "kind": "relay.screens_full",
            "node": node,
            "cap": MAX_RELAY_SCREENS,
        }));
        None
    }

    /// A slot for one more answer to `node`'s screens, or `None` at the
    /// cap, which is logged.
    pub fn take(self: &Arc<Self>, node: &str) -> Option<RelaySlot> {
        let taken = self
            .waiting
            .try_update(Ordering::AcqRel, Ordering::Acquire, |waiting| {
                (waiting < MAX_RELAY_REQUESTS).then_some(waiting + 1)
            })
            .is_ok();
        if taken {
            return Some(RelaySlot(Arc::clone(self)));
        }
        let refused = self.refused.fetch_add(1, Ordering::Relaxed) + 1;
        if refused.is_power_of_two() {
            herdr_core::diagnostic!(json!({
                "component": "node_relay",
                "kind": "relay.requests_full",
                "node": node,
                "cap": MAX_RELAY_REQUESTS,
                "refused": refused,
            }));
        }
        None
    }
}

/// Serves one linked node's terminals relay until the node closes it or its
/// link ends: every pane's output but the node's own goes down it, and the
/// node's keys and redraws for those panes come up it.
pub async fn serve_terminals(
    socket: WebSocket,
    outputs: Arc<ScreenOutputs>,
    terminals: Arc<dyn TerminalNode>,
    node: String,
    link: RemoteHost,
) {
    let tap = RelayTap::new(&node);
    outputs.add(Arc::clone(&tap));
    herdr_core::diagnostic!(json!({
        "component": "node_relay",
        "kind": "relay.terminals_opened",
        "node": node,
        "link": link.identity(),
    }));
    // Every pane this machine draws starts whole on the node.
    for pane in outputs.hub.panes() {
        if !pane.starts_with(&tap.own_prefix) {
            terminals.redraw(&pane);
        }
    }
    let (sink, stream) = socket.split();
    let reason = relay_terminals(sink, stream, &tap, terminals, &node, link.closed(), || {
        outputs.remove(&tap)
    })
    .await;
    herdr_core::diagnostic!(json!({
        "component": "node_relay",
        "kind": "relay.terminals_closed",
        "node": node,
        "reason": reason,
    }));
}

/// The relay's two directions until one ends: the tap's output goes down
/// on a task of its own, so a send the node is slow to take (a 1 MiB message
/// over a slow forward) never stops this relay reading the node's keys or
/// the link's end. The node side is split the same way. `ended` runs as
/// soon as the relay ends, before the close frame, so the core stops
/// collecting output for a relay that no longer sends it.
async fn relay_terminals<S, R>(
    sink: S,
    mut stream: R,
    tap: &Arc<RelayTap>,
    terminals: Arc<dyn TerminalNode>,
    node: &str,
    link_closed: impl std::future::Future<Output = String>,
    ended: impl FnOnce(),
) -> &'static str
where
    S: futures_util::Sink<Message> + Unpin + Send + 'static,
    R: futures_util::Stream<Item = Result<Message, axum::Error>> + Unpin,
{
    let own_prefix = tap.own_prefix.clone();
    let mut writer = {
        let tap = Arc::clone(tap);
        let terminals = Arc::clone(&terminals);
        let mut sink = sink;
        tokio::spawn(async move {
            let reason = loop {
                tap.wake.notified().await;
                if tap.is_closed() {
                    break "replaced";
                }
                let (text, redraws) = tap.take();
                for pane in redraws {
                    terminals.redraw(&pane);
                }
                if let Some(text) = text
                    && sink.send(Message::Text(text.into())).await.is_err()
                {
                    break "node_closed";
                }
            };
            (reason, sink)
        })
    };
    let _writer = AbortOnDrop(writer.abort_handle());
    let mut link_closed = std::pin::pin!(link_closed);
    // Set once the writer has finished: its sink when it handed one back,
    // None when it panicked or was cancelled. A finished task is never
    // polled again.
    let mut finished: Option<Option<S>> = None;
    let reason = loop {
        tokio::select! {
            written = &mut writer => {
                let (reason, sink) = match written {
                    Ok((reason, sink)) => (reason, Some(sink)),
                    Err(_) => ("writer_ended", None),
                };
                finished = Some(sink);
                break reason;
            },
            incoming = stream.next() => match incoming {
                Some(Ok(Message::Text(text))) => {
                    if let Err(reason) = take_down(&text, terminals.as_ref(), &own_prefix) {
                        herdr_core::diagnostic!(json!({
                            "component": "node_relay",
                            "kind": "relay.terminals_line_refused",
                            "node": node,
                            "reason": reason,
                        }));
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break "node_closed",
                Some(Ok(_)) => {}
            },
            _ = &mut link_closed => break "link_ended",
        }
    };
    tap.close();
    ended();
    let closing = match finished {
        Some(sink) => sink,
        None => {
            // A writer between sends sees the tap closed and hands its sink
            // back for the close; one held by a send the node does not take
            // is ended with the relay.
            tap.wake.notify_one();
            match tokio::time::timeout(WRITER_HANDBACK, &mut writer).await {
                Ok(written) => written.ok().map(|(_, sink)| sink),
                Err(_) => {
                    writer.abort();
                    None
                }
            }
        }
    };
    // The close is a courtesy: a node that takes nothing more has it for as
    // long as a handback lasts, never longer.
    if let Some(mut sink) = closing {
        let _ = tokio::time::timeout(WRITER_HANDBACK, sink.send(Message::Close(None))).await;
    }
    reason
}

/// One line a node's screens sent for a pane of this machine: a key, a view
/// or a redraw. A control, and anything for the node's own panes, is not the
/// node's to send.
fn take_down(text: &str, terminals: &dyn TerminalNode, own_prefix: &str) -> Result<(), String> {
    if text.len() > MAX_DOWN_LINE {
        return Err("line_too_long".to_owned());
    }
    let line: TerminalLine<TerminalDown> =
        serde_json::from_str(text).map_err(|error| format!("{:?}", error.classify()))?;
    match line.terminal {
        TerminalDown::Key { target, data, .. } => {
            if let hide_node_link::terminal::KeyTarget::Pane(pane) = &target
                && pane.starts_with(own_prefix)
            {
                return Err("own_pane".to_owned());
            }
            let bytes = decode_base64(&data)?;
            // Stamped here, on the core's clock, as a key from a screen of
            // this machine is.
            terminals.key(target, bytes, crate::server::unix_ms_now());
        }
        // A node's screens send their views with their other events, so the
        // core decides the grid each is drawn at (`pane_sizes`).
        TerminalDown::View { .. } => return Err("view".to_owned()),
        TerminalDown::Redraw { pane } => {
            if pane.starts_with(own_prefix) {
                return Err("own_pane".to_owned());
            }
            terminals.redraw(&pane);
        }
        TerminalDown::Control { .. } | TerminalDown::InputPermit { .. } => {
            return Err("control".to_owned());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys a relay took, in order.
    #[derive(Default)]
    struct Keys(Mutex<Vec<Vec<u8>>>);

    impl TerminalNode for Keys {
        fn control(&self, _: hide_node_link::terminal::TerminalControl) {}
        fn key(&self, _: hide_node_link::terminal::KeyTarget, bytes: Vec<u8>, _: u64) {
            lock(&self.0).push(bytes);
        }
        fn view(&self, _: &str, _: hide_node_link::terminal::GridSize, _: bool) {}
        fn redraw(&self, _: &str) {}
    }

    #[test]
    fn a_screen_cannot_request_an_input_permit() {
        let keys = Keys::default();
        let line = serde_json::to_string(&TerminalLine {
            terminal: TerminalDown::InputPermit {
                ticket: 1,
                pane: "core-pane".into(),
                bytes: 4,
            },
        })
        .unwrap();
        assert_eq!(take_down(&line, &keys, "device:"), Err("control".into()));
        assert!(lock(&keys.0).is_empty());
    }

    /// A send the node does not take (a large message over a slow forward)
    /// holds only the relay's writer: the node's keys are still read and
    /// typed, and the link's end still ends the relay.
    #[tokio::test]
    async fn a_send_the_node_does_not_take_never_stops_the_relay_reading_its_keys() {
        let tap = RelayTap::new("node-b");
        tap.output("core-pane", b"\x1bcwhole", true);
        let keys = Arc::new(Keys::default());
        let (sending, send_started) = tokio::sync::oneshot::channel::<()>();
        let sending = Mutex::new(Some(sending));
        let stalled = Box::pin(futures_util::sink::unfold((), move |(), _: Message| {
            if let Some(sending) = lock(&sending).take() {
                let _ = sending.send(());
            }
            std::future::pending::<Result<(), axum::Error>>()
        }));
        let (keys_in, keys_out) = tokio::sync::mpsc::unbounded_channel::<Message>();
        let stream = Box::pin(futures_util::stream::unfold(
            keys_out,
            |mut keys| async move { keys.recv().await.map(|message| (Ok(message), keys)) },
        ));
        let (end, ended) = tokio::sync::oneshot::channel::<()>();
        let relay = {
            let tap = Arc::clone(&tap);
            let terminals: Arc<dyn TerminalNode> = keys.clone();
            tokio::spawn(async move {
                relay_terminals(
                    stalled,
                    stream,
                    &tap,
                    terminals,
                    "node-b",
                    async move {
                        let _ = ended.await;
                        "closed".to_owned()
                    },
                    || {},
                )
                .await
            })
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), send_started)
            .await
            .expect("the relay never sent the output")
            .unwrap();
        let key = serde_json::to_string(&TerminalLine {
            terminal: TerminalDown::Key {
                target: hide_node_link::terminal::KeyTarget::Pane("core-pane".to_owned()),
                data: encode_base64(b"k"),
                typed_at_unix_ms: 1,
            },
        })
        .unwrap();
        keys_in.send(Message::Text(key.into())).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while lock(&keys.0).is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the key was not read while the send was held");
        end.send(()).unwrap();
        let reason = tokio::time::timeout(std::time::Duration::from_secs(5), relay)
            .await
            .expect("the link's end did not end the relay")
            .unwrap();
        assert_eq!(reason, "link_ended");
        assert_eq!(lock(&keys.0).as_slice(), [b"k".to_vec()]);
    }

    /// A writer that dies mid-send (a panic in the socket's send) ends the
    /// relay as `writer_ended`; the relay never polls the finished task again.
    #[tokio::test]
    async fn a_writer_that_dies_ends_the_relay_without_a_second_poll() {
        let tap = RelayTap::new("node-b");
        tap.output("core-pane", b"\x1bcwhole", true);
        let panicking = Box::pin(futures_util::sink::unfold((), |(), _: Message| async {
            panic!("the socket's send panicked");
            #[allow(unreachable_code)]
            Ok::<(), axum::Error>(())
        }));
        let stream = Box::pin(futures_util::stream::pending::<Result<Message, axum::Error>>());
        let terminals: Arc<dyn TerminalNode> = Arc::new(Keys::default());
        let relay = tokio::spawn(async move {
            relay_terminals(
                panicking,
                stream,
                &tap,
                terminals,
                "node-b",
                std::future::pending(),
                || {},
            )
            .await
        });
        let reason = tokio::time::timeout(std::time::Duration::from_secs(5), relay)
            .await
            .expect("the relay did not end")
            .expect("the relay panicked");
        assert_eq!(reason, "writer_ended");
    }

    /// A relay that ended stops collecting output first, then sends its
    /// close frame only as long as a handback lasts: a node that takes
    /// nothing more never holds the relay open.
    #[tokio::test]
    async fn a_close_the_node_does_not_take_never_holds_the_relay() {
        let tap = RelayTap::new("node-b");
        let order = Arc::new(Mutex::new(Vec::new()));
        let sent = Arc::clone(&order);
        let stalled = Box::pin(futures_util::sink::unfold((), move |(), _: Message| {
            lock(&sent).push("close");
            std::future::pending::<Result<(), axum::Error>>()
        }));
        let removed = Arc::clone(&order);
        let stream = Box::pin(futures_util::stream::empty::<Result<Message, axum::Error>>());
        let terminals: Arc<dyn TerminalNode> = Arc::new(Keys::default());
        let relay = tokio::spawn(async move {
            relay_terminals(
                stalled,
                stream,
                &tap,
                terminals,
                "node-b",
                std::future::pending(),
                move || lock(&removed).push("removed"),
            )
            .await
        });
        let reason = tokio::time::timeout(std::time::Duration::from_secs(5), relay)
            .await
            .expect("the close held the relay")
            .unwrap();
        assert_eq!(reason, "node_closed");
        assert_eq!(lock(&order).as_slice(), ["removed", "close"]);
    }

    #[test]
    fn a_nodes_screens_wait_on_at_most_the_cap_of_answers() {
        let requests = Arc::new(RelayRequests::default());
        let held: Vec<RelaySlot> = (0..MAX_RELAY_REQUESTS)
            .map(|_| requests.take("node").expect("under the cap"))
            .collect();
        assert!(requests.take("node").is_none(), "the next one is refused");
        drop(held);
        assert!(
            requests.take("node").is_some(),
            "an answer gives its slot back"
        );
    }

    /// A node relays at most `MAX_RELAY_SCREENS` screens, fewer than the
    /// core's client cap, so the core's own windows keep a place.
    #[test]
    fn a_node_relays_fewer_screens_than_the_core_takes() {
        let requests = Arc::new(RelayRequests::default());
        let held: Vec<RelayScreenSlot> = (0..MAX_RELAY_SCREENS)
            .map(|_| requests.take_screen("node").expect("a place under the cap"))
            .collect();
        assert!(requests.take_screen("node").is_none());
        drop(held);
        assert!(requests.take_screen("node").is_some());
    }

    fn outputs(tap: &Arc<RelayTap>) -> Vec<TerminalOutput> {
        let (text, _) = tap.take();
        text.unwrap_or_default()
            .lines()
            .map(|line| {
                match serde_json::from_str::<TerminalLine<TerminalUp>>(line)
                    .unwrap()
                    .terminal
                {
                    TerminalUp::Output(output) => output,
                    other => panic!("{other:?}"),
                }
            })
            .collect()
    }

    #[test]
    fn a_tap_skips_the_nodes_own_panes_and_starts_each_pane_whole() {
        let tap = RelayTap::new("mac");
        tap.output("remote:mac:pane:w1:p1", b"own", true);
        tap.output("w1:p1", b"partial", false);
        assert!(outputs(&tap).is_empty());
        let (_, redraws) = tap.take();
        assert!(redraws.is_empty(), "the redraw was asked once already");
        tap.output("w1:p1", b"whole", true);
        tap.output("w1:p1", b"more", false);
        let sent = outputs(&tap);
        assert_eq!(
            sent.iter().map(|output| output.full).collect::<Vec<_>>(),
            [true, false]
        );
        assert!(sent.iter().all(|output| output.pane == "w1:p1"));
    }

    /// A pane the core forgets is forgotten by the node it was sent to, so
    /// the node's hub keeps only panes the core still has; a pane the node
    /// never drew needs no word (R6).
    #[test]
    fn a_pane_the_core_forgets_is_forgotten_on_the_node() {
        let tap = RelayTap::new("mac");
        tap.output("w1:p1", b"whole", true);
        assert_eq!(outputs(&tap).len(), 1);
        tap.output("w1:p1", b"more", false);
        tap.forget("w1:p1");
        tap.forget("w1:p2");
        let (text, _) = tap.take();
        let lines: Vec<TerminalUp> = text
            .unwrap_or_default()
            .lines()
            .map(|line| {
                serde_json::from_str::<TerminalLine<TerminalUp>>(line)
                    .unwrap()
                    .terminal
            })
            .collect();
        assert!(
            matches!(lines.as_slice(), [TerminalUp::Forget { pane }] if pane == "w1:p1"),
            "{lines:?}"
        );
        // The pane coming back starts whole again.
        tap.output("w1:p1", b"partial", false);
        assert!(outputs(&tap).is_empty());
    }

    /// A pane forgotten twice before the relay sends keeps its one Forget,
    /// so the node's hub never keeps it.
    #[test]
    fn a_pane_forgotten_twice_before_a_send_is_still_forgotten_once() {
        let tap = RelayTap::new("mac");
        tap.output("w1:p1", b"whole", true);
        assert_eq!(outputs(&tap).len(), 1);
        tap.forget("w1:p1");
        tap.forget("w1:p1");
        let (text, _) = tap.take();
        let lines: Vec<TerminalUp> = text
            .unwrap_or_default()
            .lines()
            .map(|line| {
                serde_json::from_str::<TerminalLine<TerminalUp>>(line)
                    .unwrap()
                    .terminal
            })
            .collect();
        assert!(
            matches!(lines.as_slice(), [TerminalUp::Forget { pane }] if pane == "w1:p1"),
            "{lines:?}"
        );
    }

    #[test]
    fn a_tap_past_its_cap_drops_the_backlog_and_waits_for_full_frames() {
        let tap = RelayTap::new("mac");
        tap.output("w1:p1", b"whole", true);
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..4 {
            tap.output("w1:p1", &chunk, false);
        }
        let (text, redraws) = tap.take();
        assert!(text.is_none(), "the backlog was not dropped");
        assert_eq!(redraws, ["w1:p1"]);
        tap.output("w1:p1", b"partial", false);
        assert!(outputs(&tap).is_empty());
        tap.output("w1:p1", b"whole again", true);
        assert_eq!(outputs(&tap).len(), 1);
    }

    /// Remote audit R3: full frames count against the tap's cap. A node that
    /// reads nothing while eight panes draw whole holds at most the cap, and
    /// keeps the panes that fit; once it reads, every pane reaches it whole,
    /// each starting from a full frame.
    #[test]
    fn full_frames_of_many_panes_stay_under_the_tap_s_cap_and_every_pane_draws_again() {
        let tap = RelayTap::new("mac");
        let held = |tap: &RelayTap| lock(&tap.state).bytes;
        let panes: Vec<String> = (1..=8).map(|n| format!("w1:p{n}")).collect();
        // A full frame of 1 MiB once encoded, so the eight pass 4 MiB.
        let frame = vec![b'x'; 768 * 1024];
        for pane in &panes {
            tap.output(pane, &frame, true);
            tap.output(pane, b"after", false);
            assert!(held(&tap) <= TAP_CAP_BYTES, "{} held", held(&tap));
        }
        assert!(
            lock(&tap.state).redraws.len() < panes.len(),
            "some panes were kept"
        );

        // The node reads again: each message it takes is followed by the
        // full frames its redraws ask for.
        let mut drawn = HashSet::new();
        loop {
            let (text, redraws) = tap.take();
            for line in text.as_deref().unwrap_or_default().lines() {
                let TerminalUp::Output(output) =
                    serde_json::from_str::<TerminalLine<TerminalUp>>(line)
                        .unwrap()
                        .terminal
                else {
                    panic!("only output was queued");
                };
                if output.full {
                    drawn.insert(output.pane);
                } else {
                    assert!(
                        drawn.contains(&output.pane),
                        "{} before its frame",
                        output.pane
                    );
                }
            }
            if text.is_none() && redraws.is_empty() {
                break;
            }
            for pane in redraws {
                tap.output(&pane, &frame, true);
                assert!(held(&tap) <= TAP_CAP_BYTES, "{} held", held(&tap));
            }
        }
        assert_eq!(drawn, panes.into_iter().collect::<HashSet<_>>());
        assert_eq!(held(&tap), 0);
    }

    /// A full frame that alone passes the cap is not relayed and asks no
    /// redraw, since the next is likely as long; one that fits draws.
    #[test]
    fn a_full_frame_past_the_tap_s_cap_waits_for_one_that_fits() {
        let tap = RelayTap::new("mac");
        tap.output("w1:p1", &vec![b'x'; TAP_CAP_BYTES], true);
        tap.output("w1:p1", b"after", false);
        let (text, redraws) = tap.take();
        assert!(text.is_none());
        assert!(redraws.is_empty(), "a long frame is not asked again");
        tap.output("w1:p1", b"whole", true);
        assert_eq!(outputs(&tap).len(), 1);
    }
}
