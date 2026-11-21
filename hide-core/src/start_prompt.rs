//! The one launch decision for every start Hide makes with a first input,
//! a spawn's brief (`coordination::executor`), a task's first prompt
//! (`worktree_control`) and, through the spawn's brief, a promoted Memory
//! card (`runtime::memory_promotion`), under the standard of
//! docs/delivery.md, The input ladder: typed once the agent's screen takes
//! input, and launched as the agent's start argument only in the cells the
//! agent journey proves.

use hide_node_link::protocol::StartForm;

/// Why a start that would launch its first input types it instead.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The prompt itself: a control character, or past [`MAX_PROMPT_BYTES`];
    /// a task refuses such a prompt before anything starts.
    Prompt(String),
    /// The start: its native arguments give a prompt already, or it runs
    /// where no journey cell proves a launch.
    Start(String),
}

impl Refusal {
    pub(crate) fn detail(&self) -> &str {
        match self {
            Self::Prompt(detail) | Self::Start(detail) => detail,
        }
    }
}

/// The words a start of `kind`, whose native arguments are `native`, adds to
/// carry `prompt` as its launch prompt, in the agent's own form
/// (`LaunchDialect::prompt_form`); `Ok(None)` for an agent that does not
/// declare the launch prompt, typed as every input is by default. The
/// journey proves a launch only for an agent that declares it, started on
/// this machine (`here`) as `form` says its node starts the program
/// ([`start_form`]): by its name on macOS and Linux, by its file on Windows,
/// npm's launcher read as the program it runs. A device's start, a start
/// whose node did not say, one whose program Windows runs through `cmd.exe`
/// (`StartForm::Batch`), and one whose Windows command line would pass the
/// system's limit are typed.
pub(crate) fn launched(
    kind: &str,
    native: &[String],
    prompt: &str,
    here: bool,
    form: Option<&StartForm>,
) -> Result<Option<Vec<String>>, Refusal> {
    if !hide_agent_adapter::adapter(kind).is_some_and(|row| row.launch_prompt()) {
        return Ok(None);
    }
    let words = carried(kind, native, prompt)?;
    if !here {
        return Err(Refusal::Start(
            "it starts on a device, where no journey cell proves a launch".into(),
        ));
    }
    match form {
        None => Err(Refusal::Start(
            "the pane's machine did not say how the agent's program starts".into(),
        )),
        Some(StartForm::Batch) => Err(Refusal::Start(
            "its program is a batch file that is not npm's launcher, which cmd.exe would read again".into(),
        )),
        Some(StartForm::Direct { program, leading })
            if windows_command_line(program, leading, native, &words)
                > hide_shell::quote::WINDOWS_COMMAND_LINE_LIMIT =>
        {
            Err(Refusal::Start(
                "its Windows command line would pass the system's limit".into(),
            ))
        }
        Some(StartForm::Program | StartForm::Direct { .. }) => Ok(Some(words)),
    }
}

/// How the pane's node starts the program `program` now
/// (`Call::AgentInstalled`), asked before every start, since an update of the
/// agent can move what its launcher runs: `None` when the program is not on
/// the node's `PATH`. `program` is the executable a definition starts the
/// agent by ([`program_of`]), not the agent's kind: the node looks the name
/// up on its `PATH`, and Cursor's kind (`cursor`) is not its program
/// (`cursor-agent`), nor Kiro's (`kiro`) its first (`kiro-cli`).
pub(crate) fn start_form(
    node: &dyn crate::node_access::NodeLink,
    program: &str,
    timeout: std::time::Duration,
) -> Result<Option<StartForm>, crate::node_access::LinkError> {
    crate::node_access::call_as::<Option<StartForm>>(
        node,
        hide_node_link::protocol::Call::AgentInstalled {
            name: program.to_owned(),
        },
        timeout,
    )
}

/// How long a start waits for its node to say how the program starts.
pub(crate) const START_FORM_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// The executable the engine types for a start of `kind`: its definition's
/// first (`Engine::start_program_name`), the name every question to a node
/// about how the agent starts carries. `kind` itself where the engine holds
/// no definition for it, which the node then does not find.
pub(crate) fn program_of(engine: &dyn crate::engine::Engine, kind: &str) -> String {
    engine
        .start_program_name(kind)
        .unwrap_or_else(|| kind.to_owned())
}

