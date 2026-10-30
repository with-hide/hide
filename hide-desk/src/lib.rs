//! Desk's agent boundary (PRD Desk 1차): the ACP client that talks to the
//! agent the operator chose, the adapters and how each is held in its ask
//! mode, their installation, and what Hide answers when the agent asks to
//! run something. It keeps no state of its own: the core's Desk owns the
//! conversation, the choices and the one [`host::Host`] it runs.

pub mod adapter;
pub mod host;
pub mod install;
pub mod permission;
pub mod rpc;
pub mod session;

pub use adapter::AdapterId;

/// What the Desk agent is told before the operator's first message in a
/// session: its role, that it acts through the `hide` CLI, and to answer in
/// the operator's language. It is code-owned: Desk reads none of the
/// operator's own agent instructions, settings or skills (PRD Desk D-53).
pub const INSTRUCTIONS: &str = include_str!("instructions.md");

/// Where the operator's Settings › Desk put each task's work (PRD Desk
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

/// A later message in a session, with the setting's line before it when the
/// operator changed it since the session was told.
pub fn next_message(placement: Option<Placement>, message: &str) -> String {
    match placement {
        Some(placement) => format!("{}\n\nThe operator's message:\n{message}", placement.line()),
        None => message.to_owned(),
    }
}

/// The first message a session gets: Desk's instructions, where work goes,
/// the conversation carried from the agent before (when the operator
/// switched agents), then the operator's message.
pub fn first_message(placement: Placement, carried: Option<&str>, message: &str) -> String {
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

/// The most sessions one reply's chips name.
pub const MAX_REPLY_MENTIONS: usize = 16;

/// The sessions a reply names as `@session(<pane> <name>)`, in order, each
/// once, at most [`MAX_REPLY_MENTIONS`].
pub fn session_mentions(text: &str) -> Vec<(String, String)> {
    const OPEN: &str = "@session(";
    let mut found: Vec<(String, String)> = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        rest = &rest[start + OPEN.len()..];
        let Some(end) = rest.find(')') else {
            break;
        };
        let inner = &rest[..end];
        rest = &rest[end + 1..];
        let Some((pane, name)) = inner.trim().split_once(char::is_whitespace) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || found.iter().any(|(seen, _)| seen == pane) {
            continue;
        }
        found.push((pane.to_owned(), name.to_owned()));
        if found.len() == MAX_REPLY_MENTIONS {
            break;
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
