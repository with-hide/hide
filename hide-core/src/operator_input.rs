//! What the operator types into an agent's pane from a screen that is not
//! its terminal: the one rule the words pass and the one write that types
//! them, shared by the desktop conversation's input box and a phone's reply
//! and keys, so the two never drift apart (docs/ARCHITECTURE.md, The mobile
//! companion).
//!
//! The write waits for the pane node's input permit (the creation holder's
//! byte reservation, so a fork's caps count every surface's input together),
//! takes an admission from the core's input gate (a pane whose agent ends,
//! sleeps or wakes takes none), asks its caller last for the engine to type
//! with (the caller's last look that the pane is still the one it meant, and
//! the engine of the pane's machine), and only then types: words through the
//! engine's `prompt_agent`, as the operator's (one paste where the program
//! takes bracketed paste, then Enter after the prompt's gap), keys one by
//! one as the engine's key input. One key write runs in a pane at a time,
//! and a choice's keys are typed only while the visible screen shows what
//! the operator answered ([`Screen`]): before the first key, the ask's rows
//! and the command a permission was shown with, so an Esc never reaches a
//! command already running; and before each key that counts from the
//! highlight, the highlight on the row it expects, the pick included: a
//! highlight moved in the terminal would make it pick another row, such as
//! allow for refuse.

use std::sync::Arc;
use std::time::Duration;

use hide_node_link::terminal::{InputPermitError, TerminalNode, wait_for_input_permit};

use crate::engine::{EngineError, EngineHandle};
use crate::input_gate::PaneInputGate;

/// The longest text one input carries, in bytes, under the node's 64 KiB
/// held-input cap with room for attached paths.
pub const LIMIT_BYTES: usize = 16 * 1024;

/// The most files one input attaches.
pub const MAX_FILES: usize = crate::terminal_attachments::MAX_FILES;

/// How long each engine step of a write may wait.
const STEP_TIMEOUT: Duration = Duration::from_secs(5);

/// The bytes a permit reserves for one key: more than any key the engine
/// encodes takes in any keyboard mode the program may have asked for.
const KEY_BYTES: usize = 16;

/// The pause between two keys of one answer, so a program that reads its
/// input in chunks sees each key as its own press.
const KEY_GAP: Duration = Duration::from_millis(80);

/// Why a choice's keys stopped before picking: the screen does not show the
/// command the operator was shown, or the highlight on the row the next key
/// expects.
pub const SCREEN_DIFFERS: &str = "screen_differs";

/// Why a choice typed nothing: the screen shows no row of the ask, which the
/// operator answered already, or whose allowed command runs.
pub const ASK_NOT_SHOWN: &str = "ask_not_shown";

/// Why a key write typed nothing: another key write to the pane is under way.
pub const IN_FLIGHT: &str = "in_flight";

/// How many times a key reads the screen for the row it expects after a
/// down this write typed, [`KEY_GAP`] apart, so a program still drawing the
/// move is not read as a moved highlight.
const ROW_READS: usize = 6;

/// What may stand before a highlighted row on a line: a box's border.
const BORDER: [char; 4] = [' ', '│', '┃', '║'];

/// Why words are refused before anything reaches the pane: nothing to send,
/// past [`LIMIT_BYTES`], too many files, or a control character other than a
/// new line or a tab. A new line stays inside the engine's bracketed paste and
/// submits nothing; any other control character could act on the agent's
/// screen.
pub fn problem(text: &str, files: usize) -> Option<&'static str> {
    if text.trim().is_empty() && files == 0 {
        return Some("empty");
    }
    if text.len() > LIMIT_BYTES {
        return Some("too_long");
    }
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
    {
        return Some("control_characters");
    }
    if files > MAX_FILES {
        return Some("too_many_files");
    }
    None
}

/// Where the files an input attaches are before its words are typed: the
/// files a screen staged on the core's machine, and, for a pane of another
/// machine, the link they are staged on there (`Call::StageAttachment`), so
/// the desktop conversation's input and a phone's reply attach files one way.
/// Taken under the runtime lock and used off it: [`Attach::words`] calls the
/// nodes.
pub struct Attach {
    on_device: bool,
    own_node: Arc<dyn crate::node_access::NodeLink>,
    staging: Option<Result<Arc<dyn crate::node_access::NodeLink>, String>>,
}

impl Attach {
    pub(crate) fn new(
        on_device: bool,
        own_node: Arc<dyn crate::node_access::NodeLink>,
        staging: Option<Result<Arc<dyn crate::node_access::NodeLink>, String>>,
    ) -> Self {
        Self {
            on_device,
            own_node,
            staging,
        }
    }

