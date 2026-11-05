//! The agents the core's own engine runs (PRD engine-bundle-3-core-owns
//! D-16, D-17, D-18): which agent a pane runs, under what name and session,
//! and the state its screen shows.
//!
//! A pane runs an agent the engine started in it (a spawn, a wake, a fork, a
//! resume), or one the operator typed into its shell, which the engine knows
//! by the program in the pane's foreground matched against the agent
//! definitions' `executables`. The pane's holder says what runs there and
//! what the screen rules read, whenever it changes ([`PaneFacts`]); the
//! engine never reads a screen on a timer. The agent ends when its program
//! leaves the foreground.
//!
//! The state is what the screen rules read: working, waiting on the operator
//! (blocked), or at rest, and at rest after working is a finished turn
//! (`done`, with the sequence it finished at), as Herdr reports it. A screen
//! no rule matches is at rest, as Herdr reads it. The core lays the agent's
//! hook and session file over this ([`crate::agent_state`]), so the screen
//! fills only what they do not report (D-18). An agent whose screen cannot be
//! read, with no rules or a holder that cannot run them, is `unknown`.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use hide_agent_adapter::definition::{Catalog, Definition, Source};
use hide_node_link::holders::{
    ForegroundProgram, PaneFacts, RuleSet, ScreenRead, ScreenRules, ScreenState,
};

use super::shell_line::stripped_title;
use crate::engine::answers::PaneSessionKind;

/// How long a start the engine typed may take to show its program in the
/// pane's foreground before it is taken as a start that did not happen:
/// the deadline Hide gives Herdr's starts.
pub(super) const START_DEADLINE: std::time::Duration = crate::agent_start::AGENT_START_DEADLINE;

/// How long after its start an agent the engine started is first taken as
/// taking input: Herdr's `AGENT_START_SETTLE_DELAY`. A program in the
/// foreground has not drawn its screen yet, and a screen no rule matches
/// reads at rest, so a start read sooner would be typed into a program that
/// has not asked for input. The time alone does not say the program reads
/// keys yet: a start whose program still leaves the terminal editing lines
/// is pending however long it took ([`Live::line_editing`]).
pub(super) const START_SETTLE: std::time::Duration = std::time::Duration::from_secs(3);

/// The agent definitions the engine starts and recognizes agents by: the
/// built-in ones until the core hands in a read of the operator's folder
/// (`Engine::set_agent_definitions`).
pub(super) struct Definitions(Mutex<Arc<Catalog>>);

impl Definitions {
    pub(super) fn new(catalog: Arc<Catalog>) -> Self {
        Self(Mutex::new(catalog))
    }

    pub(super) fn get(&self) -> Arc<Catalog> {
        Arc::clone(&self.0.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub(super) fn set(&self, catalog: Arc<Catalog>) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = catalog;
    }
}

/// The definition a caller's kind names: its id or one of its other
/// spellings, as Herdr's names (`claude`) reach the engine too.
pub(super) fn definition<'c>(catalog: &'c Catalog, kind: &str) -> Option<&'c Definition> {
    let kind = kind.trim();
    catalog.definitions.iter().find(|definition| {
        definition.id.eq_ignore_ascii_case(kind)
            || definition
                .aliases
                .iter()
                .any(|alias| alias.eq_ignore_ascii_case(kind))
    })
}

/// Native and runtime script names take precedence over the optional
/// package identity. Both identities come from the agent definition;
/// neither installation routes nor an unscoped guess participate.
fn recognized<'c>(catalog: &'c Catalog, program: &ForegroundProgram) -> Option<&'c Definition> {
    catalog
        .definitions
        .iter()
        .find(|definition| {
            definition
                .executables
                .iter()
                .any(|executable| executable.eq_ignore_ascii_case(&program.name))
        })
        .or_else(|| {
            let package = program.package.as_deref()?;
            catalog.definitions.iter().find(|definition| {
                definition
                    .packages
                    .iter()
                    .any(|declared| declared.eq_ignore_ascii_case(package))
            })
        })
}

/// The name the core knows an agent kind by: Herdr's spelling for an agent
/// with Hide's own features (`claude`), the definition's id for any other,
/// so a row reads alike on both engines.
pub(super) fn kind_name(definition: &Definition) -> String {
    std::iter::once(&definition.id)
        .chain(&definition.aliases)
        .find_map(|name| hide_agent_adapter::adapter(name))
        .map_or_else(
            || definition.id.clone(),
            |adapter| adapter.herdr.name.to_owned(),
        )
}

/// The rule sets for `definition`'s screen, the operator's first and then
/// the built-in one it replaced (D-20), each as its holder reads it; `None`
/// for an agent with no screen rules.
pub(super) fn screen_rules(definition: &Definition) -> Option<ScreenRules> {
    let mut sets = Vec::new();
    let mut push = |rules: &hide_screen_rules::Rules, source: &Source| {
        let source = match source {
            Source::Builtin => "built-in".to_owned(),
            Source::User(file) => file.clone(),
        };
        match serde_json::to_string(rules.manifest()) {
            Ok(manifest) => sets.push(RuleSet {
                source,
                version: rules.version().to_string(),
                min_engine_version: rules.min_engine_version(),
                manifest,
            }),
            // A manifest serializes to what it read; a set that does not is
            // left out, and the holder reads with the next one.
            Err(error) => crate::diagnostic!(serde_json::json!({
                "component": "own_engine",
                "kind": "agent.rules_not_serialized",
                "agent": definition.id,
                "source": source,
                "error": error.to_string(),
            })),
        }
    };
    if let Some(screen) = &definition.screen {
        push(&screen.rules, &screen.source);
    }
    // The built-in rules stand behind the operator's, for a holder whose
    // evaluator is too old for theirs.
    if definition
        .screen
        .as_ref()
        .is_some_and(|screen| screen.source != Source::Builtin)
    {
        let base = definition.based_on.as_deref().unwrap_or(&definition.id);
        if let Some(builtin) = hide_agent_adapter::definition::builtin(base)
            && let Some(screen) = &builtin.screen
        {
            push(&screen.rules, &screen.source);
        }
    }
    (!sets.is_empty()).then(|| ScreenRules {
        agent: definition.id.clone(),
        sets,
    })
}

