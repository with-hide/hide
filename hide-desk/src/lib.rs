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

/// The first message a session gets: Desk's instructions, the conversation
/// carried from the agent before (when the operator switched agents), then
/// the operator's message.
pub fn first_message(carried: Option<&str>, message: &str) -> String {
    let mut text = String::from(INSTRUCTIONS);
    if let Some(carried) = carried.filter(|carried| !carried.is_empty()) {
        text.push_str("\nThe conversation so far, with the agent the operator used before you:\n<conversation>\n");
        text.push_str(carried);
        text.push_str("\n</conversation>\n");
    }
    text.push_str("\nThe operator's message:\n");
    text.push_str(message);
    text
}
