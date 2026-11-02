//! What a start can carry as the agent's prompt, the one rule for every
//! start Hide makes with one: a spawn's brief (`coordination::executor`), a
//! task's first prompt (`worktree_control`) and, through the spawn's brief, a
//! promoted Memory card (`runtime::memory_promotion`). A caller that gets a
//! [`Refusal`] has its own way on: the brief is typed once the agent runs, a
//! first prompt fails the start with the reason. A spawn's brief also asks
//! how the machine starts the program, since it can be typed instead
//! (`executor::launch_prompt`).

/// Why a start cannot carry a prompt.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// The prompt itself: a control character, or past [`MAX_PROMPT_BYTES`].
    Prompt(String),
    /// The start: the agent has no start form, or its native arguments give
    /// a prompt already.
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
/// (`LaunchDialect::prompt_form`).
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
    fn a_prompt_rides_the_start_as_one_word_only_where_the_start_carries_it_whole() {
        let words = |kind: &str, native: &[&str], prompt: &str| {
            let native: Vec<String> = native.iter().map(|word| (*word).to_owned()).collect();
            carried(kind, &native, prompt)
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
}
