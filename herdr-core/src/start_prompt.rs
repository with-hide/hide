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
                > hide_platform::process::WINDOWS_COMMAND_LINE_LIMIT =>
        {
            Err(Refusal::Start(
                "its Windows command line would pass the system's limit".into(),
            ))
        }
        Some(StartForm::Program | StartForm::Direct { .. }) => Ok(Some(words)),
    }
}

/// How the pane's node starts `kind`'s program now (`Call::AgentInstalled`),
/// asked before every start, since an update of the agent can move what its
/// launcher runs: `None` when the program is not on the node's `PATH`.
pub(crate) fn start_form(
    node: &dyn crate::node_access::NodeLink,
    kind: &str,
    timeout: std::time::Duration,
) -> Result<Option<StartForm>, crate::node_access::LinkError> {
    crate::node_access::call_as::<Option<StartForm>>(
        node,
        hide_node_link::protocol::Call::AgentInstalled {
            name: kind.to_owned(),
        },
        timeout,
    )
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
/// Herdr types the command into the pane's shell, and the slowest shell
/// measured bounds it: through the pinned Herdr, macOS bash took 27 s to take
/// a 76 KB command and did not finish 300 KB within 60 s, where zsh took 5 s;
/// 64 KB stays well inside the 120 s start wait on either.
pub(crate) const MAX_PROMPT_BYTES: usize = 64 * 1024;

/// The first prompt as one argument Herdr can type into the pane's shell,
/// which refuses a line break or a tab in an argument. Each line break becomes
/// U+2028 (LINE SEPARATOR), which Herdr passes through and the model reads as
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
    use super::*;

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
}