    /// The words that type `paths`, files staged on the core's machine, in
    /// front of `text`: each path quoted for the shell, in the order given,
    /// as the machine that holds the pane spells it. For a pane of another
    /// machine the files are read here and staged there first, under
    /// `request_id`. Blocks on the nodes, so a caller runs it off the
    /// runtime lock.
    pub fn words(
        &self,
        text: &str,
        paths: &[String],
        request_id: &str,
    ) -> Result<String, &'static str> {
        if paths.is_empty() {
            return Ok(text.to_owned());
        }
        let destinations = if self.on_device {
            let link = self
                .staging
                .as_ref()
                .ok_or("device_unreachable")?
                .as_ref()
                .map_err(|_| "device_unreachable")?;
            let cancelled = std::sync::atomic::AtomicBool::new(false);
            let files = crate::terminal_attachments::read_sources(
                self.own_node.as_ref(),
                paths,
                &cancelled,
            )
            .map_err(|_| "attachment_failed")?;
            crate::terminal_attachments::stage(link.as_ref(), request_id, &files, &cancelled)
                .map_err(|_| "attachment_failed")?
        } else {
            paths.to_vec()
        };
        let quoted = crate::terminal_attachments::paste_bytes(&destinations, false, self.on_device)
            .map_err(|_| "attachment_failed")?;
        let mut words = String::from_utf8(quoted).map_err(|_| "attachment_failed")?;
        words.push_str(text);
        Ok(words)
    }
}

/// What one write types.
pub enum Typed<'a> {
    /// The operator's words, submitted with Enter.
    Words(&'a str),
    /// Keys in order, each an engine key name (`down`, `enter`, `esc`) or
    /// one character typed as a key press, never pasted.
    Keys(&'a [&'a str]),
    /// A choice's keys, each typed only while the screen shows what `screen`
    /// expects there.
    Choice {
        keys: &'a [&'a str],
        screen: &'a Screen,
    },
}

impl Typed<'_> {
    /// The bytes the node's permit reserves: the words and their Enter (the
    /// bracketed-paste wrapper is protocol framing, outside the budget), or
    /// [`KEY_BYTES`] a key.
    fn reserved(&self) -> usize {
        match self {
            Self::Words(text) => text.len() + 1,
            Self::Keys(keys) | Self::Choice { keys, .. } => keys.len() * KEY_BYTES,
        }
    }
}

/// Why a write typed nothing, or may have typed something.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Unwritten {
    /// The pane's agent is ending, asleep or waking.
    Asleep,
    /// The node or the engine refused the input before typing anything.
    Refused,
    /// The pane is gone.
    Gone,
    /// The engine could not be reached; nothing was typed.
    Unavailable,
    /// The engine never answered, or answered past its contract: the input
    /// may have landed.
    Uncertain,
    /// The caller's last step refused, for the reason it named: the pane
    /// moved on, or its machine's engine could not be had; nothing was typed.
    Unready(&'static str),
}

impl Unwritten {
    /// The reason every surface names the outcome by.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Asleep => "asleep",
            Self::Refused => "input_refused",
            Self::Gone => "pane_gone",
            Self::Unavailable => "unavailable",
            Self::Uncertain => "uncertain",
            Self::Unready(reason) => reason,
        }
    }

    /// Whether nothing reached the pane, so the same input may be sent again.
    pub fn certain(self) -> bool {
        !matches!(self, Self::Uncertain)
    }

    fn of(error: &EngineError) -> Self {
        match error {
            EngineError::Refused { code, .. } if code.contains("not_found") => Self::Gone,
            EngineError::Refused { .. } => Self::Refused,
            EngineError::NotRunning(_) | EngineError::Transport(_) | EngineError::Request(_) => {
                Self::Unavailable
            }
            EngineError::Unanswered(_) | EngineError::Malformed(_) | EngineError::Answer(_) => {
                Self::Uncertain
            }
        }
    }
}

/// Types `typed` into `raw_pane`, the engine's id of the pane the core knows
/// as `core_pane`, after the node's permit and the gate's admission, with the
/// engine `ready` answers last: it looks once more that the pane is still the
/// one meant, and nothing is typed when it refuses. Blocks, so a caller runs
/// it off the runtime lock and off any async loop.
pub fn write(
    terminals: &dyn TerminalNode,
    gate: &Arc<PaneInputGate>,
    core_pane: &str,
    raw_pane: &str,
    typed: Typed<'_>,
    ready: &dyn Fn() -> Result<EngineHandle, &'static str>,
) -> Result<(), Unwritten> {
    match wait_for_input_permit(terminals, core_pane, typed.reserved()) {
        Ok(()) => {}
        Err(InputPermitError::Asleep) => return Err(Unwritten::Asleep),
        Err(_) => return Err(Unwritten::Refused),
    }
    // Held until the write returns: a sleep that starts now waits for these
    // words to land, and a pane closed since refuses them.
    let _admission = gate.admit(core_pane).map_err(|_| Unwritten::Asleep)?;
    // One key write in a pane at a time, so two screens' keys never
    // interleave between a row read and the key after it.
    let _keys = match typed {
        Typed::Words(_) => None,
        Typed::Keys(_) | Typed::Choice { .. } => Some(
            gate.hold_keys(core_pane)
                .ok_or(Unwritten::Unready(IN_FLIGHT))?,
        ),
    };
    let engine = ready().map_err(Unwritten::Unready)?;
    match typed {
        Typed::Words(text) => engine
            .prompt_agent(raw_pane, text, STEP_TIMEOUT)
            .map_err(|error| Unwritten::of(&error)),
        Typed::Keys(keys) => press(&engine, raw_pane, keys, None),
        Typed::Choice { keys, screen } => press(&engine, raw_pane, keys, Some(screen)),
    }
}

