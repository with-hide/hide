//! A bounded window of one session's raw conversation, read on the node that
//! holds its file: the events around a time or from an event number, chosen
//! as the session reader's pages stream past. Hide AI's investigation
//! (`suggestions::investigation`) and `hide session read`
//! (`links::session_command`) both read through here, and both pass what it
//! answers through [`egress`] before it leaves the machine.
//!
//! Nothing here writes, and a read never runs under `Mutex<Runtime>`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use hide_node_link::protocol::Call;
use hide_node_link::{LinkError, NodeLink};
use hide_session::{Agent, ConversationEvent, SessionReadScope};

/// How long a node is given to answer one page of a session.
pub(crate) const SESSION_TIMEOUT: Duration = Duration::from_secs(60);
/// The most characters one event of a session shows.
pub(crate) const EVENT_CHARS: usize = 2_000;

/// Which events of a session to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionWindow {
    /// `count` events centred on the first event at or after this time.
    Around { at_unix_ms: u64, count: u32 },
    /// `count` events from event `from` (counted from 0).
    From { from: u32, count: u32 },
}

/// Where one session's file is, as its node spells it.
#[derive(Clone, Debug)]
pub(crate) struct Located {
    pub agent: Agent,
    pub path: String,
    pub scope: SessionReadScope,
}

/// The events a window kept, each with its number (from 0) and how the
/// caller worded it.
#[derive(Debug)]
pub(crate) struct Events<T> {
    pub kept: Vec<(usize, T)>,
    /// Events read, whether kept or not.
    pub seen: usize,
    /// The read reached the session's last event, so `seen` is all of them.
    pub complete: bool,
    /// The read reached the window's first event; a read the byte bound
    /// stopped before it kept no answer to what was asked.
    pub reached: bool,
}

/// How far a read goes past a full window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Extent {
    /// Stop once the window is full.
    Window,
    /// Go on counting the events until the session's end, the read's byte
    /// bound or `deadline`, whichever comes first, so the answer can say
    /// how many there are.
    Count { deadline: Instant },
}

/// Why a read answered nothing.
#[derive(Debug)]
pub(crate) enum ReadFailure {
    Link(LinkError),
    /// [`Extent::Count`]'s deadline passed before the window was full.
    Deadline,
}

impl From<LinkError> for ReadFailure {
    fn from(error: LinkError) -> Self {
        Self::Link(error)
    }
}

/// Reads `window` of the session at `located` through `node`, wording each
/// kept event with `line`. Every page is one bounded reader call; the whole
/// read stops at the session reader's byte bound.
pub(crate) fn read<T>(
    node: &dyn NodeLink,
    located: &Located,
    window: SessionWindow,
    extent: Extent,
    line: &dyn Fn(usize, &ConversationEvent) -> T,
) -> Result<Events<T>, ReadFailure> {
    let mut shown = EventWindow::new(window);
    let mut consumed = 0u64;
    let mut checkpoint = None;
    let mut complete = false;
    for _ in 0..hide_session::cursor::ARCHIVE_PAGE_LIMIT {
        let timeout = match extent {
            Extent::Window => SESSION_TIMEOUT,
            Extent::Count { deadline } => {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    if shown.full() {
                        break;
                    }
                    return Err(ReadFailure::Deadline);
                }
                left.min(SESSION_TIMEOUT)
            }
        };
        let page: hide_session::ConversationRead = hide_node_link::link::call_as_reader(
            node,
            located.agent,
            hide_node_link::sessions::ReaderFeature::Conversation,
            Call::SessionConversation {
                agent: located.agent,
                path: located.path.clone(),
                scope: located.scope.clone(),
                checkpoint: checkpoint.clone(),
            },
            timeout,
        )?;
        consumed = consumed.saturating_add(page.read_bytes);
        for event in &page.events {
            shown.push(event, line);
        }
        if !page.has_more || page.read_bytes == 0 {
            complete = true;
            break;
        }
        if consumed > hide_session::SESSION_READ_LIMIT_BYTES
            || (shown.full() && extent == Extent::Window)
        {
            break;
        }
        checkpoint = Some(page.checkpoint);
    }
    Ok(shown.finish(complete, line))
}

/// Text as it may leave the machine: credentials masked, at most `bytes`
/// long and cut on a character. Answers whether anything was cut.
pub(crate) fn egress(text: &str, bytes: usize) -> (String, bool) {
    let redacted = mask(text);
    if redacted.len() <= bytes {
        return (redacted, false);
    }
    let mut end = bytes;
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    (redacted[..end].to_owned(), true)
}

/// `text` with credentials masked by Project Memory's redactor, the one
/// owner of what counts as a credential. Whatever another agent or a
/// provider reads of a session passes through here before it is cut, so a
/// cut never splits a secret past recognition.
pub(crate) fn mask(text: &str) -> String {
    hide_memory::redact(text).text
}

/// Whether one name of a path is a credential file, a folder named for
/// secrets, or Git's own folder (whose configuration can hold a token in a
/// remote's address): the names nothing reads for an agent or a provider.
pub(crate) fn credential_name(name: &str) -> bool {
    name == ".git" || hide_memory::sensitive_path(name)
}

/// `text` cut to `chars` characters, with `…` when anything was cut.
pub(crate) fn cut(text: &str, chars: usize) -> String {
    cut_marked(text, chars).0
}

