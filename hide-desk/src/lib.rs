//! Desk's agent boundary (PRD Desk 1차): the ACP client that talks to the
//! agent the operator chose, the adapters and how each starts, their
//! installation, and how Hide reads what the agent asks to run. It keeps no state of its own: the core's Desk owns the
//! conversation, the choices and the one [`host::Host`] it runs.

pub mod adapter;
pub mod commands;
pub mod host;
pub mod install;
pub mod permission;
pub mod rpc;
pub mod session;

pub use adapter::AdapterId;

/// What the Desk agent is told before the operator's first message in a
/// session: its role, that it acts through the `hide` CLI, and to answer in
/// the operator's language. It is code-owned and comes on top of the agent's
/// own setup, which Desk starts as the operator's terminal does (PRD Desk
/// D-92).
pub const INSTRUCTIONS: &str = include_str!("instructions.md");

/// Where the operator's Settings › Labs › Desk put each task's work (PRD Desk
/// B19).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Placement {
    /// Desk decides per task, as its instructions say.
    PerTask,
    Worktree,
    Checkout,
}

impl Placement {
    /// The line that tells the agent the setting; a place the operator names
    /// in a message still wins for that task.
    pub fn line(self) -> &'static str {
        match self {
            Self::PerTask => {
                "The operator's setting for where work goes: decide per task, as above. A place the operator names for a task wins for that task."
            }
            Self::Worktree => {
                "The operator's setting for where work goes: every task on a new branch named `desk/<task name>`. A place the operator names for a task wins for that task."
            }
            Self::Checkout => {
                "The operator's setting for where work goes: every task on the checkout's current branch. A place the operator names for a task wins for that task."
            }
        }
    }
}

/// What one session has been told of Desk's guide, which decides how each
/// message goes to it. A session that opened new gets the guide
/// ([`INSTRUCTIONS`]), where work goes and the conversation carried from
/// the agent before with its first ordinary message; a later message gets
/// the line of where work goes before it when the operator changed that
/// setting since the session was told. A message that runs one of the
/// agent's commands ([`commands::Kind::Command`]) goes as written, with
/// nothing before it, so the agent reads it as its command: what it would
/// have carried goes with the next ordinary message.
#[derive(Debug, Default)]
pub struct Guide {
    /// The guide is still to be told, with the conversation carried.
    pending: Option<Option<String>>,
    /// The setting the session was last told.
    placement: Option<Placement>,
}

impl Guide {
    /// A session that opened new, with the conversation it carries.
    pub fn new_session(carried: Option<String>) -> Self {
        Self {
            pending: Some(carried),
            placement: None,
        }
    }

    /// A session loaded again, which was told the guide when it was new.
    pub fn resumed() -> Self {
        Self::default()
    }

    /// The text the session gets for the operator's `message`.
    pub fn message(&mut self, placement: Placement, kind: commands::Kind, message: &str) -> String {
        if kind == commands::Kind::Command {
            return message.to_owned();
        }
        let told = self.placement.replace(placement);
        match self.pending.take() {
            Some(carried) => first_message(placement, carried.as_deref(), message),
            None if told != Some(placement) => {
                format!("{}\n\nThe operator's message:\n{message}", placement.line())
            }
            None => message.to_owned(),
        }
    }
}

/// The first message a session gets: Desk's instructions, where work goes,
/// the conversation carried from the agent before (when the operator
/// switched agents), then the operator's message.
fn first_message(placement: Placement, carried: Option<&str>, message: &str) -> String {
    let mut text = String::from(INSTRUCTIONS);
    text.push('\n');
    text.push_str(placement.line());
    text.push('\n');
    if let Some(carried) = carried.filter(|carried| !carried.is_empty()) {
        text.push_str("\nThe conversation so far, with the agent the operator used before you:\n<conversation>\n");
        text.push_str(carried);
        text.push_str("\n</conversation>\n");
    }
    text.push_str("\nThe operator's message:\n");
    text.push_str(message);
    text
}

/// What a plain refusal's message asks of the agent: no other way to the
/// same, and words for the operator.
pub const REFUSED_DO_NOT_RETRY: &str = "Do not try the same thing another way. Tell the operator in words that it did not run, and what you would do instead.";

/// The message that tells the agent the operator refused `command`, a
/// `kind` call (PRD Desk D-83), sent once the turn the refusal stopped ends:
/// with the line `거절…` carried, or, for a plain refusal an agent ended its
/// turn on, none, when it must not try the same another way.
pub fn refusal_message(kind: permission::CardKind, command: &str, line: Option<&str>) -> String {
    let what = match kind {
        permission::CardKind::Command => "shell command",
        permission::CardKind::Edit => "file change",
        permission::CardKind::Read => "file read",
        permission::CardKind::Other => "tool call",
    };
    let refused = format!(
        "The operator refused this {what} and did not let it run:\n<refused>\n{command}\n</refused>\n"
    );
    match line {
        Some(line) => format!("{refused}The operator's message:\n{line}"),
        None => format!("{refused}{REFUSED_DO_NOT_RETRY}"),
    }
}

/// The sessions a reply names as `@session(<pane> <name>)`, in order, each
/// once. Every one is a chip (PRD session-docs-and-assistant D-86): the
/// reply's own byte cap bounds how many it can name, so no count cap leaves a
/// later one as raw text.
pub fn session_mentions(text: &str) -> Vec<(String, String)> {
    mentions(text, "@session(")
}

/// The missions a reply names as `@mission(<id> <title>)`, read as
/// [`session_mentions`] reads sessions.
pub fn mission_mentions(text: &str) -> Vec<(String, String)> {
    mentions(text, "@mission(")
}

