//! Durable agent delivery owned by the core.
pub(crate) mod bell;
pub(crate) mod composer;
pub mod correspondence;
pub(crate) mod doorbell;
pub(crate) mod held;
mod human;
pub(crate) mod ladder;
pub mod ledger;
pub mod mailbox;
pub mod store;
pub mod typed;
pub mod watch;
pub mod worker;

pub mod answer;

pub use mailbox::Command;

/// How long a `request` or `block` letter waits for its answer, from the time
/// it was sent, before the wait ends as `deadline`.
pub const ANSWER_WAIT_MS: u64 = 24 * 60 * 60 * 1_000;
/// How long a pane rests, with no key Hide routed and no change of the
/// engine's status, before the doorbell types into it or reads its composer.
pub const INPUT_QUIET_MS: u64 = 30_000;
pub const TICK_MS: u64 = 60_000;
pub const INACTIVITY_MS: u64 = 20 * 60_000;
/// How long a watch waits before telling a pane observer that its target
/// stopped on a question it asked only in its own pane: one tick, the grace
/// in which the target may still go on by itself.
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
/// How the list of Views a report's sender opened begins, which the core
/// appends to the report's body as it is sent (`Runtime::list_opened_views`):
/// a retry of the report is the body its sender wrote, without the list.
pub(crate) const OPENED_VIEWS_HEAD: &str = "\n\nViews this agent opened (read one with `hide view show <view-id>` or `hide browser snapshot <view-id>`; open it again from your own pane to show it to the operator):";
/// The most bytes the ledger encodes to (`Ledger::size`): the limit its
/// records are kept within by dropping the finished ones oldest first
/// (`coordination::room::for_size`), which bounds the store's rows
/// (`delivery::store`), and the size `delivery-ledger.json` was read up to.
pub const FILE_LIMIT: usize = 16 * 1_024 * 1_024;
pub const HOOK_LIMIT: usize = 8 * 1_024;
/// The most letters one handover bundle confirms, shown and folded together
/// (`mailbox::render`). Every letter awaiting intake is open, so a bundle of
/// all of them stays under it.
pub const BUNDLE_LETTERS: usize = hide_agent_hooks::delivery::BUNDLE_LETTERS;
const _: () = assert!(OPEN_LIMIT <= BUNDLE_LETTERS);
/// How many of one sender's letters a bundle shows in full, newest first,
/// beside its questions and blocks, which it always shows.
pub const SHOWN_PER_SENDER: usize = 3;

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
/// Desk's agent (PRD Desk 1차): the origin of the sessions it starts, and
/// the observer of their watches.
pub const HIDE_DESK_ROLE: &str = "desk";
/// The role of an external tool's identity (PRD hide-external-control
/// D-10): `hide:access:<connection id>`, one per connection, which acts as
/// Desk's does, and only while its connection stands at Act.
pub const HIDE_ACCESS_ROLE: &str = "access";
/// The role of another machine's agent as this core knows it (PRD
/// device-connect-iroh D-72): `hide:core:<device>:agent|pane:<id>`, which is
/// `hide:` before that agent's address on its own core
/// (`hide_node_link::address::CoreAddress`), one identity per agent. It
/// stands as the sender of what that agent asks of this core, and as the
/// recipient of what this core's agents write to it.
pub const HIDE_CORE_ROLE: &str = "core";
/// The most bytes a ledger key holds, a name among them.
const KEY_BYTES: usize = 256;
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
            session: crate::engine::tokens::session_digest(&name),
        }
    }

    /// An external tool's code-owned identity on `node`, the core's own
    /// machine (D-07): its pane slot names the connection by id, so a new
    /// connection under the same name is another identity (D-40), and its
    /// name carries the tool's name, which a letter's recipient reads as
    /// the sender (B23).
    pub fn access(connection: &str, name: &str, node: &str) -> Self {
        let name = format!("{HIDE_PREFIX}{HIDE_ACCESS_ROLE}:{name}");
        Self {
            // A code-owned record keeps its name as its session
            // (`register_code_owned`), so the digest is of the name.
            session: crate::engine::tokens::session_digest(&name),
            pane_id: format!("{HIDE_PREFIX}{HIDE_ACCESS_ROLE}:{connection}"),
            name,
            kind: HIDE_KIND.into(),
            device_id: node.into(),
        }
    }

    /// The connection an external tool's identity acts for, by id.
    pub fn access_connection(&self) -> Option<&str> {
        if self.kind != HIDE_KIND {
            return None;
        }
        self.pane_id
            .strip_prefix(HIDE_PREFIX)?
            .strip_prefix(HIDE_ACCESS_ROLE)?
            .strip_prefix(':')
            .filter(|id| !id.is_empty())
    }

    /// The name an external tool connected with, for its identity.
    pub fn access_name(&self) -> Option<&str> {
        self.access_connection()?;
        self.name
            .strip_prefix(HIDE_PREFIX)?
            .strip_prefix(HIDE_ACCESS_ROLE)?
            .strip_prefix(':')
    }

    /// Another machine's agent as this core, on `node`, knows it (PRD
    /// device-connect-iroh D-72): `address` is the agent on its own core,
    /// `machine` that machine's name and `name` the agent's name there, so a
    /// letter's recipient reads which machine wrote it. Its pane slot is the
    /// address, which names one agent of one machine whatever either is
    /// called later, and its session is of that slot on the connection
    /// `key` ([`Self::core_caller`]), so the same agent is the same identity
    /// however it was named when it was written to, and an agent of another
    /// connection that says the same device id is another identity.
    pub fn core(
        address: crate::address::CoreAddress<'_>,
        key: &str,
        machine: &str,
        name: &str,
        node: &str,
    ) -> Self {
        let pane_id = format!("{HIDE_PREFIX}{address}");
        Self {
            session: crate::engine::tokens::session_digest(&Self::core_caller(key, &pane_id)),
            name: Self::core_name(machine, name),
            pane_id,
            kind: HIDE_KIND.into(),
            device_id: node.into(),
        }
    }

    /// The caller an agent of another core acts as here, which its session
    /// is the digest of: the connection id its machine proved, then its
    /// slot. A device id is only what a machine says of itself, and two
    /// connections may say the same one; the connection id is what keeps
    /// their agents, and what is stored for them, apart.
    pub fn core_caller(key: &str, pane_id: &str) -> String {
        format!("{key}:{pane_id}")
    }

    /// Whether this is an agent of the core on connection `key`.
    pub fn core_of(&self, key: &str) -> bool {
        self.core_address().is_some()
            && self.session
                == crate::engine::tokens::session_digest(&Self::core_caller(key, &self.pane_id))
    }

    /// The name another machine's agent goes by here: its machine's name,
    /// then its own, cut to what a ledger key holds, since the other core
    /// chose it.
    pub fn core_name(machine: &str, name: &str) -> String {
        let mut full = format!("{HIDE_PREFIX}{HIDE_CORE_ROLE}:{machine}:");
        for character in name.chars().filter(|character| !character.is_control()) {
            if full.len() + character.len_utf8() > KEY_BYTES {
                break;
            }
            full.push(character);
        }
        full
    }

    pub fn core_address(&self) -> Option<crate::address::CoreAddress<'_>> {
        if self.kind != HIDE_KIND {
            return None;
        }
        crate::address::CoreAddress::parse(self.pane_id.strip_prefix(HIDE_PREFIX)?)?.ok()
    }

    /// Who an answer to this actor names panes to: an agent of another core
    /// (`Actor::core`, whose machine is this core's node) reads them as this
    /// core's, any other caller from its own machine.
    pub(crate) fn pane_reader(&self) -> crate::address::Reader<'_> {
        if self.core_address().is_some() {
            crate::address::Reader::Core(&self.device_id)
        } else {
            crate::address::Reader::On(&self.device_id)
        }
    }

    /// This actor as an answer to `reader` shows it: its pane as that reader
    /// uses it (`address::pane_for_caller`), from the machine it runs on and
    /// that machine's own id for it. A code-owned identity (Hide's own, a
    /// tool's, the operator's) and a guest's pane name no pane of this core
    /// and stay as they are.
    pub(crate) fn read_by(&self, reader: crate::address::Reader<'_>) -> Actor {
        use crate::address::PaneAddress;
        let mut actor = self.clone();
        if !self.code_owned()
            && !self.pane_id.starts_with(HIDE_PREFIX)
            && let Ok(pane @ (PaneAddress::Node(_) | PaneAddress::Device { .. })) =
                PaneAddress::parse(&self.pane_id)
        {
            actor.pane_id =
                crate::address::pane_for_caller(reader, &self.device_id, pane.device_pane_id());
        }
        actor
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
            session: crate::engine::tokens::session_digest(&pane_id),
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

    /// Whether this is Hide itself: Hide's own identity and never an
    /// external tool's or another machine's agent's, which are code-owned
    /// too ([`Self::access_connection`], [`Self::core_address`]). A gate
    /// that only Hide may pass asks this, not [`Self::hide_owned`].
    pub fn hide_itself(&self) -> bool {
        self.hide_owned() && self.access_connection().is_none() && self.core_address().is_none()
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
    !value.is_empty() && value.len() <= KEY_BYTES && !value.chars().any(char::is_control)
}
