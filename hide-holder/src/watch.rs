//! A pane's facts, sent on a watch connection (PRD engine-bundle-3-core-owns
//! D-18, D-21): its folder, title, bells, prompts, bracketed paste, the
//! program in its foreground, and what its screen rules read on its screen,
//! whole each time any of it changed.
//!
//! A watch takes nothing over: the connection that holds the screen goes on
//! as it was. A holder keeps one watch; a newer one replaces it, and the
//! older is closed `taken_over`. The facts are pulled as the screen is: the
//! PTY reader only marks them changed, and the watch's writer reads them at
//! most once per [`MIN_FACTS_INTERVAL`] and sends them only when they differ
//! from the last it sent. Nothing wakes the writer of a pane whose program
//! is quiet, so an idle pane sends nothing. The first facts on a watch, and
//! the facts after each rule set it is given, are sent whatever they are, so
//! a node that knows nothing yet, or that changed which rules fit, hears the
//! pane at once.

use std::collections::VecDeque;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use hide_holder_protocol::{
    Closed, Facts, FrameError, HolderMessage, LineEditing, ScreenRead, ScreenState, Unknown,
    WatchMessage, read_watch, write_holder,
};
use hide_platform::ipc::{LocalStream, ShutdownHandle};
use hide_screen_rules::{Input, Manifest, Rules, State};

use crate::foreground::Foreground;
use crate::log::Log;
use crate::pane::{Observed, Pane};

/// The least time between two reads of the facts, as the screen is drawn at
/// most once per frame of a 60 Hz screen.
pub const MIN_FACTS_INTERVAL: Duration = Duration::from_millis(16);

/// How often the writer asks the terminal again whether it edits lines
/// while a program other than the pane's own holds it and has not been read
/// reading keys (it edits lines, or that is not known yet): a program turns
/// line editing off to read keys without writing anything, and one that
/// drew its screen first would otherwise be known to read keys only at its
/// next output.
pub const LINE_EDITING_RECHECK: Duration = Duration::from_millis(100);

/// How long after such a program takes the foreground the writer keeps
/// asking: the deadline Hide gives an agent's start (`herdr-core`'s
/// `AGENT_START_DEADLINE`), after which the start has ended either way and
/// the mode is read again at the program's next output.
pub const LINE_EDITING_RECHECK_FOR: Duration = Duration::from_secs(120);

/// What a watch's writer has to do: messages to send first, then the facts
/// when they may have changed.
#[derive(Default)]
pub struct Wake {
    state: Mutex<Pending>,
    wake: Condvar,
}

#[derive(Default)]
struct Pending {
    messages: VecDeque<HolderMessage>,
    changed: bool,
    /// The next facts are sent even when they are the ones last sent.
    owed: bool,
    /// Whether the writer waits for a change; only then does one wake it,
    /// so a flood's every read does not wake it while it waits out the
    /// interval between reads.
    waits_for_change: bool,
}

impl Wake {
    fn pending(&self) -> MutexGuard<'_, Pending> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn send(&self, message: HolderMessage) {
        self.pending().messages.push_back(message);
        self.wake.notify_one();
    }

    /// The pane's output, size or rules may have changed what the facts say.
    pub fn changed(&self) {
        let mut pending = self.pending();
        pending.changed = true;
        let wake = std::mem::take(&mut pending.waits_for_change);
        drop(pending);
        if wake {
            self.wake.notify_one();
        }
    }

    /// The next facts are sent whatever they are.
    pub fn owe(&self) {
        self.pending().owed = true;
        self.changed();
    }
}

/// The rule set the pane's screen is read by, shared by the watch's reader,
/// which sets it, and its writer, which reads by it.
#[derive(Default)]
pub struct RuleSlot {
    /// The watch whose reader may set it: an older watch's late message
    /// changes nothing.
    pub owner: u64,
    /// One more each time the rules change, so the writer starts its
    /// reading afresh.
    generation: u64,
    /// The rule set as the node sent it, to tell a set given again from a
    /// new one.
    manifest: Option<String>,
    /// The compiled rules, or why the set could not be read.
    rules: Option<Result<Arc<Rules>, String>>,
}

pub type SharedRules = Arc<Mutex<RuleSlot>>;