/// Why a start names its program by its name, not by a file the node read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ByName {
    /// The node could not be asked, or did not answer: logged as
    /// `start.form_unread`.
    NodeUnanswered,
    /// The node starts the program by its name: macOS and Linux, or a
    /// Windows batch file that is not npm's launcher.
    AsNamed,
    /// The program is not on the node's `PATH`.
    NotFound,
}

impl ByName {
    /// The word a diagnostic records for it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NodeUnanswered => "node_unanswered",
            Self::AsNamed => "starts_by_name",
            Self::NotFound => "not_found",
        }
    }
}

/// The file a start of `program` names on the pane's node now
/// ([`start_form`]), for a start that carries no first input, a Wake's, a
/// fork's, a reopen's or a restart's resume: on Windows the program npm's
/// launcher runs, since PowerShell would find npm's `.ps1` by the name and
/// Windows PowerShell 5.1 drops the quotes inside an argument it hands on
/// (`--settings '{"key":true}'`). Where the node starts the program by its
/// name, did not find it, or could not be asked (which is logged), the start
/// goes by its name and the answer says which.
pub(crate) fn start_program_file(
    node: &dyn crate::node_access::NodeLink,
    program: &str,
) -> Result<crate::engine::StartProgram, ByName> {
    match start_form(node, program, START_FORM_TIMEOUT) {
        Ok(Some(form)) => crate::engine::StartProgram::of(program, &form).ok_or(ByName::AsNamed),
        Ok(None) => Err(ByName::NotFound),
        Err(error) => {
            crate::diagnostic!(serde_json::json!({
                "component": "agent_start", "kind": "start.form_unread",
                "program": program, "cause": error.to_string(),
            }));
            Err(ByName::NodeUnanswered)
        }
    }
}

/// The file a start of the `kind` agent names on the pane's node, if any:
/// [`start_program_file`] of the program the engine types for `kind`
/// ([`program_of`]), for a wake, a fork or a reopen, which hold a kind and
/// not a program.
pub(crate) fn start_program_of(
    engine: &dyn crate::engine::Engine,
    node: &dyn crate::node_access::NodeLink,
    kind: &str,
) -> Option<crate::engine::StartProgram> {
    start_program_file(node, &program_of(engine, kind)).ok()
}

/// The UTF-16 units of the command line a start of `program` with these
/// words gives Windows, its terminating null included, as `Start-Process`
/// builds it (the program quoted, then the arguments quoted for the C
/// runtime), with room for the few words the engine adds (`codex_launch`).
fn windows_command_line(
    program: &str,
    leading: &[String],
    native: &[String],
    words: &[String],
) -> usize {
    /// The engine's own words: a definition's `[start]` arguments and the
    /// Codex daemon flag, a few dozen units today.
    const ENGINE_WORDS: usize = 256;
    let arguments = leading
        .iter()
        .chain(native)
        .chain(words)
        .map(|word| hide_shell::quote::windows_argument(word))
        .collect::<Vec<_>>()
        .join(" ");
    program.encode_utf16().count() + 2 + 1 + arguments.encode_utf16().count() + 1 + ENGINE_WORDS
}

/// The words a start of `kind` adds to carry `prompt` as one argument in the
/// agent's own form, whatever its declaration: [`launched`]'s, and a launched
/// record an earlier build saved without them.
pub(crate) fn carried(kind: &str, native: &[String], prompt: &str) -> Result<Vec<String>, Refusal> {
    let dialect = hide_agent_adapter::adapter(kind)
        .and_then(|row| row.start)
        .ok_or_else(|| Refusal::Start(format!("{kind} has no start Hide makes")))?;
    if dialect.carries_prompt(native) {
        return Err(Refusal::Start(
            "its native arguments give a prompt already".into(),
        ));
    }
    let argument = prompt_argument(prompt).map_err(Refusal::Prompt)?;
    Ok(dialect.prompt_arguments(&argument))
}

/// The longest first prompt, in bytes once encoded, that a start carries.
/// The engine types the command into the pane's shell, and the slowest shell
/// measured bounds it: through Herdr's `agent.start`, macOS bash took 27 s to take
/// a 76 KB command and did not finish 300 KB within 60 s, where zsh took 5 s;
/// 64 KB stays well inside the 120 s start wait on either.
pub(crate) const MAX_PROMPT_BYTES: usize = 64 * 1024;

