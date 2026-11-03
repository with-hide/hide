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

use crate::engine::answers::PaneSessionKind;

/// How long a start the engine typed may take to show its program in the
/// pane's foreground before it is taken as a start that did not happen:
/// the deadline Hide gives Herdr's starts.
pub(super) const START_DEADLINE: std::time::Duration = crate::agent_start::AGENT_START_DEADLINE;

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

/// The definition whose `executables` name `program`, by the name its
/// holder gave it (`hide_holder::foreground::program_name`: its first
/// argument's file name, or the script a runtime runs).
fn recognized<'c>(catalog: &'c Catalog, program: &ForegroundProgram) -> Option<&'c Definition> {
    catalog.definitions.iter().find(|definition| {
        definition
            .executables
            .iter()
            .any(|executable| executable.eq_ignore_ascii_case(&program.name))
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
    /// When the engine typed its start, until its program showed.
    pub(super) started: Option<Instant>,
    /// Its program is in the pane's foreground.
    pub(super) shown: bool,
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
        self.live.started = None;
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
    if before.title != facts.title {
        applied.changed = true;
    }
    let program = facts.foreground.as_ref();
    let foreground = program.and_then(|program| recognized(catalog, program));
    match (agent.as_mut(), program, foreground) {
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
    applied.rules_for = live.rules_due(agent.as_ref());
    if let Some(current) = agent.as_mut().filter(|current| current.live.shown) {
        let status = status_of(&facts.screen, current.live.status);
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
            argv: vec![name.to_owned()],
        })
    }

    fn read(state: ScreenState) -> ScreenRead {
        ScreenRead::Read {
            state,
            rule: Some("rule".to_owned()),
        }
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
    fn a_start_waits_for_its_program_and_a_screen_without_rules_is_unknown() {
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
        assert_eq!(agent.live.status, Status::Unknown);
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