fn slot(rules: &SharedRules) -> MutexGuard<'_, RuleSlot> {
    rules.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Gives the pane `manifest` to read its screen by, for the watch `owner`;
/// answers whether the set changed, and when it did, whether it changed
/// from one agent's rules to another's or to none, which drops the title
/// and progress the rules read (Herdr's change of agent).
fn set_rules(rules: &SharedRules, owner: u64, manifest: Option<String>) -> Option<bool> {
    let compiled = manifest.as_deref().map(|json| {
        serde_json::from_str::<Manifest>(json)
            .map_err(|error| error.to_string())
            .and_then(Manifest::compile)
            .map(Arc::new)
            .map_err(|error| first_line(&error))
    });
    let mut slot = slot(rules);
    if slot.owner != owner || slot.manifest == manifest {
        return None;
    }
    let agent_left = slot.manifest.is_some();
    slot.generation += 1;
    slot.manifest = manifest;
    slot.rules = compiled;
    Some(agent_left)
}

fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or_default().to_owned()
}

/// What the rules read, carried across readings for a rule that says the
/// screen proves nothing.
#[derive(Default)]
pub struct Reading {
    generation: u64,
    previous: Option<(ScreenState, Option<String>)>,
}

impl Reading {
    /// The screen read by `rules` (the slot's `generation`) on `observed`.
    fn read(
        &mut self,
        generation: u64,
        rules: Option<&Result<Arc<Rules>, String>>,
        observed: &Observed,
    ) -> ScreenRead {
        if self.generation != generation {
            self.generation = generation;
            self.previous = None;
        }
        let rules = match rules {
            None => return ScreenRead::NoRules,
            Some(Err(reason)) => {
                return ScreenRead::Unknown {
                    reason: reason.clone(),
                };
            }
            Some(Ok(rules)) => rules,
        };
        let detection = rules.evaluate(Input {
            screen: &observed.screen,
            osc_title: &observed.rules_title,
            osc_progress: &observed.progress,
        });
        let (state, rule) = match detection {
            Some(detection) if detection.skip_state_update => self
                .previous
                .clone()
                .unwrap_or((ScreenState::Unknown, None)),
            Some(detection) => (state(detection.state), Some(detection.rule.to_owned())),
            None => (ScreenState::Unknown, None),
        };
        self.previous = Some((state, rule.clone()));
        ScreenRead::Read { state, rule }
    }
}

fn state(state: State) -> ScreenState {
    match state {
        State::Idle => ScreenState::Idle,
        State::Working => ScreenState::Working,
        State::Blocked => ScreenState::Blocked,
        State::Unknown => ScreenState::Unknown,
    }
}

/// The pane's facts now: what the pane says of itself, its foreground, and
/// its screen read by its rules, with no facts extension.
pub fn facts(
    pane: &Mutex<Pane>,
    rules: &SharedRules,
    reading: &mut Reading,
    foreground: Option<hide_holder_protocol::Program>,
) -> Result<Facts, hide_vt::Error> {
    let (generation, set) = {
        let slot = slot(rules);
        (slot.generation, slot.rules.clone())
    };
    let read_screen = matches!(set, Some(Ok(_)));
    let observed = pane
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .observe(read_screen)?;
    let screen = reading.read(generation, set.as_ref(), &observed);
    Ok(Facts {
        cwd: observed.cwd,
        title: observed.title,
        bells: observed.bells,
        prompts: observed.prompts,
        bracketed_paste: observed.bracketed_paste,
        foreground,
        screen,
        line_editing: None,
        foreground_unread: None,
    })
}

/// How a watch reads whether the terminal edits lines, given the pid of the
/// program in the foreground, if any, and why it asks.
pub type LineEditingRead = Box<dyn FnMut(Option<u32>, LineAsk) -> LineEditing + Send>;

/// Why a watch's writer asks whether the terminal edits lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LineAsk {
    /// For the facts it is about to send, as often as they change. A Unix
    /// holder reads its master; a Windows holder answers what it last read,
    /// since asking its console costs a helper's round trip.
    Facts,
    /// On the writer's timer ([`LINE_EDITING_RECHECK`]), while a program
    /// holds the terminal and the answer is not that it reads keys.
    Recheck,
}