/// Presses `keys` one by one in `raw_pane`; with `screen`, the first only
/// once the screen shows the ask's rows and the command it expects, and each
/// key that counts from the highlight only once the screen shows it on the
/// row that key expects, the first on the first row and each after a down
/// one row lower.
fn press(
    engine: &EngineHandle,
    raw_pane: &str,
    keys: &[&str],
    screen: Option<&Screen>,
) -> Result<(), Unwritten> {
    let mut place = 0;
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            // A production wait: the program reads the next key as its own
            // press only once it has read this one.
            #[allow(clippy::disallowed_methods)]
            std::thread::sleep(KEY_GAP);
        }
        if let Some(screen) = screen {
            let row = counts_from_highlight(key).then_some(place);
            if row.is_some() || (index == 0 && *screen != Screen::default()) {
                await_screen(engine, raw_pane, screen, row, index > 0)?;
            }
        }
        if *key == "down" {
            place += 1;
        }
        if let Err(error) = engine.send_keys(raw_pane, &[key], STEP_TIMEOUT) {
            // A key after the first lands on a form the first moved.
            let unwritten = Unwritten::of(&error);
            return Err(if index > 0 && unwritten.certain() {
                Unwritten::Uncertain
            } else {
                unwritten
            });
        }
    }
    Ok(())
}

/// Reads the visible screen until it shows what `screen` expects, with the
/// highlight on row `place` when given: once, or, after a key this write
/// typed, up to [`ROW_READS`] times while the program draws the move. Any
/// other screen stops the write before this key, so nothing is picked.
fn await_screen(
    engine: &EngineHandle,
    raw_pane: &str,
    screen: &Screen,
    place: Option<usize>,
    after_key: bool,
) -> Result<(), Unwritten> {
    let reads = if after_key { ROW_READS } else { 1 };
    let mut visible = String::new();
    for read in 0..reads {
        if read > 0 {
            // A production wait: the program has not drawn the last key yet.
            #[allow(clippy::disallowed_methods)]
            std::thread::sleep(KEY_GAP);
        }
        // A failed read typed nothing, whatever the engine answered.
        visible =
            engine
                .read_visible(raw_pane, STEP_TIMEOUT)
                .map_err(|error| match Unwritten::of(&error) {
                    Unwritten::Uncertain => Unwritten::Unavailable,
                    certain => certain,
                })?;
        if screen.shows(&visible, place) {
            return Ok(());
        }
    }
    // Before any key, a screen with none of the ask's rows has left the ask.
    let left = !after_key
        && screen
            .rows
            .as_ref()
            .is_some_and(|rows| !rows.any_highlighted(&visible));
    Err(Unwritten::Unready(if left {
        ASK_NOT_SHOWN
    } else {
        SCREEN_DIFFERS
    }))
}

/// An ask on the agent's own screen that a choice answers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ask {
    /// The agent's question of one choice, with how many choices it lists.
    Question(usize),
    PlanApproval,
    /// A tool call waiting for the operator's permission.
    Permission,
}

impl Ask {
    pub fn name(self) -> &'static str {
        match self {
            Self::Question(_) => "question",
            Self::PlanApproval => "plan_approval",
            Self::Permission => "permission",
        }
    }
}

/// The choices the agent's key table lets a screen that is not its terminal
/// pick on `ask`, in the order its screen lists them: a question's by place
/// (`1` for the first), a plan's `approve` and `keep_planning`, a permission's
/// `allow`, `allow_always` where the agent offers it, and `deny`. Empty for
/// an agent, or a kind of ask, that has no table (`[answer]` in its
/// definition file), or whose keys count from the highlighted row on a
/// screen whose highlight the table cannot read: it is answered in the
/// terminal.
pub fn choices(agent_kind: &str, ask: Ask) -> Vec<String> {
    let Some(answer) = answer_table(agent_kind) else {
        return Vec::new();
    };
    let offered = offered(answer, ask);
    // Keys that count from the highlight are typed only where the screen
    // shows where they start; without that, the ask is answered in the
    // terminal.
    let blind = row_of(answer, ask).is_none()
        && offered.iter().any(|choice| {
            keys_of(answer, ask, choice)
                .is_some_and(|keys| keys.iter().any(|key| counts_from_highlight(key)))
        });
    if blind { Vec::new() } else { offered }
}

