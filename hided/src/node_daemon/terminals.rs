//! The node's terminals for its screens: this machine's panes from its
//! own terminals, every other pane through the one terminals relay the node
//! keeps to its core, with keys for those held while it is away.

use super::*;
use crate::owned_task::AbortOnDrop;
use hide_node_link::address::{is_pane_of, pane_of, scoped_pane_id};

/// Terminal lines for the core's panes waiting to go up the terminals
/// relay, in bytes; past it a key is refused to its screen.
const HELD_BYTES: usize = 64 * 1024;
/// How long one such line may wait; past it it is dropped and its screen
/// told.
const HELD_FOR: Duration = Duration::from_secs(3);
/// How long a terminals relay that failed waits before it opens again on
/// the same link.
const RELAY_RETRY: Duration = Duration::from_secs(1);

/// This machine's panes, as its hub names them: the core's names for them.
pub(super) struct OwnPanes {
    pub(super) hub: Arc<TerminalHub>,
    /// This machine's node id, which the core scopes its panes under.
    pub(super) node: String,
}

impl OutputSink for OwnPanes {
    fn output(&self, pane: &str, bytes: &[u8], full: bool) {
        self.hub
            .output(&scoped_pane_id(&self.node, pane), bytes, full);
    }

    fn forget(&self, pane: &str) {
        self.hub.forget(&scoped_pane_id(&self.node, pane));
    }
}

/// Where a screen's keys and redraws go: this machine's panes to
/// its own terminals for the link's life, every other pane up the
/// terminals relay.
pub struct ScreenTerminals {
    pub(super) own_node: String,
    live: watch::Receiver<Option<Arc<LiveLink>>>,
    held: Mutex<HeldLines>,
    /// Wakes the terminals relay's writer when a line is held.
    ready: Notify,
    /// Keys each screen typed for the core's panes that were not sent.
    pub(super) unsent: Arc<UnsentNotices>,
}

/// Why a key a screen typed for one of the core's panes was not sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Unsent {
    /// It waited for the terminals relay longer than [`HELD_FOR`].
    WaitedTooLong,
    /// The link it was typed for ended before it went up.
    LinkEnded,
    /// The relay's write of it failed or was cut off.
    CutOff,
}

impl Unsent {
    const ALL: [Unsent; 3] = [Unsent::WaitedTooLong, Unsent::LinkEnded, Unsent::CutOff];

    fn bit(self) -> u8 {
        1 << self as u8
    }

    fn says(self) -> &'static str {
        match self {
            Unsent::WaitedTooLong => "they waited too long",
            Unsent::LinkEnded => "the link to the core ended",
            Unsent::CutOff => "the relay to the core was cut off",
        }
    }
}

/// Not-sent keys per live screen connection, told once per burst: each
/// screen is woken once however many of its lines were lost, and a screen
/// slow to look loses nothing, because the count waits here rather than in
/// a queue that could pass it by. Bounded by the live screens: a line whose
/// screen already left is only logged.
#[derive(Default)]
pub(super) struct UnsentNotices {
    screens: Mutex<HashMap<u64, Pending>>,
}

#[derive(Default)]
struct Pending {
    keys: usize,
    causes: u8,
    wake: Arc<Notify>,
}

impl UnsentNotices {
    /// Starts counting for one screen connection until the returned handle
    /// is dropped.
    pub(super) fn watch(self: &Arc<Self>, connection: u64) -> UnsentScreen {
        let wake = Arc::new(Notify::new());
        lock(&self.screens).insert(
            connection,
            Pending {
                wake: Arc::clone(&wake),
                ..Pending::default()
            },
        );
        UnsentScreen {
            notices: Arc::clone(self),
            connection,
            wake,
        }
    }

    fn report(&self, connection: u64, cause: Unsent) {
        let mut screens = lock(&self.screens);
        if let Some(pending) = screens.get_mut(&connection) {
            pending.keys += 1;
            pending.causes |= cause.bit();
            pending.wake.notify_one();
        }
    }
}

/// One screen connection's not-sent keys.
pub(super) struct UnsentScreen {
    notices: Arc<UnsentNotices>,
    connection: u64,
    wake: Arc<Notify>,
}

impl UnsentScreen {
    /// Resolves when keys of this screen were not sent since it last looked.
    pub(super) async fn ready(&self) {
        self.wake.notified().await;
    }