/// The words a start of `definition` types: its program, its `[start]`
/// arguments, then the caller's.
pub(super) fn start_argv(
    definition: &Definition,
    args: Vec<String>,
) -> Result<Vec<String>, String> {
    let program = definition
        .executables
        .first()
        .ok_or_else(|| format!("the {} definition names no program", definition.id))?;
    Ok(std::iter::once(program.clone())
        .chain(definition.start_args.iter().cloned())
        .chain(args)
        .collect())
}

/// The words a resume of `definition` types: its `[resume] session` with
/// `session` in place, or its `[resume] last` when the session is not
/// known; `None` when it resumes neither way.
pub(super) fn resume_argv(definition: &Definition, session: Option<&str>) -> Option<Vec<String>> {
    let args = match (session, &definition.resume_session, &definition.resume_last) {
        (Some(session), Some(template), _) => template
            .iter()
            .map(|word| word.replace("{session}", session))
            .collect(),
        (_, _, Some(last)) => last.clone(),
        _ => return None,
    };
    start_argv(definition, args).ok()
}

/// The session a program's arguments resume, by `definition`'s `[resume]
/// session` selector, before any `--`.
/// The session a start the engine types runs, before its agent says: the
/// one its resume names, so a woken agent names the conversation it resumed
/// at once. Not when an agent whose integration reports its session as a
/// file's path (Pi, omp) resumes by id: an id recorded for a woken omp read
/// as another conversation than the path its registration held, so the
/// registration was refused (`identity_changed`, journey 871294628), and
/// such a session waits for the agent's own report. Resumed by that file's
/// path (Pi's `--session <path>`), the path is the session it reports.
/// Nor for a wake of OpenCode, which takes keys only once it has loaded the
/// session its plugin then reports (`AgentLaunch::wakes`); a spawn that
/// resumes OpenCode is known by its resume at once.
pub(super) fn started_session(
    definition: &Definition,
    argv: &[String],
    wakes: bool,
) -> Option<String> {
    let session = resumed_in(definition, argv)?;
    let format = hide_agent_adapter::adapter(&definition.id).and_then(|adapter| adapter.session);
    if wakes && format.is_some_and(hide_agent_adapter::SessionFormat::reports_session_once_loaded) {
        return None;
    }
    let reports_path = format.is_some_and(hide_agent_adapter::SessionFormat::reports_session_path);
    // A Windows device's path comes in its own spelling.
    let names_path = hide_platform::path::is_wire_absolute(&session.replace('\\', "/"));
    (!reports_path || names_path).then_some(session)
}

pub(super) fn resumed_in(definition: &Definition, argv: &[String]) -> Option<String> {
    let template = definition.resume_session.as_ref()?;
    let at = template.iter().position(|word| word == "{session}")?;
    let selector = &template[..at];
    let args: Vec<&String> = argv
        .iter()
        .skip(1)
        .take_while(|arg| arg.as_str() != "--")
        .collect();
    // A fork's selector names its source, not the running conversation.
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--fork" | "--fork-session" | "fork"))
    {
        return None;
    }
    args.windows(selector.len() + 1)
        .find(|window| {
            window[..selector.len()]
                .iter()
                .zip(selector)
                .all(|(arg, word)| *arg == word)
        })
        .map(|window| window[selector.len()].clone())
        .filter(|session| {
            !session.is_empty()
                && session.len() <= 4096
                && !session.starts_with('-')
                && !session.chars().any(char::is_control)
        })
}

/// How a session reference is named: an id Hide's readers take, or else the
/// path of its file.
pub(super) fn session_kind(session: &str) -> PaneSessionKind {
    if hide_session::valid_native_id(session) {
        PaneSessionKind::Id
    } else {
        PaneSessionKind::Path
    }
}

/// The agent a pane runs, or ran when its session was saved.
#[derive(Clone, Debug, Default)]
pub(super) struct PaneAgent {
    /// The definition's id.
    pub(super) kind: String,
    /// The name it was started under, when the engine started it.
    pub(super) name: Option<String>,
    /// Its session, an id or a session file's path, once known.
    pub(super) session: Option<String>,
    /// What it does now; kept in memory only.
    pub(super) live: Live,
}