/// The choices `answer` has keys for on `ask`.
fn offered(answer: &hide_agent_adapter::definition::Answer, ask: Ask) -> Vec<String> {
    match ask {
        Ask::Question(count) if answer.question.is_some() => (1..=count.min(QUESTION_CHOICES))
            .map(|at| at.to_string())
            .collect(),
        Ask::PlanApproval if answer.plan_approval.is_some() => {
            vec!["approve".to_owned(), "keep_planning".to_owned()]
        }
        Ask::Permission => match &answer.permission {
            Some(permission) => std::iter::once("allow")
                .chain(permission.allow_always.as_ref().map(|_| "allow_always"))
                .chain(["deny"])
                .map(str::to_owned)
                .collect(),
            None => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// The keys that pick `choice` on `ask`, from the agent's key table; `None`
/// for a choice [`choices`] does not offer.
pub fn choice_keys(agent_kind: &str, ask: Ask, choice: &str) -> Option<Vec<String>> {
    if !choices(agent_kind, ask)
        .iter()
        .any(|offered| offered == choice)
    {
        return None;
    }
    keys_of(answer_table(agent_kind)?, ask, choice)
}

/// The keys `answer` types for `choice` on `ask`.
fn keys_of(
    answer: &hide_agent_adapter::definition::Answer,
    ask: Ask,
    choice: &str,
) -> Option<Vec<String>> {
    Some(match ask {
        Ask::Question(_) => {
            let keys = answer.question.as_ref()?;
            let place: usize = choice.parse().ok()?;
            std::iter::repeat_n(keys.down.clone(), place - 1)
                .chain([keys.pick.clone()])
                .collect()
        }
        Ask::PlanApproval => {
            let keys = answer.plan_approval.as_ref()?;
            match choice {
                "approve" => keys.approve.clone(),
                _ => keys.keep_planning.clone(),
            }
        }
        Ask::Permission => {
            let keys = answer.permission.as_ref()?;
            match choice {
                "allow" => keys.allow.clone(),
                "allow_always" => keys.allow_always.clone()?,
                _ => keys.deny.clone(),
            }
        }
    })
}

/// What the agent's screen must show while a choice's keys are typed: the
/// ask's rows and the command the operator was shown, before the first key,
/// and the highlighted row, for a key that counts from it.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Screen {
    /// How the screen draws the ask's highlighted row; `None` where the
    /// agent's table does not say.
    rows: Option<Rows>,
    /// The start of the command a permission was shown with: the ask on the
    /// screen is that command's.
    command: Option<String>,
}

/// A highlighted row as the agent's screen draws it: its table's `marker`
/// before the text its `row` names, the row's place (`{n}.`) or its
/// question's choice label.
#[derive(Clone, Debug, Eq, PartialEq)]
struct Rows {
    marker: String,
    row: String,
    /// The question's choices, for a `row` that names a row by its label.
    labels: Vec<String>,
}

/// How much of a command's first line the screen must show: short enough
/// to sit on one line of any pane a dialog fits in.
const COMMAND_PROBE_CHARS: usize = 40;

/// The places a numbered row is looked for at, when any row of the ask
/// will do: more than any ask an agent's screen lists.
const ROW_PLACES: usize = 16;

impl Screen {
    /// This screen, which also shows the start of `command`, the one the
    /// operator was shown.
    #[must_use]
    pub fn showing(mut self, command: &str) -> Self {
        let first = command.trim().lines().next().unwrap_or_default().trim();
        self.command = Some(first.chars().take(COMMAND_PROBE_CHARS).collect());
        self
    }

    /// Whether `visible`, a pane's visible text, shows what this screen
    /// expects: the highlight on row `place` when given, else on any row of
    /// the ask, where its table says how rows are drawn.
    pub fn shows(&self, visible: &str, place: Option<usize>) -> bool {
        let command = self
            .command
            .as_deref()
            .is_none_or(|command| visible.contains(command));
        let row = match (&self.rows, place) {
            (Some(rows), Some(place)) => rows.highlighted(visible, place),
            (None, Some(_)) => false,
            (Some(rows), None) => rows.any_highlighted(visible),
            (None, None) => true,
        };
        command && row
    }
}

impl Rows {
    /// Whether `screen` shows the highlight on any row of the ask.
    fn any_highlighted(&self, screen: &str) -> bool {
        let labelled = self.row.contains(hide_agent_adapter::definition::ROW_LABEL);
        let places = if labelled {
            self.labels.len()
        } else {
            ROW_PLACES
        };
        (0..places).any(|place| self.highlighted(screen, place))
    }

    /// Whether `screen`, a pane's visible text, shows the highlight on row
    /// `place` (0 for the first): the lowest line that begins with the
    /// marker, past a box's border, is that row. A screen with no
    /// highlighted row, or another one, does not.
    fn highlighted(&self, screen: &str, place: usize) -> bool {
        use hide_agent_adapter::definition::{ROW_LABEL, ROW_NUMBER};
        let whole = self.row.contains(ROW_LABEL);
        let expected = if whole {
            let Some(label) = self.labels.get(place) else {
                return false;
            };
            self.row.replace(ROW_LABEL, label)
        } else {
            self.row.replace(ROW_NUMBER, &(place + 1).to_string())
        };
        let Some(row) = screen.lines().rev().find_map(|line| {
            line.trim_start_matches(BORDER)
                .strip_prefix(self.marker.as_str())
        }) else {
            return false;
        };
        let Some(rest) = row
            .trim_start()
            .strip_prefix(expected.as_str())
            .map(|rest| rest.trim_end_matches(BORDER))
        else {
            return false;
        };
        // A label is the row's whole text; a place starts it.
        if whole {
            rest.is_empty()
        } else {
            rest.is_empty() || rest.starts_with(char::is_whitespace)
        }
    }
}

/// What the screen must show while the keys of a choice on `ask` are typed:
/// the ask's rows, as its agent's table draws them, which an Esc or a letter
/// waits for too, so it never lands on a screen the ask has left. `labels`
/// are a question's choices, which a table may name its rows by.
pub fn choice_screen(agent_kind: &str, ask: Ask, labels: &[String]) -> Screen {
    let rows = answer_table(agent_kind).and_then(|answer| {
        Some(Rows {
            marker: answer.marker.clone()?,
            row: row_of(answer, ask)?.to_owned(),
            labels: labels.to_vec(),
        })
    });
    Screen {
        rows,
        command: None,
    }
}

/// Whether the agent's permission names no command, since it records a call
/// only once it is allowed (`unnamed` in its key table).
pub fn unnamed_permission(agent_kind: &str) -> bool {
    answer_table(agent_kind)
        .and_then(|answer| answer.permission.as_ref())
        .is_some_and(|permission| permission.unnamed)
}

/// Whether `key` acts on the highlighted row, or moves it: every key but
/// `esc` and a character.
fn counts_from_highlight(key: &str) -> bool {
    key != "esc" && key.chars().count() != 1
}

/// The text `ask`'s rows are drawn with, from its table.
fn row_of(answer: &hide_agent_adapter::definition::Answer, ask: Ask) -> Option<&str> {
    answer.marker.as_ref()?;
    match ask {
        Ask::Question(_) => answer.question.as_ref()?.row.as_deref(),
        Ask::PlanApproval => answer.plan_approval.as_ref()?.row.as_deref(),
        Ask::Permission => answer.permission.as_ref()?.row.as_deref(),
    }
}

/// The most choices of a question picked by place: as many as a question's
/// record keeps (`hide_session::turns::CHOICE_LIMIT`), each reached with at
/// most that many keys.
const QUESTION_CHOICES: usize = hide_session::turns::CHOICE_LIMIT;

/// The key table of the built-in agent `agent_kind` names (its id or an
/// alias): the tables are checked on each agent's screens before they ship,
/// so an operator's own file carries none.
fn answer_table(agent_kind: &str) -> Option<&'static hide_agent_adapter::definition::Answer> {
    hide_agent_adapter::definition::find(hide_agent_adapter::definition::builtins(), agent_kind)?
        .answer
        .as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_are_refused_before_the_pane_for_what_they_could_do_there() {
        assert_eq!(problem("  \n", 0), Some("empty"));
        assert_eq!(problem("", 1), None);
        assert_eq!(problem("멈추고\n다시 해줘\t끝", 0), None);
        assert_eq!(problem("a\u{1b}[201~b", 0), Some("control_characters"));
        assert_eq!(problem("a\rb", 0), Some("control_characters"));
        assert_eq!(problem(&"a".repeat(LIMIT_BYTES), 0), None);
        assert_eq!(problem(&"a".repeat(LIMIT_BYTES + 1), 0), Some("too_long"));
        assert_eq!(
            problem(&"가".repeat(LIMIT_BYTES / 3 + 1), 0),
            Some("too_long")
        );
        assert_eq!(problem("files", MAX_FILES + 1), Some("too_many_files"));
    }

    use crate::engine::fake::{FakeEngine, reply};
    use hide_node_link::terminal::{GridSize, InputPermitReply, KeyTarget, TerminalControl};
    use std::sync::Mutex;

    /// A node that answers every permit as `allow` says and records the
    /// bytes asked for; anything else a writer did to the terminal would be
    /// a phone or a conversation moving the operator's screen.
    struct Node {
        allow: Option<InputPermitError>,
        asked: Mutex<Vec<usize>>,
    }

    impl TerminalNode for Node {
        fn input_permit(&self, _: &str, bytes: usize, reply: InputPermitReply) {
            self.asked.lock().unwrap().push(bytes);
            reply(self.allow.map_or(Ok(()), Err));
        }
        fn control(&self, _: TerminalControl) {
            panic!("an operator write must not attach or focus");
        }
        fn key(&self, _: KeyTarget, _: Vec<u8>, _: u64) {
            panic!("an operator write goes to the engine, not the key path");
        }
        fn view(&self, _: &str, _: GridSize, _: bool) {
            panic!("an operator write must not resize");
        }
        fn redraw(&self, _: &str) {
            panic!("an operator write must not redraw");
        }
    }

    fn node(allow: Option<InputPermitError>) -> Node {
        Node {
            allow,
            asked: Mutex::default(),
        }
    }

    /// Words of several lines reach the engine whole, as the operator's
    /// prompt, after a permit for them and their Enter; keys go one by one,
    /// each as a key press. A refused permit, a closed pane and a last look
    /// that says the pane moved on type nothing.
    #[test]
    fn a_write_types_only_after_the_permit_the_gate_and_the_last_look() {
        let fake = FakeEngine::start(|_, _| reply(()));
        let engine = EngineHandle::new(fake.engine());
        let ready = || Ok(engine.clone());
        let gate = Arc::new(PaneInputGate::default());
        let open = node(None);
        let words = "first line\nsecond\tline";
        let written = write(&open, &gate, "w1:p1", "p1", Typed::Words(words), &ready);
        assert_eq!(written, Ok(()));
        assert_eq!(*open.asked.lock().unwrap(), [words.len() + 1]);
        assert_eq!(
            fake.calls_of("prompt_agent"),
            [serde_json::json!({"target": "p1", "text": words, "by": "Operator"})]
        );
        let keys = ["down", "enter"];
        assert_eq!(
            write(&open, &gate, "w1:p1", "p1", Typed::Keys(&keys), &ready),
            Ok(())
        );
        assert_eq!(
            fake.calls_of("send_keys"),
            [
                serde_json::json!({"pane": "p1", "keys": ["down"]}),
                serde_json::json!({"pane": "p1", "keys": ["enter"]}),
            ]
        );

        let refused = node(Some(InputPermitError::Discarded));
        let asleep = node(Some(InputPermitError::Asleep));
        let moved = || Err("stale");
        assert_eq!(
            write(&refused, &gate, "w1:p1", "p1", Typed::Words("yes"), &ready),
            Err(Unwritten::Refused)
        );
        assert_eq!(
            write(&asleep, &gate, "w1:p1", "p1", Typed::Words("yes"), &ready),
            Err(Unwritten::Asleep)
        );
        assert_eq!(
            write(&open, &gate, "w1:p1", "p1", Typed::Keys(&keys), &moved),
            Err(Unwritten::Unready("stale"))
        );
        gate.close("w1:p1");
        assert_eq!(
            write(&open, &gate, "w1:p1", "p1", Typed::Words("yes"), &ready),
            Err(Unwritten::Asleep)
        );
        assert_eq!(fake.methods(), ["prompt_agent", "send_keys", "send_keys"]);
    }

    /// A choice is typed as the keys the agent's own screen takes for it,
    /// as checked in its real pane: a question's place as that many moves
    /// down and a pick, an approval's choice as its table's keys; an agent or
    /// a kind of ask without a table offers nothing and types nothing.
    #[test]
    fn a_choice_is_the_keys_its_agents_screen_takes_and_nothing_without_a_table() {
        assert_eq!(choices("claude", Ask::Question(3)), ["1", "2", "3"]);
        assert_eq!(
            choice_keys("claude", Ask::Question(3), "3"),
            Some(vec![
                "down".to_owned(),
                "down".to_owned(),
                "enter".to_owned()
            ])
        );
        assert_eq!(choice_keys("claude", Ask::Question(3), "4"), None);
        assert_eq!(choice_keys("claude", Ask::Question(3), "0"), None);
        assert_eq!(
            choices("codex", Ask::Permission),
            ["allow", "allow_always", "deny"]
        );
        assert_eq!(
            choice_keys("codex", Ask::Permission, "allow_always"),
            Some(vec!["down".to_owned(), "enter".to_owned()])
        );
        assert_eq!(choices("omp", Ask::Question(2)), ["1", "2"]);
        // omp's plan and permission rows are fixed words with no place to
        // count by.
        assert!(choices("omp", Ask::Permission).is_empty());
        assert!(choices("omp", Ask::PlanApproval).is_empty());
        assert_eq!(
            choice_keys("grok", Ask::PlanApproval, "approve"),
            Some(vec!["a".to_owned()])
        );
        assert!(choices("grok", Ask::Permission).is_empty());
        // Their keys count from a highlight shown by colour alone.
        assert!(choices("opencode", Ask::Permission).is_empty());
        assert!(choices("opencode", Ask::Question(2)).is_empty());
        assert!(choices("grok", Ask::Question(2)).is_empty());
        assert!(choices("pi", Ask::Question(2)).is_empty());
        assert!(choices("not-an-agent", Ask::Permission).is_empty());
    }

    /// Every key a built-in table names is one the core's own engine
    /// presses, so a choice never fails at the pane for its spelling.
    #[test]
    fn every_key_in_a_built_in_table_is_one_the_engine_types() {
        for definition in hide_agent_adapter::definition::builtins() {
            for ask in [
                Ask::Question(QUESTION_CHOICES),
                Ask::PlanApproval,
                Ask::Permission,
            ] {
                for choice in choices(&definition.id, ask) {
                    let keys = choice_keys(&definition.id, ask, &choice).unwrap();
                    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
                    assert!(
                        crate::engine::own::types_keys(&keys),
                        "{} {} {choice}: {keys:?}",
                        definition.id,
                        ask.name()
                    );
                }
            }
        }
    }

    /// Rows as the agents drew them in real panes (live-checks, 2026-11-23):
    /// a row is read where the highlight stands on it, and not where it
    /// stands on another, such as allow-always for allow.
    #[test]
    fn a_row_is_read_only_where_the_agents_screen_highlights_it() {
        struct Rows(Screen);
        impl Rows {
            fn highlighted(&self, visible: &str, place: usize) -> bool {
                self.0.shows(visible, Some(place))
            }
        }
        let rows = |agent: &str, ask: Ask, labels: &[&str]| {
            let labels: Vec<String> = labels.iter().map(|label| (*label).to_owned()).collect();
            Rows(choice_screen(agent, ask, &labels))
        };
        let claude = rows("claude", Ask::Permission, &[]);
        let first = [
            " Do you want to proceed?",
            " ❯ 1. Yes",
            "   2. Yes, and always allow access to /private/tmp/hlc/proj from this project",
            "   4. No",
        ]
        .join("\n");
        let second = [
            " Do you want to proceed?",
            "   1. Yes",
            " ❯ 2. Yes, and always allow access to /private/tmp/hlc/proj from this project",
            "   4. No",
        ]
        .join("\n");
        assert!(claude.highlighted(&first, 0));
        assert!(!claude.highlighted(&second, 0));
        assert!(claude.highlighted(&second, 1));
        assert!(!claude.highlighted(&first, 1));
        assert!(!claude.highlighted(" Do you want to proceed?\n", 0));
        let codex = rows("codex", Ask::Question(4), &[]);
        assert!(codex.highlighted(
            "  Which color do you prefer?\n  › 1. Red (Recommended)  Choose red.\n    2. Green",
            0
        ));
        // A tenth row is not the first.
        assert!(!codex.highlighted("› 10. Other", 0));
        let antigravity = rows("antigravity", Ask::Permission, &[]);
        assert!(antigravity.highlighted("  1. Yes, run command\n> 4. No, cancel", 3));
        // omp numbers no rows: a question's rows are its choices, whole.
        let omp = rows("omp", Ask::Question(3), &["Red", "Green", "Blue"]);
        let red = "│ ❯ ○ Red                                     │\n│   ○ Green │";
        assert!(omp.highlighted(red, 0));
        assert!(!omp.highlighted(red, 1));
        assert!(omp.highlighted("│   ○ Red   │\n│ ❯ ○ Green │", 1));
        assert!(!rows("omp", Ask::Question(3), &["Re"]).highlighted(red, 0));
        // Esc waits for any row of the ask, wherever the highlight stands;
        // Grok's letters have no rows to wait for.
        let deny = choice_screen("claude", Ask::Permission, &[]);
        assert!(deny.shows(&second, None));
        assert!(!deny.shows("  ⎿  $ python3 -c \"import time\"\n\n ❯ \n", None));
        assert_eq!(
            choice_screen("grok", Ask::PlanApproval, &[]),
            Screen::default()
        );
    }

    /// An engine whose screen is Antigravity's permission for `cargo test
    /// --workspace` with the highlight on row `at`, which each down this
    /// write types moves one row, and the first down `jump` rows more, as a
    /// key typed in the terminal between two keys would.
    fn menu(at: usize, jump: usize) -> Arc<FakeEngine<'static>> {
        let at = Arc::new(Mutex::new((at, jump)));
        FakeEngine::start(move |method, params| {
            let mut state = at.lock().unwrap();
            match method {
                "read_pane" => {
                    let rows = [
                        "Yes, run command",
                        "Yes, and always allow",
                        "Yes, always",
                        "No, cancel",
                    ];
                    let text = std::iter::once("  cargo test --workspace".to_owned())
                        .chain(rows.iter().enumerate().map(|(row, words)| {
                            let mark = if row == state.0 { ">" } else { " " };
                            format!("{mark} {}. {words}", row + 1)
                        }))
                        .collect::<Vec<_>>()
                        .join("\n");
                    reply(crate::engine::PaneText {
                        text,
                        truncated: false,
                    })
                }
                "send_keys" => {
                    if params["keys"][0] == "down" {
                        let other = std::mem::take(&mut state.1);
                        state.0 += 1 + other;
                    }
                    reply(())
                }
                _ => reply(()),
            }
        })
    }

    /// A choice's keys are typed only while the screen shows what the
    /// operator answered: the command the card showed before the first key,
    /// and the highlight where each key that counts from it expects it. A
    /// highlight moved in the terminal before the answer, or another
    /// command's ask, types nothing, and a key typed in the terminal between
    /// two keys stops the write before the pick, so a refusal never becomes
    /// an allow.
    #[test]
    fn keys_stop_before_the_pick_when_the_screen_is_not_what_they_expect() {
        let gate = Arc::new(PaneInputGate::default());
        let open = node(None);
        let screen = choice_screen("antigravity", Ask::Permission, &[])
            .showing("cargo test --workspace\nmore");
        let keys = ["down", "down", "down", "enter"];
        let write_with = |fake: &Arc<FakeEngine<'static>>, keys: &[&str], screen: &Screen| {
            let engine = EngineHandle::new(fake.engine());
            let ready = || Ok(engine.clone());
            write(
                &open,
                &gate,
                "w1:p1",
                "p1",
                Typed::Choice { keys, screen },
                &ready,
            )
        };
        let answer = |fake: &Arc<FakeEngine<'static>>| write_with(fake, &keys, &screen);
        let sent = |fake: &Arc<FakeEngine<'static>>| -> Vec<String> {
            fake.calls_of("send_keys")
                .iter()
                .map(|call| call["keys"][0].as_str().unwrap().to_owned())
                .collect()
        };

        let moved = menu(1, 0);
        assert_eq!(answer(&moved), Err(Unwritten::Unready(SCREEN_DIFFERS)));
        assert_eq!(moved.methods(), ["read_pane"]);
        assert_eq!(
            moved.calls_of("read_pane")[0],
            serde_json::json!({"pane": "p1", "source": "visible", "lines": 0})
        );

        let interrupted = menu(0, 1);
        assert_eq!(
            answer(&interrupted),
            Err(Unwritten::Unready(SCREEN_DIFFERS))
        );
        assert_eq!(sent(&interrupted), ["down"]);

        let steady = menu(0, 0);
        assert_eq!(answer(&steady), Ok(()));
        assert_eq!(sent(&steady), keys);

        // Esc counts from no row, yet waits for the ask's rows and the
        // command the card showed: an ask allowed already, whose command
        // runs, or another command's, takes no Esc that would stop it.
        let rows = choice_screen("antigravity", Ask::Permission, &[]);
        let other = rows.clone().showing("rm -rf build");
        let elsewhere = menu(0, 0);
        assert_eq!(
            write_with(&elsewhere, &["esc"], &other),
            Err(Unwritten::Unready(SCREEN_DIFFERS))
        );
        assert_eq!(elsewhere.methods(), ["read_pane"]);
        let running = FakeEngine::start(|method, _| match method {
            "read_pane" => reply(crate::engine::PaneText {
                text: "  cargo test --workspace\n  Running 3 tests\n> ".to_owned(),
                truncated: false,
            }),
            _ => reply(()),
        });
        let shown = rows.showing("cargo test --workspace");
        assert_eq!(
            write_with(&running, &["esc"], &shown),
            Err(Unwritten::Unready(ASK_NOT_SHOWN))
        );
        assert_eq!(running.methods(), ["read_pane"]);
        let here = menu(2, 0);
        assert_eq!(write_with(&here, &["esc"], &shown), Ok(()));
        assert_eq!(sent(&here), ["esc"]);
    }

    /// One key write runs in a pane at a time: another, from a second phone
    /// or a quick key, is refused while it runs and types nothing.
    #[test]
    fn a_key_write_is_refused_while_another_holds_the_pane() {
        let fake = FakeEngine::start(|_, _| reply(()));
        let engine = EngineHandle::new(fake.engine());
        let ready = || Ok(engine.clone());
        let gate = Arc::new(PaneInputGate::default());
        let open = node(None);
        let held = gate.hold_keys("w1:p1").unwrap();
        assert_eq!(
            write(&open, &gate, "w1:p1", "p1", Typed::Keys(&["down"]), &ready),
            Err(Unwritten::Unready(IN_FLIGHT))
        );
        // Words are no key write, and another pane is not held.
        assert_eq!(
            write(&open, &gate, "w1:p1", "p1", Typed::Words("yes"), &ready),
            Ok(())
        );
        assert_eq!(
            write(&open, &gate, "w1:p2", "p2", Typed::Keys(&["down"]), &ready),
            Ok(())
        );
        drop(held);
        assert_eq!(
            write(&open, &gate, "w1:p1", "p1", Typed::Keys(&["down"]), &ready),
            Ok(())
        );
        assert_eq!(fake.methods(), ["prompt_agent", "send_keys", "send_keys"]);
    }

    #[test]
    fn only_an_unanswered_engine_leaves_the_write_in_doubt() {
        let refused = |code: &str| EngineError::Refused {
            code: code.to_owned(),
            message: String::new(),
        };
        assert_eq!(Unwritten::of(&refused("pane_not_found")), Unwritten::Gone);
        assert_eq!(Unwritten::of(&refused("busy")), Unwritten::Refused);
        assert_eq!(
            Unwritten::of(&EngineError::Transport("closed".into())),
            Unwritten::Unavailable
        );
        assert_eq!(
            Unwritten::of(&EngineError::Unanswered("timeout".into())),
            Unwritten::Uncertain
        );
        assert!(!Unwritten::Uncertain.certain());
        assert!(Unwritten::Unready("stale").certain());
    }
}