    /// The one notice for every key not sent since the last, if any.
    pub(super) fn take(&self) -> Option<String> {
        let mut screens = lock(&self.notices.screens);
        let pending = screens.get_mut(&self.connection)?;
        if pending.keys == 0 {
            return None;
        }
        let keys = std::mem::take(&mut pending.keys);
        let causes = std::mem::take(&mut pending.causes);
        drop(screens);
        let why: Vec<&str> = Unsent::ALL
            .into_iter()
            .filter(|cause| causes & cause.bit() != 0)
            .map(Unsent::says)
            .collect();
        let what = if keys == 1 {
            "A key for one of the core's panes was not sent".to_owned()
        } else {
            format!("{keys} keys for the core's panes were not sent")
        };
        Some(format!("{what}: {}", why.join("; ")))
    }
}

impl Drop for UnsentScreen {
    fn drop(&mut self) {
        lock(&self.notices.screens).remove(&self.connection);
    }
}

/// Lines for the core's panes not yet written up the terminals relay, in
/// the order the screens sent them.
#[derive(Default)]
struct HeldLines {
    lines: VecDeque<HeldLine>,
    bytes: usize,
}

struct HeldLine {
    text: String,
    connection: u64,
    /// A key or a paste, which its screen is told of when it is not sent.
    /// A redraw is not: the screen's next view asks again.
    input: bool,
    at: Instant,
}

impl HeldLine {
    /// The screen to tell when this line is not sent: its typist's, for
    /// input only.
    fn teller(&self) -> Option<u64> {
        self.input.then_some(self.connection)
    }

    /// Drops this line for `cause`: an input line's screen is told, a
    /// redraw is only logged.
    fn drop_for(self, cause: Unsent, unsent: &UnsentNotices) {
        match self.teller() {
            Some(connection) => not_sent(connection, cause, unsent),
            None => herdr_core::diagnostic!(json!({
                "component": "node_daemon",
                "kind": "terminals.redraw_dropped",
                "connection": self.connection,
                "cause": format!("{cause:?}"),
            })),
        }
    }
}

impl HeldLines {
    /// The next line still in time and the screen to tell if it is not
    /// sent (none for a redraw), dropping each line that is not in time.
    fn next(&mut self, now: Instant, unsent: &UnsentNotices) -> Option<(String, Option<u64>)> {
        while let Some(line) = self.lines.pop_front() {
            self.bytes -= line.text.len();
            if now.duration_since(line.at) <= HELD_FOR {
                let teller = line.teller();
                return Some((line.text, teller));
            }
            line.drop_for(Unsent::WaitedTooLong, unsent);
        }
        None
    }

    /// Drops every line older than [`HELD_FOR`], telling its screen.
    fn expire(&mut self, now: Instant, unsent: &UnsentNotices) {
        while self
            .lines
            .front()
            .is_some_and(|line| now.duration_since(line.at) > HELD_FOR)
        {
            if let Some(line) = self.lines.pop_front() {
                self.bytes -= line.text.len();
                line.drop_for(Unsent::WaitedTooLong, unsent);
            }
        }
    }

    /// Drops every line: the link they were typed for ended.
    fn clear(&mut self, unsent: &UnsentNotices) {
        for line in self.lines.drain(..) {
            line.drop_for(Unsent::LinkEnded, unsent);
        }
        self.bytes = 0;
    }
}

/// A held line taken for the relay: an input line's screen is told it was
/// not sent unless the write finished, so a write that failed or was cut off
/// with its relay loses no key unsaid. A redraw's `connection` is `None`.
struct InFlight<'a> {
    connection: Option<u64>,
    unsent: &'a UnsentNotices,
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if let Some(connection) = self.connection.take() {
            not_sent(connection, Unsent::CutOff, self.unsent);
        }
    }
}

fn not_sent(connection: u64, cause: Unsent, unsent: &UnsentNotices) {
    herdr_core::diagnostic!(json!({
        "component": "node_daemon",
        "kind": match cause {
            Unsent::CutOff => "terminals.held_unsent",
            Unsent::WaitedTooLong | Unsent::LinkEnded => "terminals.held_dropped",
        },
        "connection": connection,
        "cause": format!("{cause:?}"),
    }));
    unsent.report(connection, cause);
}