/// What an agent does now.
#[derive(Clone, Debug, Default)]
pub(super) struct Live {
    /// When the engine typed its start, until the start settled
    /// ([`PaneAgent::launch_pending`]).
    pub(super) started: Option<Instant>,
    /// The start whose not-yet-delivered metadata can still be withdrawn.
    /// Foreground or session evidence invalidates it, even at the same value.
    pub(super) start_claim: Option<Arc<()>>,
    /// A listed name the same attempt can still restore on non-delivery.
    pub(super) start_previous_name: Option<String>,
    /// Its program is in the pane's foreground.
    pub(super) shown: bool,
    /// The terminal edits lines, as its holder last read it: the program has
    /// not turned that off to read keys, so what is typed waits in the
    /// terminal's line and the Enter that submits it reaches the program as
    /// a line break. Pi 1.0.4 started on a CI runner took more than
    /// [`START_SETTLE`] after its start line to do so, and a brief typed at
    /// that mark sat in its editor, never submitted (nightly 41789009557).
    /// `None` where the holder cannot say (Windows, or a holder or node
    /// built before it), which leaves a start to the settle alone, as
    /// before; never read as reading keys.
    pub(super) line_editing: Option<bool>,
    /// The holder could not say what holds the terminal, so who the agent
    /// is stays as it was and it takes no input until it can: read as the
    /// shell holding it, it would have ended the agent and its session.
    pub(super) foreground_unread: bool,
    /// Its definition declares no screen rules (omp), so its screen never
    /// reads at rest and a start cannot wait for that: it settles on what
    /// the holder can say, its program shown and reading keys, and the
    /// settle. Waiting for a rest that never comes held every omp start
    /// pending to its 120 s deadline (journey 871294628).
    pub(super) rest_unread: bool,
    pub(super) status: Status,
    /// One more each time its status changed, from the engine's own count.
    pub(super) seq: u64,
    /// The sequence its current stop finished work at, while it rests after
    /// working.
    pub(super) completion: Option<u64>,
}

impl PaneAgent {
    /// An agent the engine starts now.
    pub(super) fn started(kind: &str, name: Option<&str>, session: Option<String>) -> Self {
        Self {
            kind: kind.to_owned(),
            name: name.map(str::to_owned),
            session,
            live: Live {
                started: Some(Instant::now()),
                ..Live::default()
            },
        }
    }

    /// Marks the agent's program shown; whether that changed anything.
    fn show(&mut self) -> bool {
        let changed = !self.live.shown;
        self.live.shown = true;
        self.live.start_claim = None;
        self.live.start_previous_name = None;
        changed
    }

    /// Whether the agent runs or is starting: an agent the save names that
    /// no program has shown for since the daemon started is not listed.
    pub(super) fn listed(&self, now: Instant) -> bool {
        self.live.shown || self.starting(now)
    }

    /// Whether the engine typed its start and its program has not shown yet,
    /// within the start's deadline.
    pub(super) fn starting(&self, now: Instant) -> bool {
        !self.live.shown
            && self
                .live
                .started
                .is_some_and(|started| now.duration_since(started) < START_DEADLINE)
    }

    /// Whether the start the engine typed is still pending, as Herdr's
    /// `launch_pending` reads a start of its own: until, at least
    /// [`START_SETTLE`] after the start, or after the answer to a question
    /// its screen asked, the agent's program shows and its screen is at
    /// rest. A screen that asks a question (Claude Code's folder trust)
    /// keeps the start pending past its deadline, since what is typed there
    /// answers the question; anything else ends it at the deadline.
    pub(super) fn launch_pending(&self, now: Instant) -> bool {
        let Some(started) = self.live.started else {
            return false;
        };
        !self.settled(started, now)
            && ((self.live.shown && self.live.status == Status::Blocked)
                || now.duration_since(started) < START_DEADLINE)
    }

    /// The next moment this agent's pending start can end with nothing the
    /// holder reports changing: its settle, then its deadline. `None` once it
    /// is not pending. Past the settle a start still pending waits on what
    /// the holder reports next (its program reading keys, its screen
    /// resting), which is a change of its own.
    pub(super) fn start_changes_at(&self, now: Instant) -> Option<Instant> {
        let started = self.live.started?;
        if !self.launch_pending(now) {
            return None;
        }
        [started + START_SETTLE, started + START_DEADLINE]
            .into_iter()
            .find(|at| *at > now)
    }

    /// Whether the agent's program takes what is typed to it: it shows, the
    /// holder can say what holds the terminal, and the program does not
    /// leave it editing lines. This is not whether its
    /// start is pending: a start ends at its deadline, while a program that
    /// never turns line editing off takes no input however long ago it
    /// started, and a retried letter must not be typed into its line.
    pub(super) fn interactive(&self) -> bool {
        self.live.shown && !self.live.foreground_unread && self.live.line_editing != Some(true)
    }

    /// Whether a start made at `started` has settled by `now`: its program
    /// takes input ([`Self::interactive`]) and its screen rests, or cannot be
    /// read at rest ([`Live::rest_unread`]), at least [`START_SETTLE`] after
    /// the start.
    fn settled(&self, started: Instant, now: Instant) -> bool {
        self.interactive()
            && (matches!(self.live.status, Status::Idle | Status::Done) || self.live.rest_unread)
            && now.duration_since(started) >= START_SETTLE
    }
}

/// An agent's state as Herdr's `agent_status` names it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Status {
    #[default]
    Unknown,
    Idle,
    Working,
    Blocked,
    Done,
}

impl Status {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }
}

/// What a pane's holder last said of it, kept in memory only.
#[derive(Clone, Debug, Default)]
pub(super) struct PaneLive {
    pub(super) facts: PaneFacts,
    /// The holder has said something since the pane was started or found.
    pub(super) heard: bool,
    /// The last requested decision: outer None is undecided, Some(None)
    /// explicitly removes rules. A new engine must send its first decision
    /// even for a shell whose holder kept the previous agent's rules.
    pub(super) rules_sent: Option<Option<String>>,
}