/// A watch's writer: its messages in order, and the facts whenever they may
/// have changed and differ from the last it sent.
pub struct FactsWriter {
    pub generation: u64,
    pub wake: Arc<Wake>,
    pub pane: Arc<Mutex<Pane>>,
    pub rules: SharedRules,
    /// The pane's program, which never counts as its foreground.
    pub shell: u32,
    /// Whether this watch negotiated the optional package extension.
    pub program_package: bool,
    /// The terminal's foreground process group, or why it is not known.
    pub group: Box<dyn Fn() -> Result<u32, Unknown> + Send>,
    /// Whether the terminal edits lines under the foreground program it is
    /// given (`None` for the shell), where the watch's node accepted
    /// [`hide_holder_protocol::Capability::LineEditing`].
    pub line_editing: Option<LineEditingRead>,
    /// Whether the watch's node accepted
    /// [`hide_holder_protocol::Capability::ForegroundUnread`].
    pub foreground_unread: bool,
    pub log: Log,
}

enum Job {
    Send(HolderMessage),
    Read {
        owed: bool,
    },
    /// Nothing changed by the time the terminal's line editing is asked
    /// again ([`LINE_EDITING_RECHECK`]).
    Recheck,
}

impl FactsWriter {
    pub fn serve(
        mut self,
        mut stream: LocalStream,
        shutdown: &ShutdownHandle,
        written: &Sender<()>,
    ) {
        let mut reading = Reading::default();
        let mut foreground = Foreground::default();
        let mut last: Option<Facts> = None;
        let mut next = Instant::now();
        // The program that holds the terminal while it is not read reading
        // keys and since when, so its line editing is asked again for a
        // bounded time.
        let mut editing: Option<(u32, Instant)> = None;
        // Whether the last read could not say what holds the terminal, so
        // the log has it once per spell.
        let mut was_unread = false;
        loop {
            let recheck = editing
                .filter(|(_, since)| since.elapsed() < LINE_EDITING_RECHECK_FOR)
                .map(|_| Instant::now() + LINE_EDITING_RECHECK);
            let owed = match self.next_job(next, recheck) {
                Job::Send(message) => {
                    let closing = matches!(message, HolderMessage::Closed(_));
                    if write_holder(&mut stream, &message).is_err() || closing {
                        break;
                    }
                    continue;
                }
                Job::Recheck => {
                    let sent = last.as_ref().and_then(|facts| facts.line_editing);
                    let program = editing.map(|(pid, _)| pid);
                    let read = self.line_editing.as_mut();
                    if read.map(|read| read(program, LineAsk::Recheck)) == sent {
                        continue;
                    }
                    false
                }
                Job::Read { owed } => owed,
            };
            next = Instant::now() + MIN_FACTS_INTERVAL;
            let (mut program, unread) =
                match foreground.program(self.shell, (self.group)(), &self.log) {
                    Ok(program) => (program, None),
                    Err(unread) => (None, Some(unread)),
                };
            if let Some(unread) = unread
                && !was_unread
            {
                self.log.event(
                    "holder.foreground_unread",
                    &[
                        ("generation", self.generation.into()),
                        ("reason", unread.reason().into()),
                    ],
                );
            }
            was_unread = unread.is_some();
            if !self.program_package
                && let Some(program) = &mut program
            {
                program.package = None;
            }
            let pid = program.as_ref().map(|program| program.pid);
            let facts = match facts(&self.pane, &self.rules, &mut reading, program) {
                Ok(facts) => Facts {
                    line_editing: self
                        .line_editing
                        .as_mut()
                        .map(|read| read(pid, LineAsk::Facts)),
                    foreground_unread: self.foreground_unread.then_some(unread.is_some()),
                    ..facts
                },
                Err(error) => {
                    self.log.event(
                        "holder.facts_unread",
                        &[
                            ("generation", self.generation.into()),
                            ("error", error.to_string().into()),
                        ],
                    );
                    continue;
                }
            };
            // Asked again while the program has not been read reading keys:
            // a Windows holder answers not known until its first recheck
            // has asked the console.
            editing = match (&facts.foreground, facts.line_editing) {
                (Some(program), Some(LineEditing::Edits | LineEditing::Unknown)) => {
                    Some(match editing {
                        Some((pid, since)) if pid == program.pid => (pid, since),
                        _ => (program.pid, Instant::now()),
                    })
                }
                _ => None,
            };
            if !owed && last.as_ref() == Some(&facts) {
                continue;
            }
            last = Some(facts.clone());
            if write_holder(&mut stream, &HolderMessage::Facts(facts)).is_err() {
                break;
            }
        }
        shutdown.shutdown();
        let _ = written.send(());
    }

