//! The one launch decision for every start Hide makes with a first input,
//! a spawn's brief (`coordination::executor`), a task's first prompt
//! (`worktree_control`) and, through the spawn's brief, a promoted Memory
//! card (`runtime::memory_promotion`), under the standard of
//! docs/delivery.md, The input ladder: typed once the agent's screen takes
//! input, and launched as the agent's start argument only in the cells the
//! agent journey proves.

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
/// this machine (`here`), on macOS or Linux: a device's start and every start
/// on Windows, where the journey starts each CLI through npm's `.cmd`
/// launcher, are typed until a cell proves them.
pub(crate) fn launched(
    kind: &str,
    native: &[String],
    prompt: &str,
    here: bool,
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
    if cfg!(windows) {
        return Err(Refusal::Start(
            "no Windows journey cell proves a launch".into(),
        ));
    }
    Ok(Some(words))
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

    #[test]
    fn a_first_input_launches_only_where_the_agent_declares_it_and_the_journey_proves_it() {
        let words = |kind: &str, native: &[&str], prompt: &str, here: bool| {
            let native: Vec<String> = native.iter().map(|word| (*word).to_owned()).collect();
            launched(kind, &native, prompt, here)
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
        let launched_here = words("codex", &[], "-x\ny", true);
        if cfg!(windows) {
            assert!(matches!(launched_here, Err(Refusal::Start(_))));
        } else {
            assert_eq!(
                launched_here,
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
    }
}