/// [`cut`], and whether anything was cut.
pub(crate) fn cut_marked(text: &str, chars: usize) -> (String, bool) {
    let mut kept: String = text.chars().take(chars).collect();
    let cut = kept.len() < text.len();
    if cut {
        kept.push('…');
    }
    (kept, cut)
}

/// The events of one session a read shows, chosen as they stream past
/// page by page: `count` events from the window's pivot, with up to half of
/// them before it when the window is around a time. An event before the
/// pivot is held as read and worded only once it is kept, so the wording
/// (a redaction, for `hide session read`) runs on the shown events alone.
struct EventWindow<T> {
    window: SessionWindow,
    before: usize,
    count: usize,
    ring: VecDeque<(usize, ConversationEvent)>,
    kept: Vec<(usize, T)>,
    pivot: bool,
    seen: usize,
}

impl<T> EventWindow<T> {
    fn new(window: SessionWindow) -> Self {
        let (before, count) = match window {
            SessionWindow::Around { count, .. } => (count as usize / 2, count as usize),
            SessionWindow::From { count, .. } => (0, count as usize),
        };
        Self {
            window,
            before,
            count,
            ring: VecDeque::new(),
            kept: Vec::new(),
            pivot: false,
            seen: 0,
        }
    }

    /// One more event; `line` words it by its number, and only for an event
    /// the window keeps.
    fn push(&mut self, event: &ConversationEvent, line: &dyn Fn(usize, &ConversationEvent) -> T) {
        let index = self.seen;
        self.seen += 1;
        if !self.pivot {
            self.pivot = match self.window {
                SessionWindow::Around { at_unix_ms: at, .. } => event.at_unix_ms >= at,
                SessionWindow::From { from, .. } => index >= from as usize,
            };
            if self.pivot {
                self.keep_ring(line);
            }
        }
        if self.pivot {
            if !self.full() {
                self.kept.push((index, line(index, event)));
            }
        } else if self.before > 0 {
            self.ring.push_back((index, event.clone()));
            if self.ring.len() > self.before {
                self.ring.pop_front();
            }
        }
    }

    fn keep_ring(&mut self, line: &dyn Fn(usize, &ConversationEvent) -> T) {
        let held = std::mem::take(&mut self.ring);
        self.kept.extend(
            held.into_iter()
                .map(|(index, event)| (index, line(index, &event))),
        );
    }

    fn full(&self) -> bool {
        self.kept.len() >= self.count
    }

    /// What the window kept. A window around a time after the session's
    /// last event keeps its end; one the read stopped short of keeps
    /// nothing, rather than events that are not the ones asked for.
    fn finish(
        mut self,
        complete: bool,
        line: &dyn Fn(usize, &ConversationEvent) -> T,
    ) -> Events<T> {
        if complete && !self.pivot && matches!(self.window, SessionWindow::Around { .. }) {
            self.keep_ring(line);
        }
        Events {
            reached: self.pivot || complete,
            kept: if self.pivot || complete {
                self.kept
            } else {
                Vec::new()
            },
            seen: self.seen,
            complete,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fill(window: SessionWindow, times: &[u64]) -> Events<String> {
        let line = |index: usize, _: &ConversationEvent| format!("#{index}");
        let mut shown = EventWindow::new(window);
        for at in times {
            let event =
                ConversationEvent::new("user", hide_session::EventKind::Human, *at, String::new());
            shown.push(&event, &line);
        }
        shown.finish(true, &line)
    }

    fn numbers(events: &Events<String>) -> Vec<usize> {
        events.kept.iter().map(|(index, _)| *index).collect()
    }

    #[test]
    fn a_window_around_a_time_keeps_half_its_events_before_it() {
        let times: Vec<u64> = (0..20).map(|n| n * 10).collect();
        let events = fill(
            SessionWindow::Around {
                at_unix_ms: 100,
                count: 6,
            },
            &times,
        );
        assert_eq!(numbers(&events), [7, 8, 9, 10, 11, 12]);
        assert_eq!(events.seen, 20);
    }

    #[test]
    fn a_window_from_a_number_starts_there_and_one_past_the_end_keeps_nothing() {
        let times: Vec<u64> = (0..5).collect();
        let from = fill(SessionWindow::From { from: 3, count: 4 }, &times);
        assert_eq!(numbers(&from), [3, 4]);
        let past = fill(SessionWindow::From { from: 9, count: 4 }, &times);
        assert!(past.kept.is_empty());
        assert_eq!(past.seen, 5);
    }

    #[test]
    fn a_window_around_a_time_after_the_last_event_keeps_the_end() {
        let events = fill(
            SessionWindow::Around {
                at_unix_ms: 1_000,
                count: 4,
            },
            &[1, 2, 3, 4, 5],
        );
        assert_eq!(numbers(&events), [3, 4]);
    }

    #[test]
    fn egress_masks_a_credential_before_it_cuts() {
        let (text, cut) = egress("token sk-livevalue1234567890abcdef here", 1_000);
        assert!(!text.contains("sk-livevalue"), "{text}");
        assert!(!cut);
        let (short, cut) = egress("가나다라마", 7);
        assert_eq!(short, "가나");
        assert!(cut);
    }

    #[test]
    fn credential_names_cover_keys_secrets_and_the_git_folder() {
        for name in [
            ".env",
            ".env.local",
            "server.pem",
            "id_ed25519",
            "my-secrets",
            ".git",
        ] {
            assert!(credential_name(name), "{name}");
        }
        for name in ["projects", "session.jsonl", "-Users-me-app"] {
            assert!(!credential_name(name), "{name}");
        }
    }
}