    /// Waits for a message to send, or for the facts to have changed by the
    /// time they may be read again; with neither, it waits until `recheck`,
    /// when it is given, and with no bound otherwise.
    fn next_job(&self, next: Instant, recheck: Option<Instant>) -> Job {
        let mut pending = self.wake.pending();
        loop {
            if let Some(message) = pending.messages.pop_front() {
                return Job::Send(message);
            }
            let now = Instant::now();
            if pending.changed {
                if now >= next {
                    pending.changed = false;
                    pending.waits_for_change = false;
                    return Job::Read {
                        owed: std::mem::take(&mut pending.owed),
                    };
                }
                pending.waits_for_change = false;
                pending = self
                    .wake
                    .wake
                    .wait_timeout(pending, next - now)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                continue;
            }
            pending.waits_for_change = true;
            pending = match recheck {
                Some(at) if now >= at => {
                    pending.waits_for_change = false;
                    return Job::Recheck;
                }
                Some(at) => {
                    self.wake
                        .wake
                        .wait_timeout(pending, at - now)
                        .unwrap_or_else(PoisonError::into_inner)
                        .0
                }
                None => self
                    .wake
                    .wake
                    .wait(pending)
                    .unwrap_or_else(PoisonError::into_inner),
            };
        }
    }
}

/// A watch's reader: the rule sets the node gives, until it lets go, the
/// stream ends, or the watch is replaced. `gone` hands why it ended to the
/// holder's loop.
pub struct WatchReader {
    pub generation: u64,
    pub wake: Arc<Wake>,
    pub pane: Arc<Mutex<Pane>>,
    pub rules: SharedRules,
    pub log: Log,
}

impl WatchReader {
    pub fn serve(self, mut stream: LocalStream, gone: impl FnOnce(Result<(), FrameError>)) {
        loop {
            match read_watch(&mut stream) {
                Ok(WatchMessage::Rules(manifest)) => {
                    let given = manifest.is_some();
                    if let Some(agent_left) = set_rules(&self.rules, self.generation, manifest) {
                        if agent_left {
                            self.pane
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .clear_retained();
                        }
                        let refused = match &slot(&self.rules).rules {
                            Some(Err(reason)) => Some(reason.clone()),
                            _ => None,
                        };
                        self.log.event(
                            "holder.rules_set",
                            &[
                                ("generation", self.generation.into()),
                                ("given", given.into()),
                                ("refused", serde_json::json!(refused)),
                            ],
                        );
                    }
                    self.wake.owe();
                }
                Ok(WatchMessage::Release) => return gone(Ok(())),
                Err(error) => return gone(Err(error)),
            }
        }
    }
}

/// The watch that follows the pane, as the holder's loop sees it.
pub struct WatchConnection {
    pub generation: u64,
    pub wake: Arc<Wake>,
    pub shutdown: ShutdownHandle,
    /// Says when the writer has written its last message.
    pub written: std::sync::mpsc::Receiver<()>,
}

impl WatchConnection {
    /// Sends `closed` after what is queued and lets the writer end the
    /// stream; answers once it did or `limit` passed.
    pub fn close(self, closed: Closed, limit: Duration) {
        self.wake.send(HolderMessage::Closed(closed));
        if self.written.recv_timeout(limit).is_err() {
            self.shutdown.shutdown();
        }
    }
}

#[cfg(test)]
mod tests {
    use hide_holder_protocol::Size;

    use super::*;

    const CLAUDE_CODE: &str = include_str!("../../hide-agent-adapter/agents/claude-code.toml");

    /// The claude-code agent definition's `[screen]` table, as the node
    /// sends it.
    fn claude_code_rules() -> String {
        let file: toml::Table = toml::from_str(CLAUDE_CODE).unwrap();
        let screen: Manifest = file["screen"].clone().try_into().unwrap();
        serde_json::to_string(&screen).unwrap()
    }

    fn pane() -> Mutex<Pane> {
        Mutex::new(Pane::new(Size { rows: 20, cols: 80 }, None).unwrap())
    }