impl ScreenTerminals {
    pub(super) fn new(own_node: String, live: watch::Receiver<Option<Arc<LiveLink>>>) -> Self {
        Self {
            own_node,
            live,
            held: Mutex::new(HeldLines::default()),
            ready: Notify::new(),
            unsent: Arc::default(),
        }
    }

    fn own(&self) -> Option<Arc<LiveLink>> {
        self.live.borrow().clone()
    }

    /// Holds `line` for the terminals relay, behind every line held before
    /// it; refused past [`HELD_BYTES`]. One line is always held, however
    /// large, so a paste goes up as it would on the core's own screen.
    fn up(&self, line: TerminalDown, connection: u64) -> Result<(), String> {
        let input = matches!(line, TerminalDown::Key { .. });
        let text = serde_json::to_string(&TerminalLine { terminal: line })
            .map_err(|error| error.to_string())?;
        let mut held = lock(&self.held);
        if !held.lines.is_empty() && held.bytes + text.len() > HELD_BYTES {
            drop(held);
            herdr_core::diagnostic!(json!({
                "component": "node_daemon",
                "kind": "terminals.held_full",
                "connection": connection,
                "cap": HELD_BYTES,
            }));
            return Err(
                "The core's panes are not taking keys now; this key was not sent".to_owned(),
            );
        }
        held.bytes += text.len();
        held.lines.push_back(HeldLine {
            text,
            connection,
            input,
            at: Instant::now(),
        });
        drop(held);
        self.ready.notify_one();
        Ok(())
    }

    /// A key a screen typed: into this machine's pane, or held for the
    /// core's.
    pub(super) fn key(
        &self,
        connection: u64,
        target: KeyTarget,
        bytes: Vec<u8>,
        typed_at_unix_ms: u64,
    ) -> Result<(), String> {
        if let KeyTarget::Pane(pane) = &target
            && let Some(own) = pane_of(pane, &self.own_node)
        {
            let Some(link) = self.own() else {
                return Err(
                    "This machine's panes are not reachable now; this key was not sent".to_owned(),
                );
            };
            link.terminals
                .key(KeyTarget::Pane(own.to_owned()), bytes, typed_at_unix_ms);
            return Ok(());
        }
        self.up(
            TerminalDown::Key {
                target,
                data: encode_base64(&bytes),
                typed_at_unix_ms,
            },
            connection,
        )
    }

    /// A screen's view of `pane` needs it drawn whole.
    pub(super) fn redraw(&self, connection: u64, pane: &str) {
        if let Some(own) = pane_of(pane, &self.own_node) {
            if let Some(link) = self.own() {
                link.terminals.redraw(own);
            }
            return;
        }
        // A redraw refused is asked again by the screen's next view.
        let _ = self.up(
            TerminalDown::Redraw {
                pane: pane.to_owned(),
            },
            connection,
        );
    }
}

/// How often a screen tells the core it still types into one pane.
const INPUT_NOTICE_EVERY: Duration = Duration::from_secs(1);
/// The panes a screen's notices are remembered for; past it they start over.
const INPUT_NOTICE_PANES: usize = 64;

/// The panes one screen told its core it typed into, and when: the core
/// sizes a pane at the grid of the screen that last sent it input
/// (`pane_sizes`), and never sees the keys this daemon takes. A screen that
/// keeps typing says so once a second, so another screen's input in
/// between is overruled within that.
#[derive(Default)]
pub(super) struct InputNotices {
    told: HashMap<String, Instant>,
}

impl InputNotices {
    /// The notice to send for a key into `pane` at `now`, if one is due.
    pub(super) fn notice(&mut self, pane: &str, now: Instant) -> Option<String> {
        if self
            .told
            .get(pane)
            .is_some_and(|told| now.duration_since(*told) < INPUT_NOTICE_EVERY)
        {
            return None;
        }
        if self.told.len() >= INPUT_NOTICE_PANES && !self.told.contains_key(pane) {
            self.told.clear();
        }
        self.told.insert(pane.to_owned(), now);
        Some(
            json!({
                "schema_version": SCHEMA_VERSION,
                "kind": "terminal_input",
                "payload": {"pane_id": pane},
            })
            .to_string(),
        )
    }
}