/// The first prompt as one argument the engine can type into the pane's shell,
/// which refuses a line break or a tab in an argument. Each line break becomes
/// U+2028 (LINE SEPARATOR), which the engine passes through and the model reads as
/// a line break, and a tab four spaces; any other control character is refused
/// rather than dropped, and so is a prompt over [`MAX_PROMPT_BYTES`]. Every
/// start event checks its prompt with this before anything is created, so a
/// refused prompt leaves no tab or worktree behind; the worker encodes it.
pub(crate) fn prompt_argument(prompt: &str) -> Result<String, String> {
    let prompt = hide_agent_hooks::delivery::input_text(prompt);
    let mut argument = String::with_capacity(prompt.len());
    let mut chars = prompt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                chars.next_if_eq(&'\n');
                argument.push('\u{2028}');
            }
            '\n' => argument.push('\u{2028}'),
            c if c.is_control() => {
                return Err(format!(
                    "The first prompt has a control character (U+{:04X}) that cannot be passed to the agent; remove it and start again.",
                    u32::from(c)
                ));
            }
            c => argument.push(c),
        }
    }
    if argument.len() > MAX_PROMPT_BYTES {
        return Err(format!(
            "The first prompt is longer than {} KB, more than the agent's command line can carry; shorten it and start again.",
            MAX_PROMPT_BYTES / 1024
        ));
    }
    Ok(argument)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::node_access::{LinkAnswer, LinkError, NodeLink};

    #[test]
    fn a_launched_prompt_uses_the_same_tab_width_as_typed_and_channel_input() {
        let brief = "@missing.md\n\tIt needs no change.\n\nReply with exactly TOKEN";
        let argument = prompt_argument(brief).unwrap();
        assert_eq!(
            argument,
            "@missing.md\u{2028}    It needs no change.\u{2028}\u{2028}Reply with exactly TOKEN"
        );
        assert_eq!(
            hide_agent_hooks::delivery::prompt_digest(&argument),
            hide_agent_hooks::delivery::prompt_digest(brief)
        );
    }

    #[test]
    fn a_first_input_launches_only_where_the_agent_declares_it_and_the_journey_proves_it() {
        let by_name = StartForm::Program;
        let words = |kind: &str, native: &[&str], prompt: &str, here: bool| {
            let native: Vec<String> = native.iter().map(|word| (*word).to_owned()).collect();
            launched(kind, &native, prompt, here, Some(&by_name))
        };
        // Typed by default: OpenCode, Pi and Cursor declare no launch.
        for kind in ["opencode", "pi", "cursor"] {
            assert_eq!(words(kind, &[], "go", true), Ok(None), "{kind}");
        }
        assert!(matches!(
            words("codex", &[], "a\u{7}b", true),
            Err(Refusal::Prompt(_))
        ));
        assert!(matches!(
            words("codex", &["--", "mine"], "go", true),
            Err(Refusal::Start(_))
        ));
        assert!(matches!(
            words("codex", &[], "go", false),
            Err(Refusal::Start(_))
        ));
        assert_eq!(
            words("codex", &[], "-x\ny", true),
            Ok(Some(vec!["--".into(), "-x\u{2028}y".into()]))
        );
        assert_eq!(
            words("antigravity", &[], "@a", true),
            Ok(Some(vec!["-i".into(), "@a".into()]))
        );
        // A whole brief, 16 KiB, rides the start.
        let brief = "x".repeat(16 * 1024);
        assert_eq!(
            words("claude", &[], &brief, true),
            Ok(Some(vec!["--".into(), brief]))
        );
    }

    /// How the node starts the program decides with the rest: a Windows
    /// start that names its file launches, a program `cmd.exe` would run,
    /// a node that did not say, and a Windows command line past the
    /// system's limit are typed.
    #[test]
    fn a_first_input_rides_only_a_start_whose_node_says_it_reaches_the_program_whole() {
        let direct = StartForm::Direct {
            program: "C:\\nodejs\\node.exe".into(),
            leading: vec!["C:\\npm\\node_modules\\@openai\\codex\\bin\\codex.js".into()],
        };
        let brief = format!("-{}", "x".repeat(16 * 1024 - 1));
        assert_eq!(
            launched("codex", &[], &brief, true, Some(&direct)),
            Ok(Some(vec!["--".into(), brief.clone()]))
        );
        for form in [None, Some(&StartForm::Batch)] {
            assert!(matches!(
                launched("codex", &[], &brief, true, form),
                Err(Refusal::Start(_))
            ));
        }
        // A task's first prompt may be 64 KiB, twice what Windows takes.
        let long = "x".repeat(MAX_PROMPT_BYTES);
        assert!(matches!(
            launched("codex", &[], &long, true, Some(&direct)),
            Err(Refusal::Start(_))
        ));
        assert_eq!(
            launched("codex", &[], &long, true, Some(&StartForm::Program)),
            Ok(Some(vec!["--".into(), long]))
        );
    }

    /// A node that answers every `Call::AgentInstalled` with `answer` and
    /// keeps the program names it was asked about.
    struct Asked {
        answer: Result<serde_json::Value, ()>,
        names: Mutex<Vec<String>>,
    }

    impl Asked {
        fn new(answer: Result<serde_json::Value, ()>) -> Self {
            Self {
                answer,
                names: Mutex::default(),
            }
        }

        fn names(&self) -> Vec<String> {
            self.names.lock().unwrap().clone()
        }
    }

    impl NodeLink for Asked {
        fn call(
            &self,
            call: hide_node_link::protocol::Call,
            _: std::time::Duration,
        ) -> Result<LinkAnswer, LinkError> {
            let hide_node_link::protocol::Call::AgentInstalled { name } = call else {
                panic!("the node was asked {call:?}");
            };
            self.names.lock().unwrap().push(name);
            self.answer
                .clone()
                .map(LinkAnswer::Parsed)
                .map_err(|()| LinkError::Unknown("the node did not answer".into()))
        }
    }

    /// Issue 628: the node looks a name up on its `PATH`, so every question
    /// about how an agent starts carries the definition's first executable,
    /// not the agent's kind: Cursor's kind is `cursor` and its program
    /// `cursor-agent`, Kiro's `kiro` and `kiro-cli`, Antigravity's
    /// `antigravity` and `agy`.
    #[test]
    fn a_start_asks_its_node_about_the_definitions_first_executable_not_the_kind() {
        let fake =
            crate::engine::fake::FakeEngine::start(|method, _| panic!("unexpected {method}"));
        let engine = fake.engine();
        for (kind, program) in [
            ("cursor", "cursor-agent"),
            ("kiro", "kiro-cli"),
            ("antigravity", "agy"),
            ("claude", "claude"),
            // An alias reads as the definition it names.
            ("claude-code", "claude"),
            // A kind no definition names is asked as it is.
            ("unheard-of", "unheard-of"),
        ] {
            assert_eq!(program_of(engine.as_ref(), kind), program, "{kind}");
            let node = Asked::new(Ok(serde_json::json!("program")));
            assert!(start_program_of(engine.as_ref(), &node, kind).is_none());
            assert_eq!(node.names(), [program], "{kind}");
        }
        // A node that names the file runs it under the program's name, which is
        // what the engine names the program of a start by.
        let direct = Asked::new(Ok(
            serde_json::json!({"direct": {"program": "C:\\npm\\cursor-agent.js", "leading": ["a"]}}),
        ));
        let file = start_program_of(engine.as_ref(), &direct, "cursor").expect("the file is named");
        assert_eq!(file.name, "cursor-agent");
        assert_eq!(file.program, "C:\\npm\\cursor-agent.js");
    }

    /// Why a start went by the program's name is what a restart's resume
    /// records beside `by_file`, so a pane whose resume went by name can be told
    /// from one whose node did not answer.
    #[test]
    fn a_start_that_goes_by_the_programs_name_says_why() {
        let why = |answer: Result<serde_json::Value, ()>| {
            let node = Asked::new(answer);
            start_program_file(&node, "claude").map_err(ByName::as_str)
        };
        assert_eq!(why(Err(())), Err("node_unanswered"));
        assert_eq!(why(Ok(serde_json::json!("program"))), Err("starts_by_name"));
        assert_eq!(why(Ok(serde_json::json!("batch"))), Err("starts_by_name"));
        assert_eq!(why(Ok(serde_json::Value::Null)), Err("not_found"));
        let file = why(Ok(
            serde_json::json!({"direct": {"program": "p", "leading": []}}),
        ));
        assert_eq!(file.unwrap().program, "p");
    }
}