    fn feed(pane: &Mutex<Pane>, bytes: &[u8]) {
        pane.lock().unwrap().feed(bytes, &mut Vec::new());
    }

    fn rules_given(manifest: Option<String>) -> SharedRules {
        let rules = SharedRules::default();
        assert_eq!(set_rules(&rules, 0, manifest), Some(false));
        rules
    }

    fn read(pane: &Mutex<Pane>, rules: &SharedRules, reading: &mut Reading) -> ScreenRead {
        facts(pane, rules, reading, None).unwrap().screen
    }

    fn working(rule: &str) -> ScreenRead {
        ScreenRead::Read {
            state: ScreenState::Working,
            rule: Some(rule.to_owned()),
        }
    }

    /// A redraw of Claude Code's screen as it draws one: synchronized
    /// output on, home, cleared, the body, off.
    fn redraw(body: &str) -> Vec<u8> {
        let mut bytes = b"\x1b[?2026h\x1b[H\x1b[2J".to_vec();
        bytes.extend_from_slice(body.replace('\n', "\r\n").as_bytes());
        bytes.extend_from_slice(b"\x1b[?2026l");
        bytes
    }

    const PROMPT_BOX: &str = "\
────────────────────────────────────────────────────────────
❯
────────────────────────────────────────────────────────────
  ⏵⏵ accept edits on (shift+tab to cycle)";

    fn working_body(spinner: char, seconds: u32) -> String {
        format!(
            "● I'll read the parser first.\n\n● Read(src/pane.rs)\n  ⎿  Read 802 lines\n\n\
             {spinner} Pondering… ({seconds}s · ↓ 1.2k tokens · esc to interrupt)\n\n{PROMPT_BOX}"
        )
    }