impl PaneLive {
    /// The agent whose rules the holder should have, when that is not the
    /// one it was given; recorded as given.
    pub(super) fn rules_due(&mut self, agent: Option<&PaneAgent>) -> Option<Option<String>> {
        let due = agent.map(|agent| agent.kind.clone());
        (self.rules_sent.as_ref() != Some(&due)).then(|| {
            self.rules_sent = Some(due.clone());
            due
        })
    }
}

/// What applying a pane's facts changed.
#[derive(Debug, Default, Eq, PartialEq)]
pub(super) struct Applied {
    /// The session as a snapshot projects it moved.
    pub(super) changed: bool,
    /// The pane's holder is due another agent's rules (or none).
    pub(super) rules_for: Option<Option<String>>,
}

/// Lays `facts` on a pane whose folder is `cwd` and whose agent is `agent`:
/// the folder it reports, the agent its foreground runs, and the state its
/// screen shows. `seq` is the engine's count of state changes.
pub(super) fn apply(
    catalog: &Catalog,
    cwd: &mut String,
    agent: &mut Option<PaneAgent>,
    live: &mut PaneLive,
    facts: &PaneFacts,
    seq: &mut u64,
) -> Applied {
    let now = Instant::now();
    let mut applied = Applied::default();
    let before = live.facts.clone();
    live.heard = true;
    live.facts = facts.clone();
    if let Some(reported) = facts.cwd.as_ref().filter(|reported| *reported != cwd) {
        cwd.clone_from(reported);
        applied.changed = true;
    }
    // The title is compared, and projected, without the activity glyph an
    // agent turns in front of it every frame (Claude Code's spinner), as
    // Herdr compares it: a turning glyph alone is no change of the session.
    if stripped(&before) != stripped(facts) {
        applied.changed = true;
    }
    let program = facts.foreground.as_ref();
    let foreground = program.and_then(|program| recognized(catalog, program));
    match (agent.as_mut(), program, foreground) {
        // The system could not say what holds the terminal: the agent stays
        // who it is until the holder reads its terminal again.
        _ if facts.foreground_unread => {}
        // The program of the agent the pane runs is in the foreground, or a
        // program no definition names where that agent was started or ran:
        // the agent itself, run by a wrapper or a runtime under another
        // name (`node …/cli.js`), as Herdr reads an agent it launched.
        (Some(current), _, Some(definition)) if definition.id == current.kind => {
            applied.changed |= current.show();
        }
        (Some(current), Some(_), None) => {
            applied.changed |= current.show();
        }
        // Another agent's program: the operator typed it, or the start that
        // was typed ran something else.
        (_, _, Some(definition)) => {
            *agent = Some(PaneAgent {
                kind: definition.id.clone(),
                live: Live {
                    shown: true,
                    ..Live::default()
                },
                ..PaneAgent::default()
            });
            applied.changed = true;
        }
        // Nothing is in the foreground but the shell: an agent that showed
        // has ended, and one still starting waits until its deadline.
        (Some(current), None, None) => {
            if current.live.shown || !current.starting(now) {
                *agent = None;
                applied.changed = true;
            }
        }
        (None, _, None) => {}
    }
    if let Some(current) = agent.as_mut()
        && current.live.foreground_unread != facts.foreground_unread
    {
        current.live.foreground_unread = facts.foreground_unread;
        applied.changed = true;
    }
    applied.rules_for = live.rules_due(agent.as_ref());
    if let Some(current) = agent.as_mut().filter(|current| current.live.shown) {
        current.live.rest_unread =
            definition(catalog, &current.kind).is_none_or(|definition| definition.screen.is_none());
        // A start that settled on what the screen read last stays settled
        // whatever this screen reads, as Herdr's active agent does.
        if current
            .live
            .started
            .is_some_and(|started| current.settled(started, now))
        {
            current.live.started = None;
        }
        if facts.line_editing != current.live.line_editing {
            // The program taking the terminal's keys restarts a pending
            // start's settle, as an answered question does: it draws its
            // input screen after, and OpenCode 2.0.29 dropped what was typed
            // up to a second after it took the keys.
            if current.live.line_editing == Some(true)
                && facts.line_editing == Some(false)
                && current.live.started.is_some()
            {
                current.live.started = Some(now);
            }
            current.live.line_editing = facts.line_editing;
            applied.changed |= current.live.started.is_some();
        }
        let mut status = status_of(&facts.screen, current.live.status);
        // A screen no rules read never says the agent rests, so once its
        // program takes input it reads idle, as Herdr reads an agent it
        // started, and its integration's turns say when it works (they
        // outrank this, `agent_state::axes`). Read unknown, a woken or
        // forked omp stayed a starting row with no state change to publish
        // its settle (journey 41850644522).
        if current.live.rest_unread && status == Status::Unknown && current.interactive() {
            status = Status::Idle;
        }
        // An answered question restarts a pending start's settle: the agent
        // draws its next screen after it, and input typed while it does is
        // lost (Claude Code 2.1.318 on Windows after its folder trust).
        if current.live.status == Status::Blocked
            && status != Status::Blocked
            && current.live.started.is_some()
        {
            current.live.started = Some(now);
        }
        if status != current.live.status {
            *seq += 1;
            current.live.completion = match status {
                Status::Done => Some(current.live.completion.unwrap_or(*seq)),
                _ => None,
            };
            current.live.status = status;
            current.live.seq = *seq;
            applied.changed = true;
        }
    }
    applied
}

