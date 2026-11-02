//! What a start can carry as the agent's prompt, the one rule for every
//! start Hide makes with one: a spawn's brief (`coordination::executor`), a
//! task's first prompt (`worktree_control`) and, through the spawn's brief, a
//! promoted Memory card (`runtime::memory_promotion`). A caller that gets a
//! [`Refusal`] has its own way on: the brief is typed once the agent runs, a
//! first prompt fails the start with the reason.

use hide_node_link::protocol::StartForm;

/// Why a start cannot carry a prompt.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The prompt itself: a control character, or past [`MAX_PROMPT_BYTES`].
    Prompt(String),
    /// The start: the agent has no start form, its native arguments give a
    /// prompt already, its machine could not say how it starts, or it starts
    /// through `cmd.exe`.
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
/// carry `prompt` as one argument in the agent's own form
/// (`LaunchDialect::prompt_form`). `form` says how the pane's machine starts
/// the agent's program (`Call::AgentInstalled`) and is asked last, only for a
/// prompt every other check let through. On Windows a program that is a
/// batch file, npm's `.cmd` shim of every CLI among them, runs through
/// `cmd.exe`, which reads the start line again: it refuses a line past 8,191
/// characters, expands `%`, and a shim may read its arguments again, so such
/// a start carries no prompt.
pub(crate) fn carried(
    kind: &str,
    native: &[String],
    prompt: &str,
    form: impl FnOnce() -> Result<StartForm, String>,
) -> Result<Vec<String>, Refusal> {
    let dialect = hide_agent_adapter::adapter(kind)
        .and_then(|row| row.start)
        .ok_or_else(|| Refusal::Start(format!("{kind} has no start Hide makes")))?;
    if dialect.carries_prompt(native) {
        return Err(Refusal::Start(
            "its native arguments give a prompt already".into(),
        ));
    }
    let argument = prompt_argument(prompt).map_err(Refusal::Prompt)?;
    match form().map_err(Refusal::Start)? {
        StartForm::Program => Ok(dialect.prompt_arguments(&argument)),
        StartForm::Batch => Err(Refusal::Start(format!(
            "{kind} starts through cmd.exe on this Windows machine, which reads a prompt in its arguments again"
        ))),
    }
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
    let mut argument = String::with_capacity(prompt.len());
    let mut chars = prompt.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                chars.next_if_eq(&'\n');
                argument.push('\u{2028}');
            }
            '\n' => argument.push('\u{2028}'),
            '\t' => argument.push_str("    "),
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

    fn program() -> Result<StartForm, String> {
        Ok(StartForm::Program)
    }

    #[test]
    fn a_prompt_rides_the_start_as_one_word_only_where_the_start_carries_it_whole() {
        let words = |kind: &str, native: &[&str], prompt: &str| {
            let native: Vec<String> = native.iter().map(|word| (*word).to_owned()).collect();
            carried(kind, &native, prompt, program)
        };
        assert_eq!(
            words("opencode", &[], "-go\tnow").unwrap(),
            ["--prompt=-go    now"]
        );
        assert_eq!(words("codex", &[], "-x\ny").unwrap(), ["--", "-x\u{2028}y"]);
        assert_eq!(words("antigravity", &[], "@a").unwrap(), ["-i", "@a"]);
        // A whole brief, 16 KiB, rides the start.
        let brief = "x".repeat(16 * 1024);
        assert_eq!(
            words("claude", &[], &brief).unwrap(),
            ["--", brief.as_str()]
        );
        assert!(matches!(
            words("codex", &["--", "mine"], "go"),
            Err(Refusal::Start(_))
        ));
        assert!(matches!(
            words("codex", &[], "a\u{7}b"),
            Err(Refusal::Prompt(_))
        ));
        assert!(matches!(words("cursor", &[], "go"), Err(Refusal::Start(_))));
    }

    #[test]
    fn a_batch_start_carries_no_prompt_and_the_form_is_asked_only_for_a_prompt_it_could_carry() {
        let batch = carried("codex", &[], "go", || Ok(StartForm::Batch));
        assert!(
            matches!(batch, Err(Refusal::Start(ref why)) if why.contains("cmd.exe")),
            "{batch:?}"
        );
        let unknown = carried("codex", &[], "go", || Err("link_lost".into()));
        assert_eq!(unknown, Err(Refusal::Start("link_lost".into())));
        let asked = std::cell::Cell::new(false);
        let form = || {
            asked.set(true);
            program()
        };
        assert!(carried("codex", &[], "a\u{7}b", form).is_err());
        assert!(!asked.get(), "a refused prompt asks the machine nothing");
    }
}