    /// The Conductor's question: with no pending-idle hold, does a working
    /// agent's screen read idle for a moment between spinner frames? Every
    /// chunk is read, more often than the 16 ms writer would, and the
    /// facts a writer would send (each read that differs from the last)
    /// are working through every frame and idle only at the end.
    #[test]
    fn a_working_claude_code_reads_working_across_every_spinner_frame_and_idle_at_its_end() {
        let pane = pane();
        let rules = rules_given(Some(claude_code_rules()));
        let mut reading = Reading::default();
        let mut reads = Vec::new();
        let titles = [
            '⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏', '◐', '◓', '◑', '◒',
        ];
        let screen_spinners = ['·', '✢', '✳', '✶', '✻', '✽'];
        for (frame, title) in titles.iter().enumerate() {
            feed(
                &pane,
                format!("\x1b]0;{title} Read the parser\x07").as_bytes(),
            );
            reads.push(read(&pane, &rules, &mut reading));
            let body = working_body(screen_spinners[frame % screen_spinners.len()], frame as u32);
            let drawn = redraw(&body);
            // The redraw split as the PTY may hand it over: cleared, half
            // drawn, then whole.
            let (cleared, rest) = drawn.split_at(b"\x1b[?2026h\x1b[H\x1b[2J".len());
            let (half, end) = rest.split_at(rest.len() / 2);
            for chunk in [cleared, half, end] {
                feed(&pane, chunk);
                reads.push(read(&pane, &rules, &mut reading));
            }
            // A frame that redraws only the prompt box, the spinner line
            // erased for the moment.
            feed(&pane, &redraw(PROMPT_BOX));
            reads.push(read(&pane, &rules, &mut reading));
        }
        for (index, read) in reads.iter().enumerate() {
            assert!(
                matches!(
                    read,
                    ScreenRead::Read {
                        state: ScreenState::Working,
                        ..
                    }
                ),
                "read {index} of the working frames: {read:?}"
            );
        }
        feed(&pane, "\x1b]0;\u{2733} Read the parser\x07".as_bytes());
        feed(
            &pane,
            &redraw(&format!(
                "● I'll read the parser first.\n\n● The parser keeps its own title.\n\n{PROMPT_BOX}"
            )),
        );
        reads.push(read(&pane, &rules, &mut reading));
        let mut sent: Vec<&ScreenRead> = Vec::new();
        for read in &reads {
            if sent.last() != Some(&read) {
                sent.push(read);
            }
        }
        assert_eq!(
            sent,
            vec![
                &working("osc_title_working"),
                &ScreenRead::Read {
                    state: ScreenState::Idle,
                    rule: Some("live_prompt_box".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn a_rule_that_proves_nothing_keeps_the_state_and_new_rules_start_afresh() {
        let manifest = |version: &str| {
            format!(
                r#"{{"version":"{version}","min_engine_version":1,"rules":[
                {{"id":"busy","state":"working","priority":2,"contains":["busy"]}},
                {{"id":"viewer","state":"unknown","priority":3,"contains":["viewer"],"skip_state_update":true}}]}}"#
            )
        };
        let pane = pane();
        let rules = rules_given(Some(manifest("1")));
        let mut reading = Reading::default();
        feed(&pane, b"busy\r\n");
        assert_eq!(read(&pane, &rules, &mut reading), working("busy"));
        feed(&pane, b"viewer\r\n");
        assert_eq!(read(&pane, &rules, &mut reading), working("busy"));
        assert_eq!(set_rules(&rules, 0, Some(manifest("2"))), Some(true));
        assert_eq!(
            read(&pane, &rules, &mut reading),
            ScreenRead::Read {
                state: ScreenState::Unknown,
                rule: None
            }
        );
        feed(&pane, b"\x1b[2J\x1b[Hnothing\r\n");
        assert_eq!(
            read(&pane, &rules, &mut reading),
            ScreenRead::Read {
                state: ScreenState::Unknown,
                rule: None
            }
        );
    }

    #[test]
    fn rules_that_do_not_compile_read_unknown_with_why_and_none_read_no_rules() {
        let pane = pane();
        let mut reading = Reading::default();
        let rules = rules_given(Some(
            r#"{"version":"1","min_engine_version":99,"rules":[]}"#.to_owned(),
        ));
        assert_eq!(
            read(&pane, &rules, &mut reading),
            ScreenRead::Unknown {
                reason: "rules require engine 99, this one is 3".to_owned()
            }
        );
        let rules = rules_given(Some("not json".to_owned()));
        assert!(matches!(
            read(&pane, &rules, &mut reading),
            ScreenRead::Unknown { reason } if reason.starts_with("expected ")
        ));
        assert_eq!(
            read(&pane, &SharedRules::default(), &mut reading),
            ScreenRead::NoRules
        );
    }

    #[test]
    fn rules_given_again_or_by_a_replaced_watch_change_nothing() {
        let rules = SharedRules::default();
        let json = Some(claude_code_rules());
        assert_eq!(set_rules(&rules, 0, json.clone()), Some(false));
        assert_eq!(set_rules(&rules, 0, json.clone()), None);
        slot(&rules).owner = 1;
        assert_eq!(set_rules(&rules, 0, None), None);
        assert_eq!(set_rules(&rules, 1, None), Some(true));
        assert_eq!(set_rules(&rules, 1, json), Some(false));
    }

    #[test]
    fn the_facts_say_the_folder_title_bells_prompts_and_bracketed_paste() {
        let pane = pane();
        let rules = SharedRules::default();
        let mut reading = Reading::default();
        let folder = if cfg!(windows) {
            "C:/work/a%20b"
        } else {
            "/work/a%20b"
        };
        feed(
            &pane,
            format!(
                "\x1b]7;file://localhost{}{folder}\x07\x1b]2;job\x07\x07\x07\x1b]133;A\x07\x1b[?2004h",
                if cfg!(windows) { "/" } else { "" }
            )
            .as_bytes(),
        );
        let facts = facts(&pane, &rules, &mut reading, None).unwrap();
        let expected = if cfg!(windows) {
            "C:\\work\\a b"
        } else {
            "/work/a b"
        };
        assert_eq!(facts.cwd.as_deref(), Some(expected));
        assert_eq!(facts.title.as_deref(), Some("job"));
        assert_eq!((facts.bells, facts.prompts), (2, 1));
        assert!(facts.bracketed_paste);
        assert_eq!(facts.screen, ScreenRead::NoRules);
        feed(&pane, b"\x1b]7;relative/folder\x07\x1b[?2004l");
        let facts = super::facts(&pane, &rules, &mut reading, None).unwrap();
        assert_eq!(facts.cwd.as_deref(), Some(expected));
        assert!(!facts.bracketed_paste);
    }
}
