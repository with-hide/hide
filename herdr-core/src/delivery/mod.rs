//! Durable agent delivery owned by the core.
pub(crate) mod bell;
pub(crate) mod composer;
pub(crate) mod doorbell;
mod human;
pub mod ledger;
pub mod mailbox;
pub mod watch;
pub mod worker;

pub mod answer;

pub use mailbox::Command;

pub const DELIVERY_EXPIRY_MS: u64 = 60 * 60 * 1_000;
/// How long a `request` or `block` letter waits for its answer, from the time
/// it was sent, before the wait ends as `deadline`.
pub const ANSWER_WAIT_MS: u64 = 24 * 60 * 60 * 1_000;
pub const INPUT_QUIET_MS: u64 = 30_000;
pub const TICK_MS: u64 = 60_000;
pub const INACTIVITY_MS: u64 = 20 * 60_000;
/// How long a watch waits before telling a pane observer that its target
/// stopped on a question it asked only in its own pane, or that Herdr holds
/// it at a blocked prompt: one tick, the grace in which the target may still
/// go on by itself.
pub const QUESTION_WINDOW_MS: u64 = TICK_MS;
pub const SECOND_WARNING_MS: u64 = 60 * 60_000;
/// How long a finished letter stays after it finished. It exists so a retried
/// send with the same intent returns the first letter, which happens within
/// minutes; a full ledger drops finished letters sooner (`mailbox::send`).
pub const RETENTION_MS: u64 = 24 * 60 * 60_000;
pub const OPEN_LIMIT: usize = 1_024;
pub const LETTER_LIMIT: usize = 5_000;
pub const WATCH_LIMIT: usize = 32;
pub const BODY_LIMIT: usize = 16 * 1_024;
pub const FILE_LIMIT: usize = 16 * 1_024 * 1_024;
pub const HOOK_LIMIT: usize = 8 * 1_024;
pub const HOOK_LETTERS: usize = 5;

#[derive(
    Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
pub struct Actor {
    pub pane_id: String,
    pub name: String,
    pub kind: String,
    pub device_id: String,
    pub session: Option<String>,
}

pub const FACTORY_PREFIX: &str = "factory:";
/// The kind of Hide's own code-owned identity: the origin of the agents
/// Hide starts for the operator rather than for a pane, such as a Memory
/// card's promotion into the repository (PRD project-memory-v2 D-25). It has
/// no pane and reads no letters; only the core constructs one.
pub const HIDE_KIND: &str = "hide";
pub const HIDE_PREFIX: &str = "hide:";
/// The one role Hide starts agents as today.
pub const HIDE_MEMORY_ROLE: &str = "memory";
/// The kind of the operator's own code-owned identity: the sender of the
/// feedback letters the desktop app's Send makes from the operator's View
/// comments (PRD view-opener-feedback D-17). It has no pane, reads no
/// letters and is constructed only by the core, for the shell's own Send.
pub const OPERATOR_KIND: &str = "operator";
pub const OPERATOR_PREFIX: &str = "operator:";
/// The one operator identity: the person at the screen.
pub const OPERATOR_NAME: &str = "operator";

impl Actor {
    /// Hide's own code-owned identity for `role` on `node`, the core's own.
    pub fn hide(role: &str, node: &str) -> Self {
        let name = format!("{HIDE_PREFIX}{role}");
        Self {
            pane_id: name.clone(),
            name: name.clone(),
            kind: HIDE_KIND.into(),
            device_id: node.into(),
            session: crate::wire::session_digest(&name),
        }
    }

    /// The operator's own code-owned identity on `node`, the core's own:
    /// the sender of a feedback letter (PRD view-opener-feedback D-17).
    pub fn operator(node: &str) -> Self {
        let pane_id = format!("{OPERATOR_PREFIX}screen");
        Self {
            pane_id: pane_id.clone(),
            name: OPERATOR_NAME.into(),
            kind: OPERATOR_KIND.into(),
            device_id: node.into(),
            session: crate::wire::session_digest(&pane_id),
        }
    }

    /// Whether this is Hide's or the operator's code-owned identity.
    pub fn code_owned(&self) -> bool {
        (self.kind == HIDE_KIND && self.pane_id.starts_with(HIDE_PREFIX)) || self.operator_owned()
    }

    /// Whether this is the operator's own identity.
    pub fn operator_owned(&self) -> bool {
        self.kind == OPERATOR_KIND && self.pane_id.starts_with(OPERATOR_PREFIX)
    }

    /// Whether this is Hide's own code-owned identity, the origin of the work
    /// Hide starts itself, as opposed to a pane's.
    pub fn hide_owned(&self) -> bool {
        self.kind == HIDE_KIND && self.pane_id.starts_with(HIDE_PREFIX)
    }

    /// Mailbox authority requires a positively observed native session. Watch
    /// targets may lack it and use status-only activity, without mailbox access.
    pub(crate) fn require_native_identity(&self) -> Result<(), String> {
        if self.valid() && self.session.is_some() {
            Ok(())
        } else {
            Err("native_identity_required".into())
        }
    }

    pub(crate) fn valid(&self) -> bool {
        valid_key(&self.pane_id)
            && valid_key(&self.name)
            && valid_key(&self.kind)
            && valid_key(&self.device_id)
            && self.session.as_deref().is_none_or(valid_key)
    }

    pub(crate) fn same_identity(&self, other: &Self) -> bool {
        self.pane_id == other.pane_id
            && self.device_id == other.device_id
            && self.session == other.session
    }
}

/// A name or pane a pane may never claim: it would read as a Factory, as
/// Hide itself or as the operator.
pub(crate) fn reserved_name(value: &str) -> bool {
    // A name that reads as the operator's once trimmed or in another case is
    // the operator's too: the labeler trims a sender's name.
    let operator = value.trim().to_lowercase();
    value.starts_with(FACTORY_PREFIX)
        || value.starts_with(HIDE_PREFIX)
        || operator.starts_with(OPERATOR_PREFIX)
        || operator == OPERATOR_NAME
}

/// A pane's native session as a letter pull names it: the helper's own
/// rule, so the CLI, the daemon and the hook agree.
pub use hide_agent_hooks::delivery::{
    SESSION_LIMIT, prompt_digest, valid_prompt_digest, valid_session,
};

pub(crate) fn valid_key(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}