/// The title `facts` carry as a session shows it ([`stripped_title`]).
pub(super) fn stripped(facts: &PaneFacts) -> Option<String> {
    facts.title.as_deref().and_then(stripped_title)
}

/// The state `screen` reads for an agent that was `before`: at rest after
/// working is a finished turn, which stays one until the agent works again.
fn status_of(screen: &ScreenRead, before: Status) -> Status {
    let read = match screen {
        ScreenRead::Read {
            state: ScreenState::Working,
            ..
        } => Status::Working,
        ScreenRead::Read {
            state: ScreenState::Blocked,
            ..
        } => Status::Blocked,
        // No rule matched: at rest, as Herdr reads a screen it knows nothing
        // of for an agent whose rules it has.
        ScreenRead::Read {
            state: ScreenState::Idle,
            ..
        }
        | ScreenRead::Read {
            state: ScreenState::Unknown,
            rule: None,
        } => Status::Idle,
        ScreenRead::Read {
            state: ScreenState::Unknown,
            rule: Some(_),
        }
        | ScreenRead::NoRules
        | ScreenRead::Unknown { .. } => Status::Unknown,
    };
    match (before, read) {
        (Status::Working | Status::Done, Status::Idle) => Status::Done,
        (_, read) => read,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        Catalog::read(Vec::new())
    }

    fn program(name: &str) -> Option<ForegroundProgram> {
        Some(ForegroundProgram {
            pid: 7,
            name: name.to_owned(),
            package: None,
            argv: vec![name.to_owned()],
        })
    }

    fn read(state: ScreenState) -> ScreenRead {
        ScreenRead::Read {
            state,
            rule: Some("rule".to_owned()),
        }
    }

    #[test]
    fn installed_packages_identify_agents_without_guessing_from_cli_js() {
        let catalog = catalog();
        for (package, kind) in [
            ("@anthropic-ai/claude-code", "claude"),
            ("@openai/codex", "codex"),
            ("@xai-official/grok", "grok"),
            ("opencode-ai", "opencode"),
            ("@earendil-works/pi-coding-agent", "pi"),
            ("@oh-my-pi/pi-coding-agent", "omp"),
            ("@moonshot-ai/kimi-code", "kimi"),
            ("@qwen-code/qwen-code", "qwen"),
            ("@letta-ai/letta-code", "letta"),
        ] {
            let mut program = program("cli").unwrap();
            program.package = Some(package.to_owned());
            assert_eq!(
                recognized(&catalog, &program).map(kind_name).as_deref(),
                Some(kind)
            );
        }
        for unknown in [
            "helper",
            "@unknown/pi-coding-agent",
            "@unknown/kimi-code",
            "@unknown/qwen-code",
        ] {
            let mut program = program("cli").unwrap();
            program.package = Some(unknown.to_owned());
            assert!(recognized(&catalog, &program).is_none());
        }
    }

    #[test]
    fn a_package_keeps_the_script_name_and_can_name_a_generic_script() {
        let catalog = catalog();
        for (name, package) in [
            ("gemini", "@oh-my-pi/pi-coding-agent"),
            ("letta", "@letta-ai/letta-code"),
            ("codex", "@earendil-works/pi-coding-agent"),
        ] {
            let mut program = program(name).unwrap();
            assert_eq!(
                recognized(&catalog, &program).map(kind_name).as_deref(),
                Some(name)
            );
            program.package = Some(package.to_owned());
            assert_eq!(
                recognized(&catalog, &program).map(kind_name).as_deref(),
                Some(name)
            );
        }
        let mut program = program("main").unwrap();
        program.package = Some("@moonshot-ai/kimi-code".to_owned());
        assert_eq!(
            recognized(&catalog, &program).map(kind_name).as_deref(),
            Some("kimi")
        );
    }

    struct Pane {
        cwd: String,
        agent: Option<PaneAgent>,
        live: PaneLive,
        seq: u64,
    }

    impl Pane {
        fn new() -> Self {
            Self {
                cwd: "/work".to_owned(),
                agent: None,
                live: PaneLive::default(),
                seq: 0,
            }
        }

        fn facts(&mut self, facts: PaneFacts) -> Applied {
            apply(
                &catalog(),
                &mut self.cwd,
                &mut self.agent,
                &mut self.live,
                &facts,
                &mut self.seq,
            )
        }

        fn status(&self) -> Option<&'static str> {
            self.agent.as_ref().map(|agent| agent.live.status.as_str())
        }
    }

    /// Claude Code turns a spinner glyph in front of its title every frame:
    /// the title's text alone is the session's, as Herdr compares it.
    #[test]
    fn a_title_whose_spinner_alone_turns_changes_nothing_and_new_text_does() {
        let mut pane = Pane::new();
        let titled = |title: &str| PaneFacts {
            title: Some(title.to_owned()),
            ..PaneFacts::default()
        };
        assert!(pane.facts(titled("⠋ Fix the tests")).changed);
        for turned in ["⠙ Fix the tests", "◐ Fix the tests", "✳ Fix the tests"] {
            assert!(!pane.facts(titled(turned)).changed, "{turned}");
        }
        assert!(pane.facts(titled("✳ Fix the build")).changed);
    }

    #[test]
    fn a_program_typed_into_the_shell_is_the_agent_its_definition_names() {
        let mut pane = Pane::new();
        let applied = pane.facts(PaneFacts {
            foreground: program("gemini"),
            screen: read(ScreenState::Working),
            ..PaneFacts::default()
        });
        assert!(applied.changed);
        assert_eq!(applied.rules_for, Some(Some("gemini".to_owned())));
        assert_eq!(pane.agent.as_ref().unwrap().kind, "gemini");
        assert_eq!(pane.status(), Some("working"));
        // Its program leaving the foreground ends it.
        let applied = pane.facts(PaneFacts::default());
        assert_eq!(applied.rules_for, Some(None));
        assert!(pane.agent.is_none());
    }

    /// An agent run by a runtime under another name: pi's npm build runs as
    /// `node …/dist/bundle/cli.js`, which the holder names `cli`.
    #[test]
    fn a_started_agent_is_whatever_its_start_runs() {
        let mut pane = Pane::new();
        pane.agent = Some(PaneAgent::started("pi", Some("worker"), None));
        pane.facts(PaneFacts {
            foreground: program("cli"),
            screen: read(ScreenState::Working),
            ..PaneFacts::default()
        });
        let agent = pane.agent.as_ref().unwrap();
        assert_eq!((agent.kind.as_str(), agent.live.shown), ("pi", true));
        assert_eq!(pane.status(), Some("working"));
        pane.facts(PaneFacts::default());
        assert!(pane.agent.is_none(), "the shell holds the terminal again");

        pane.facts(PaneFacts {
            foreground: program("cli"),
            ..PaneFacts::default()
        });
        assert!(
            pane.agent.is_none(),
            "an unnamed program is no agent of its own"
        );
    }

    #[test]
    fn a_stop_after_work_is_a_finished_turn_until_the_agent_works_again() {
        let mut pane = Pane::new();
        let mut at = |state| {
            pane.facts(PaneFacts {
                foreground: program("claude"),
                screen: read(state),
                ..PaneFacts::default()
            });
            let agent = pane.agent.as_ref().unwrap();
            (agent.live.status, agent.live.completion, agent.live.seq)
        };
        assert_eq!(at(ScreenState::Idle), (Status::Idle, None, 1));
        assert_eq!(at(ScreenState::Working), (Status::Working, None, 2));
        assert_eq!(at(ScreenState::Idle), (Status::Done, Some(3), 3));
        // The same reading again moves nothing.
        assert_eq!(at(ScreenState::Idle), (Status::Done, Some(3), 3));
        assert_eq!(at(ScreenState::Blocked), (Status::Blocked, None, 4));
    }

    #[test]
    fn a_start_waits_for_its_program_and_a_screen_without_rules_reads_idle_once_shown() {
        let mut pane = Pane::new();
        pane.agent = Some(PaneAgent::started("omp", Some("worker"), None));
        // The shell holds the terminal while the start is typed.
        pane.facts(PaneFacts::default());
        assert!(pane.agent.as_ref().unwrap().starting(Instant::now()));
        pane.facts(PaneFacts {
            foreground: program("omp"),
            ..PaneFacts::default()
        });
        let agent = pane.agent.as_ref().unwrap();
        assert!(agent.live.shown);
        assert_eq!(agent.name.as_deref(), Some("worker"));
        // Where the holder cannot say whether it reads keys, the settle
        // alone decides its input; its turns come from its integration.
        assert_eq!(agent.live.status, Status::Idle);
    }

    /// An agent whose definition has no screen rules (omp) never reads at
    /// rest, so its start settles once its program reads keys and the settle
    /// passed, not at its 120 s deadline; one with rules still waits for its
    /// screen to rest.
    #[test]
    fn a_start_without_screen_rules_settles_once_its_program_reads_keys() {
        let at = |name: &str, line_editing| PaneFacts {
            foreground: program(name),
            line_editing,
            ..PaneFacts::default()
        };
        let mut pane = Pane::new();
        pane.agent = Some(PaneAgent::started("omp", Some("worker"), None));
        pane.facts(at("omp", Some(true)));
        let started = Instant::now() - START_SETTLE;
        pane.agent.as_mut().unwrap().live.started = Some(started);
        assert!(
            pane.agent.as_ref().unwrap().launch_pending(Instant::now()),
            "its terminal still edits lines"
        );
        assert_eq!(pane.status(), Some("unknown"), "it takes no input yet");
        pane.facts(at("omp", Some(false)));
        let agent = pane.agent.as_mut().unwrap();
        // Read at rest once it takes input, a state change the core hears.
        assert_eq!(agent.live.status, Status::Idle);
        assert_eq!(agent.live.seq, 1);
        assert!(agent.launch_pending(Instant::now()), "a settle of its own");
        agent.live.started = Some(Instant::now() - START_SETTLE);
        assert!(!agent.launch_pending(Instant::now()));

        let mut pane = Pane::new();
        pane.agent = Some(PaneAgent::started("claude-code", Some("worker"), None));
        pane.facts(PaneFacts {
            screen: read(ScreenState::Working),
            ..at("claude", Some(false))
        });
        pane.agent.as_mut().unwrap().live.started = Some(started);
        assert!(
            pane.agent.as_ref().unwrap().launch_pending(Instant::now()),
            "a screen with rules that does not rest keeps its start pending"
        );
    }

    /// Herdr's `launch_pending` for a start of its own: the agent takes
    /// input once its screen rests after the settle delay, and a question on
    /// its screen (Claude Code's folder trust, whose default answer exits)
    /// keeps the start pending past its deadline. Typing into it there on
    /// Windows answered the question and ended the agent.
    #[test]
    fn a_start_takes_input_only_once_its_screen_rests_after_the_settle() {
        let mut pane = Pane::new();
        pane.agent = Some(PaneAgent::started("claude-code", Some("worker"), None));
        let at = |state| PaneFacts {
            foreground: program("claude"),
            screen: read(state),
            ..PaneFacts::default()
        };
        pane.facts(at(ScreenState::Idle));
        let agent = pane.agent.as_ref().unwrap();
        assert!(agent.live.shown);
        assert!(
            agent.launch_pending(Instant::now()),
            "drawn within the settle"
        );

        pane.facts(at(ScreenState::Blocked));
        let started = Instant::now() - START_SETTLE;
        pane.agent.as_mut().unwrap().live.started = Some(started);
        let agent = pane.agent.as_ref().unwrap();
        assert!(agent.launch_pending(Instant::now()));
        assert!(
            agent.launch_pending(started + START_DEADLINE * 2),
            "a question waits for its answer"
        );

        pane.facts(at(ScreenState::Idle));
        let agent = pane.agent.as_mut().unwrap();
        assert!(
            agent.launch_pending(Instant::now()),
            "the screen after an answer is drawn within a settle of its own"
        );
        let answered = agent.live.started.unwrap();
        assert!(answered > started);
        agent.live.started = Some(answered - START_SETTLE);
        assert!(!agent.launch_pending(Instant::now()));
        pane.facts(at(ScreenState::Working));
        let agent = pane.agent.as_ref().unwrap();
        assert!(
            !agent.launch_pending(Instant::now()),
            "a settled start stays settled"
        );
        assert_eq!(agent.live.started, None);
    }

    /// A program in the foreground whose screen rests can still leave the
    /// terminal editing lines: Pi 1.0.4 on a CI runner had not turned it off
    /// [`START_SETTLE`] after its start, and a brief typed then sat in its
    /// editor with its Enter read as a line break. Its start settles a
    /// settle after the program reads keys; a holder that cannot say keeps
    /// the settle alone.
    #[test]
    fn a_start_takes_input_only_once_its_program_reads_keys() {
        let mut pane = Pane::new();
        pane.agent = Some(PaneAgent::started("pi", Some("worker"), None));
        let at = |line_editing| PaneFacts {
            foreground: program("pi"),
            screen: read(ScreenState::Idle),
            line_editing,
            ..PaneFacts::default()
        };
        pane.facts(at(Some(true)));
        let started = Instant::now() - START_SETTLE;
        pane.agent.as_mut().unwrap().live.started = Some(started);
        let agent = pane.agent.as_ref().unwrap();
        assert!(
            agent.launch_pending(Instant::now()),
            "its screen rests past the settle, but the terminal edits lines"
        );
        assert!(!agent.interactive());
        let deadline = started + START_DEADLINE;
        assert!(
            !agent.launch_pending(deadline),
            "the start ends at its deadline"
        );
        assert!(
            !agent.interactive(),
            "yet a program that leaves the terminal editing lines takes no input"
        );
        let applied = pane.facts(at(Some(false)));
        assert!(applied.changed, "the start's readiness moved");
        let agent = pane.agent.as_mut().unwrap();
        assert!(
            agent.launch_pending(Instant::now()),
            "the screen it draws once it reads keys is drawn within a settle of its own"
        );
        let took_keys = agent.live.started.unwrap();
        assert!(took_keys > started);
        assert!(agent.interactive());
        agent.live.started = Some(took_keys - START_SETTLE);
        assert!(!agent.launch_pending(Instant::now()));

        let mut pane = Pane::new();
        pane.agent = Some(PaneAgent::started("pi", Some("worker"), None));
        pane.facts(at(None));
        pane.agent.as_mut().unwrap().live.started = Some(started);
        assert!(
            !pane.agent.as_ref().unwrap().launch_pending(Instant::now()),
            "a holder that cannot say leaves the start to the settle"
        );
    }

    /// A terminal whose foreground the system could not say keeps the
    /// agent it runs, its name and session, and takes no input; only a
    /// terminal read as the shell's again ends it.
    #[test]
    fn an_unread_foreground_keeps_the_agent_and_takes_no_input() {
        let mut pane = Pane::new();
        pane.agent = Some(PaneAgent::started(
            "pi",
            Some("worker"),
            Some("/s/pi.jsonl".to_owned()),
        ));
        pane.facts(PaneFacts {
            foreground: program("pi"),
            screen: read(ScreenState::Idle),
            ..PaneFacts::default()
        });
        assert!(pane.agent.as_ref().unwrap().interactive());
        let applied = pane.facts(PaneFacts {
            foreground_unread: true,
            screen: read(ScreenState::Idle),
            ..PaneFacts::default()
        });
        assert!(applied.changed);
        let agent = pane.agent.as_ref().expect("the agent stays");
        assert_eq!(agent.name.as_deref(), Some("worker"));
        assert_eq!(agent.session.as_deref(), Some("/s/pi.jsonl"));
        assert!(!agent.interactive(), "it takes no input meanwhile");

        pane.facts(PaneFacts {
            foreground: program("pi"),
            screen: read(ScreenState::Idle),
            ..PaneFacts::default()
        });
        assert!(pane.agent.as_ref().unwrap().interactive());
        pane.facts(PaneFacts {
            screen: read(ScreenState::Idle),
            ..PaneFacts::default()
        });
        assert!(pane.agent.is_none(), "the shell holds the terminal again");
    }

    #[test]
    fn a_start_with_no_screen_at_rest_ends_at_its_deadline() {
        let mut pane = Pane::new();
        pane.agent = Some(PaneAgent::started("claude-code", Some("worker"), None));
        pane.facts(PaneFacts {
            foreground: program("claude"),
            screen: read(ScreenState::Working),
            ..PaneFacts::default()
        });
        let agent = pane.agent.as_ref().unwrap();
        let started = agent.live.started.unwrap();
        assert!(agent.launch_pending(started + START_SETTLE));
        assert!(!agent.launch_pending(started + START_DEADLINE));
    }

    #[test]
    fn a_reported_folder_becomes_the_panes_folder() {
        let mut pane = Pane::new();
        let applied = pane.facts(PaneFacts {
            cwd: Some("/work/sub".to_owned()),
            ..PaneFacts::default()
        });
        assert!(applied.changed);
        assert_eq!(pane.cwd, "/work/sub");
        assert!(
            !pane
                .facts(PaneFacts {
                    cwd: Some("/work/sub".to_owned()),
                    ..PaneFacts::default()
                })
                .changed
        );
    }

    #[test]
    fn a_resume_types_the_session_or_the_last_conversation() {
        let catalog = catalog();
        let claude = definition(&catalog, "claude").unwrap();
        assert_eq!(claude.id, "claude-code");
        assert_eq!(kind_name(claude), "claude");
        assert_eq!(
            resume_argv(claude, Some("abc")).unwrap(),
            ["claude", "--resume", "abc"]
        );
        assert_eq!(resume_argv(claude, None), None);
        let gemini = definition(&catalog, "gemini").unwrap();
        assert_eq!(kind_name(gemini), "gemini");
        assert_eq!(
            resume_argv(gemini, None).unwrap(),
            ["gemini", "--resume", "latest"]
        );
        let codex = definition(&catalog, "codex").unwrap();
        let argv: Vec<String> = [
            "codex", "--model", "x", "resume", "s-1", "--", "resume", "s-2",
        ]
        .iter()
        .map(|word| (*word).to_owned())
        .collect();
        assert_eq!(resumed_in(codex, &argv).as_deref(), Some("s-1"));
        assert_eq!(resumed_in(codex, &argv[..3]), None);
    }

    /// A start records the session its resume names, except for an agent
    /// that reports its session as a file's path and resumes by id: omp's id
    /// would read as another conversation and a woken omp's registration
    /// would be refused. Pi resumed by its file's path records the path;
    /// Claude reports the id it resumes.
    #[test]
    fn a_start_records_its_resumed_session_unless_its_agent_reports_a_path() {
        let catalog = catalog();
        let words = |words: &[&str]| -> Vec<String> {
            words.iter().map(|word| (*word).to_owned()).collect()
        };
        let omp = definition(&catalog, "omp").unwrap();
        let argv = words(&["omp", "--resume", "01a1a635-dc01-749e-b762-b4aa840d457c"]);
        assert!(resumed_in(omp, &argv).is_some());
        assert_eq!(started_session(omp, &argv, false), None);
        let pi = definition(&catalog, "pi").unwrap();
        for path in ["/home/me/.pi/s.jsonl", "C:\\Users\\me\\.pi\\s.jsonl"] {
            let argv = words(&["pi", "--session", path]);
            assert_eq!(started_session(pi, &argv, false).as_deref(), Some(path));
        }
        let agy = definition(&catalog, "agy").unwrap();
        let argv = words(&["agy", "--conversation", "c-1"]);
        assert_eq!(resumed_in(agy, &argv).as_deref(), Some("c-1"));
        assert_eq!(started_session(agy, &argv, false).as_deref(), Some("c-1"));
        // A woken OpenCode takes keys only once it has loaded the session
        // its plugin then reports; a spawn that resumes it is known at once.
        let opencode = definition(&catalog, "opencode").unwrap();
        let argv = words(&["opencode", "-s", "ses_e584ebfb9ffeITTA6qYHnKhdkZ"]);
        assert_eq!(started_session(opencode, &argv, true), None);
        assert_eq!(
            started_session(opencode, &argv, false).as_deref(),
            Some("ses_e584ebfb9ffeITTA6qYHnKhdkZ")
        );
        let claude = definition(&catalog, "claude").unwrap();
        let argv = words(&["claude", "--resume", "0b8f1f3e-5c2a-4a8e-9a54-2d4c1f6b7e90"]);
        assert_eq!(
            started_session(claude, &argv, false).as_deref(),
            Some("0b8f1f3e-5c2a-4a8e-9a54-2d4c1f6b7e90")
        );
    }

    #[test]
    fn an_agent_with_screen_rules_hands_its_holder_the_built_in_set() {
        let catalog = catalog();
        let rules = screen_rules(definition(&catalog, "claude").unwrap()).unwrap();
        assert_eq!(rules.agent, "claude-code");
        assert_eq!(rules.sets.len(), 1);
        assert_eq!(rules.sets[0].source, "built-in");
        let manifest: hide_screen_rules::Manifest =
            serde_json::from_str(&rules.sets[0].manifest).unwrap();
        assert!(manifest.compile().is_ok());
        assert!(screen_rules(definition(&catalog, "omp").unwrap()).is_none());
    }
}