/// A screen's key, taken here; every other event goes to the core. A key
/// into a pane answers that pane. A view goes to the core too, which decides
/// the grid every screen's view of a pane is drawn at (`pane_sizes`) and
/// sends it to the pane's node.
pub(super) fn take_terminal_key(
    state: &NodeState,
    connection: u64,
    event: &Value,
) -> Result<Option<String>, String> {
    let (target, bytes) = terminal_key(event)?;
    let typed = match &target {
        KeyTarget::Pane(pane) => Some(pane.clone()),
        KeyTarget::Request(_) => None,
    };
    state
        .terminals
        .key(connection, target, bytes, crate::server::unix_ms_now())?;
    Ok(typed)
}

/// Keeps one terminals relay to the core for each live link: the core's
/// panes' output into this daemon's hub, and screens' keys and
/// redraws for those panes up to the core.
pub(super) async fn keep_terminals_relay(
    mut live: watch::Receiver<Option<Arc<LiveLink>>>,
    hub: Arc<TerminalHub>,
    terminals: Arc<ScreenTerminals>,
) {
    loop {
        let link = live.borrow_and_update().clone();
        let Some(link) = link else {
            if live.changed().await.is_err() {
                return;
            }
            continue;
        };
        let ended = terminals_relay(&link, &mut live, &hub, &terminals).await;
        herdr_core::diagnostic!(json!({
            "component": "node_daemon",
            "kind": "terminals.relay_ended",
            "generation": link.generation,
            "reason": ended,
        }));
        let still = live
            .borrow()
            .as_ref()
            .is_some_and(|now| now.generation == link.generation);
        if still {
            tokio::time::sleep(RELAY_RETRY).await;
            lock(&terminals.held).expire(Instant::now(), &terminals.unsent);
        } else {
            // Lines typed for a link that ended are never delivered later.
            lock(&terminals.held).clear(&terminals.unsent);
        }
    }
}

async fn terminals_relay(
    link: &LiveLink,
    live: &mut watch::Receiver<Option<Arc<LiveLink>>>,
    hub: &TerminalHub,
    terminals: &Arc<ScreenTerminals>,
) -> String {
    let upstream = match open_relay(link, "terminals").await {
        Ok(upstream) => upstream,
        Err(message) => return format!("open_failed: {message}"),
    };
    let (sink, mut from_core) = upstream.split();
    // Written on a task of its own: a write the core is slow to take never
    // stops this relay reading the core's output, which would fill the SSH
    // channel and stall every other one on the connection.
    let mut writer = tokio::spawn(write_held(Arc::clone(terminals), sink));
    let _abort = AbortOnDrop(writer.abort_handle());
    loop {
        tokio::select! {
            changed = live.changed() => {
                let same = changed.is_ok()
                    && live
                        .borrow()
                        .as_ref()
                        .is_some_and(|now| now.generation == link.generation);
                if !same {
                    return "link_ended".to_owned();
                }
            }
            _ = &mut writer => return "core_closed".to_owned(),
            from_core = from_core.next() => match from_core {
                Some(Ok(tungstenite::Message::Text(text))) => {
                    ingest(hub, &terminals.own_node, &text);
                }
                Some(Ok(tungstenite::Message::Close(_))) | None | Some(Err(_)) => {
                    return "core_closed".to_owned();
                }
                Some(Ok(_)) => {}
            },
        }
    }
}

/// Writes held lines up the relay in order until a write fails; a line taken
/// and not written is told to its screen as not sent.
async fn write_held<S>(terminals: Arc<ScreenTerminals>, mut sink: S)
where
    S: futures_util::Sink<tungstenite::Message> + Unpin,
{
    loop {
        let next = lock(&terminals.held).next(Instant::now(), &terminals.unsent);
        let Some((line, teller)) = next else {
            terminals.ready.notified().await;
            continue;
        };
        let mut in_flight = InFlight {
            connection: teller,
            unsent: &terminals.unsent,
        };
        if sink
            .send(tungstenite::Message::Text(line.into()))
            .await
            .is_err()
        {
            return;
        }
        in_flight.connection = None;
    }
}