/// Each `<open><id> <name>)` in `text`, in order and once per id. The name
/// may hold parentheses of its own, `(11-25)`, as long as they close; a
/// token the line ends before it closes, or with no name, stays text.
fn mentions(text: &str, open: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(open) {
        rest = &rest[start + open.len()..];
        let Some(end) = closing(rest) else {
            continue;
        };
        let inner = &rest[..end];
        rest = &rest[end + 1..];
        let Some((id, name)) = inner.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || found.iter().any(|(seen, _)| seen == id) {
            continue;
        }
        found.push((id.to_owned(), name.to_owned()));
    }
    found
}

/// Where the parenthesis already open before `text` closes, on its line.
fn closing(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (at, char) in text.char_indices() {
        match char {
            '\n' => return None,
            '(' => depth += 1,
            ')' if depth == 0 => return Some(at),
            ')' => depth -= 1,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use commands::Kind;

    #[test]
    fn a_plain_refusal_forbids_another_way_and_a_line_is_the_operator_s_word() {
        let plain = refusal_message(permission::CardKind::Edit, "notes.md", None);
        assert!(plain.contains("file change"));
        assert!(plain.contains("<refused>\nnotes.md\n</refused>"));
        assert!(plain.ends_with(REFUSED_DO_NOT_RETRY));
        // What the sentence must keep saying, whatever its wording: no
        // other way to the same, and words that it did not run.
        assert!(REFUSED_DO_NOT_RETRY.contains("another way"));
        assert!(REFUSED_DO_NOT_RETRY.contains("did not run"));
        assert!(!plain.contains("The operator's message"));

        let line = refusal_message(
            permission::CardKind::Command,
            "rm -rf build",
            Some("keep build, clean dist"),
        );
        assert!(line.ends_with("The operator's message:\nkeep build, clean dist"));
        assert!(!line.contains(REFUSED_DO_NOT_RETRY));
    }

    #[test]
    fn a_new_session_s_first_command_goes_alone_and_the_guide_goes_with_the_next_message() {
        let mut guide = Guide::new_session(Some("operator: hi".into()));
        assert_eq!(
            guide.message(Placement::Worktree, Kind::Command, "/compact"),
            "/compact"
        );
        assert_eq!(
            guide.message(Placement::Worktree, Kind::Command, "/context"),
            "/context"
        );
        let next = guide.message(Placement::Worktree, Kind::Message, "fix the login");
        assert!(next.starts_with(INSTRUCTIONS), "{next}");
        assert!(next.contains(Placement::Worktree.line()));
        assert!(next.contains("<conversation>\noperator: hi\n</conversation>"));
        assert!(next.ends_with("The operator's message:\nfix the login"));
        // Told once: the guide goes no more.
        assert_eq!(
            guide.message(Placement::Worktree, Kind::Message, "and the docs"),
            "and the docs"
        );
    }

    #[test]
    fn a_new_session_s_first_message_carries_the_guide() {
        let mut guide = Guide::new_session(None);
        let first = guide.message(Placement::PerTask, Kind::Message, "hello");
        assert!(first.starts_with(INSTRUCTIONS));
        assert!(!first.contains("<conversation>"));
        assert_eq!(
            guide.message(Placement::PerTask, Kind::Message, "again"),
            "again"
        );
    }

    #[test]
    fn a_changed_setting_s_line_waits_past_a_command_for_the_next_message() {
        let mut guide = Guide::resumed();
        // A loaded session was told the guide once; the setting's line goes
        // before its first message here.
        assert_eq!(
            guide.message(Placement::Checkout, Kind::Message, "go"),
            format!(
                "{}\n\nThe operator's message:\ngo",
                Placement::Checkout.line()
            )
        );
        assert_eq!(
            guide.message(Placement::Checkout, Kind::Message, "on"),
            "on"
        );
        assert_eq!(
            guide.message(Placement::Worktree, Kind::Command, "/compact"),
            "/compact"
        );
        assert_eq!(
            guide.message(Placement::Worktree, Kind::Message, "next"),
            format!(
                "{}\n\nThe operator's message:\nnext",
                Placement::Worktree.line()
            )
        );
    }

    #[test]
    fn a_reply_s_missions_are_read_with_parentheses_in_their_titles() {
        let text = "@mission(mission-63 결제 흐름 정리 (11-25)) runs; again \
                    @mission(mission-63 결제 흐름 정리 (11-25)), @mission(mission-9) and \
                    @mission(mission-7 open (one\nline) then @mission(mission-8 last)";
        assert_eq!(
            mission_mentions(text),
            [
                ("mission-63".to_owned(), "결제 흐름 정리 (11-25)".to_owned()),
                ("mission-8".to_owned(), "last".to_owned()),
            ]
        );
    }

    #[test]
    fn a_reply_s_session_names_are_read_once_each_in_order() {
        let text = "@session(w1:p2 fix login) is done; @session(w3:p1 docs) waits. \
                    Again @session(w1:p2 fix login), then @session(broken) and @session(w4:p1 tail";
        assert_eq!(
            session_mentions(text),
            [
                ("w1:p2".to_owned(), "fix login".to_owned()),
                ("w3:p1".to_owned(), "docs".to_owned()),
            ]
        );
    }

    #[test]
    fn every_session_a_long_reply_names_is_read_past_the_sixteenth() {
        let text: String = (1..=40)
            .map(|n| format!("- @session(w{n}:p1 job-{n}) runs\n"))
            .collect();
        let found = session_mentions(&text);
        assert_eq!(found.len(), 40);
        assert_eq!(found[16], ("w17:p1".to_owned(), "job-17".to_owned()));
        assert_eq!(found[39], ("w40:p1".to_owned(), "job-40".to_owned()));
    }
}