/// The core's panes' output, one terminal line each, into the hub.
fn ingest(hub: &TerminalHub, own_node: &str, text: &str) {
    for line in text.lines() {
        let output = match serde_json::from_str::<TerminalLine<TerminalUp>>(line) {
            Ok(TerminalLine {
                terminal: TerminalUp::Output(output),
            }) => output,
            // The pane is gone on the core: nothing of it is kept here.
            Ok(TerminalLine {
                terminal: TerminalUp::Forget { pane },
            }) => {
                if !is_pane_of(&pane, own_node) {
                    hub.forget(&pane);
                }
                continue;
            }
            _ => continue,
        };
        // The core sends no pane of this machine; one it did is not drawn
        // over this machine's own.
        if is_pane_of(&output.pane, own_node) || output.pane.len() > MAX_PANE_ID_BYTES {
            continue;
        }
        if let Ok(bytes) = decode_base64(&output.data) {
            hub.output(&output.pane, &bytes, output.full);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Keys for the core's panes typed while the terminals relay is away
    /// wait in order; past the byte bound the screen is refused at once,
    /// and a key that waited too long is dropped and its screen told (R5).
    #[test]
    fn keys_for_the_core_s_panes_wait_for_the_relay_in_order_and_none_is_lost_unsaid() {
        let terminals =
            ScreenTerminals::new("remote:screen:pane:".to_owned(), watch::channel(None).1);
        let first_screen = terminals.unsent.watch(1);
        let second_screen = terminals.unsent.watch(2);
        let key = |text: &str| KeyTarget::Pane(format!("core-pane-{text}"));
        terminals.key(1, key("a"), b"a".to_vec(), 1).unwrap();
        terminals.key(2, key("b"), b"b".to_vec(), 2).unwrap();
        let now = Instant::now();
        let mut held = lock(&terminals.held);
        let (first, typed_by) = held.next(now, &terminals.unsent).expect("the first key");
        assert!(first.contains("core-pane-a"), "{first}");
        assert_eq!(typed_by, Some(1));
        // The second waited too long: dropped, and its screen told.
        let late = now + HELD_FOR + Duration::from_millis(1);
        assert_eq!(held.next(late, &terminals.unsent), None);
        drop(held);
        assert_eq!(first_screen.take(), None);
        let told = second_screen.take().expect("the second screen is told");
        assert!(told.contains("waited too long"), "{told}");

        // A paste larger than the bound goes alone; behind it, a key that
        // would cross the bound is refused.
        let paste = vec![b'x'; HELD_BYTES * 2];
        terminals.key(3, key("c"), paste, 3).unwrap();
        let refused = terminals.key(3, key("d"), b"d".to_vec(), 4).unwrap_err();
        assert!(refused.contains("not sent"), "{refused}");
        // This machine's own panes never wait for the relay; with no live
        // link a key for one is refused to its screen, never lost unsaid.
        let unreached = terminals
            .key(
                3,
                KeyTarget::Pane("remote:screen:pane:w:p".to_owned()),
                b"x".to_vec(),
                5,
            )
            .unwrap_err();
        assert!(unreached.contains("not sent"), "{unreached}");
    }

    /// A line the relay took and could not write, because the write failed
    /// or the relay ended during it, is told to its screen as not sent.
    #[tokio::test]
    async fn a_key_the_relay_took_and_did_not_send_is_told_to_its_screen() {
        let terminals = Arc::new(ScreenTerminals::new(
            "remote:screen:pane:".to_owned(),
            watch::channel(None).1,
        ));
        let first_screen = terminals.unsent.watch(7);
        let second_screen = terminals.unsent.watch(8);
        let key = KeyTarget::Pane("core-pane".to_owned());
        terminals.key(7, key.clone(), b"a".to_vec(), 1).unwrap();
        let failing = Box::pin(futures_util::sink::unfold(
            (),
            |(), _: tungstenite::Message| async { Err::<(), ()>(()) },
        ));
        write_held(Arc::clone(&terminals), failing).await;
        let told = first_screen.take().expect("the failed write is told");
        assert!(told.contains("cut off"), "{told}");

        terminals.key(8, key, b"b".to_vec(), 2).unwrap();
        let stalled = Box::pin(futures_util::sink::unfold(
            (),
            |(), _: tungstenite::Message| std::future::pending::<Result<(), ()>>(),
        ));
        let writer = tokio::spawn(write_held(Arc::clone(&terminals), stalled));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !lock(&terminals.held).lines.is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the writer never took the line");
        writer.abort();
        let _ = writer.await;
        let told = second_screen.take().expect("the cut-off write is told");
        assert!(told.contains("cut off"), "{told}");
    }

    /// A redraw held beside keys is no key: when it expires, is cleared
    /// with its link or is cut off, the screen hears nothing of it (its next
    /// view asks again), and the key beside it is still told.
    #[tokio::test]
    async fn a_dropped_redraw_is_never_told_as_an_unsent_key() {
        let terminals = Arc::new(ScreenTerminals::new(
            "remote:screen:pane:".to_owned(),
            watch::channel(None).1,
        ));
        let screen = terminals.unsent.watch(1);
        let key = KeyTarget::Pane("core-pane".to_owned());
        terminals.redraw(1, "core-pane");
        terminals.key(1, key.clone(), b"a".to_vec(), 1).unwrap();
        let late = Instant::now() + HELD_FOR + Duration::from_millis(1);
        lock(&terminals.held).expire(late, &terminals.unsent);
        assert_eq!(
            screen.take().as_deref(),
            Some("A key for one of the core's panes was not sent: they waited too long")
        );

        terminals.redraw(1, "core-pane");
        lock(&terminals.held).clear(&terminals.unsent);
        assert_eq!(screen.take(), None, "a cleared redraw");

        terminals.redraw(1, "core-pane");
        let failing = Box::pin(futures_util::sink::unfold(
            (),
            |(), _: tungstenite::Message| async { Err::<(), ()>(()) },
        ));
        write_held(Arc::clone(&terminals), failing).await;
        assert_eq!(screen.take(), None, "a redraw whose write failed");
    }

    /// A burst of lost keys, more than any queue of notices held, reaches
    /// each screen as one notice with the count and the actual cause, and a
    /// screen that already left is only logged.
    #[test]
    fn a_burst_of_unsent_keys_is_one_notice_per_screen_with_its_cause() {
        let terminals =
            ScreenTerminals::new("remote:screen:pane:".to_owned(), watch::channel(None).1);
        let waiting = terminals.unsent.watch(1);
        let linked = terminals.unsent.watch(2);
        let left = terminals.unsent.watch(3);
        let key = KeyTarget::Pane("core-pane".to_owned());
        for at in 0..70 {
            terminals.key(1, key.clone(), b"a".to_vec(), at).unwrap();
        }
        terminals.key(3, key.clone(), b"c".to_vec(), 70).unwrap();
        drop(left);
        let late = Instant::now() + HELD_FOR + Duration::from_millis(1);
        lock(&terminals.held).expire(late, &terminals.unsent);
        for at in 0..70 {
            terminals.key(2, key.clone(), b"b".to_vec(), at).unwrap();
        }
        lock(&terminals.held).clear(&terminals.unsent);

        assert_eq!(
            waiting.take().as_deref(),
            Some("70 keys for the core's panes were not sent: they waited too long")
        );
        assert_eq!(waiting.take(), None, "told once");
        assert_eq!(
            linked.take().as_deref(),
            Some("70 keys for the core's panes were not sent: the link to the core ended")
        );
        assert_eq!(linked.take(), None, "told once");
        assert!(
            lock(&terminals.unsent.screens).len() == 2,
            "the screen that left is not kept"
        );
    }

    #[test]
    fn a_screen_tells_its_core_it_types_into_a_pane_once_a_second() {
        let mut told = InputNotices::default();
        let start = Instant::now();
        let notice = told.notice("p", start).expect("the first key is told");
        let notice: Value = serde_json::from_str(&notice).unwrap();
        assert_eq!(notice["kind"], "terminal_input");
        assert_eq!(notice["payload"]["pane_id"], "p");
        assert!(
            told.notice("p", start + Duration::from_millis(999))
                .is_none()
        );
        assert!(told.notice("q", start).is_some(), "each pane on its own");
        assert!(told.notice("p", start + INPUT_NOTICE_EVERY).is_some());
        // Past the cap the panes start over rather than grow.
        for pane in 0..INPUT_NOTICE_PANES * 2 {
            told.notice(&pane.to_string(), start);
        }
        assert!(told.told.len() <= INPUT_NOTICE_PANES);
    }
}
