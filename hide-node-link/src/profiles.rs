//! The agent profiles a node keeps for its machine (PRD agent-profiles D-04,
//! D-24, D-29, D-30; docs/ARCHITECTURE.md, The node's profile file).
//!
//! Each machine's node owns one profile file and answers the core's three
//! calls on it: [`crate::protocol::Call::ProfileList`],
//! [`crate::protocol::Call::ProfileApply`] and
//! [`crate::protocol::Call::ProfileInstall`]. A node that predates them
//! refuses them unrun, and the core then offers only each agent's built-in
//! default profile on that machine.

use std::collections::BTreeMap;

use hide_agent_adapter::{AccountRef, ProfileId};
use hide_kit::ProfilePlace;
use serde::{Deserialize, Serialize};

/// The most profiles one machine keeps, the built-in ones aside (D-29).
pub const MAX_PROFILES: usize = 64;

/// A change to a machine's profiles. The account a profile runs in is fixed
/// when it is added (D-24), so an edit names only the rest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ProfileOp {
    /// A new profile of the agent `kind`. A folder may start with `~`, which
    /// the node expands under its home; `None` runs in the agent's default
    /// account, with a program or variables of its own.
    Add {
        kind: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<AccountRef>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env: Vec<(String, String)>,
        /// It becomes the agent's default profile on this machine.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        default: bool,
    },
    /// The profile's name, program and variables, each replaced whole.
    Edit {
        id: ProfileId,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        program: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        env: Vec<(String, String)>,
    },
    /// The profile a start of the agent `kind` uses when nothing else names
    /// one (D-10); the built-in default's id puts that one back.
    Default { kind: String, id: ProfileId },
    /// Removes the profile: Hide's pieces leave its account once no other
    /// profile runs there, and the profile goes with them. The core decides
    /// before it asks whether an agent still runs with it (D-30).
    Remove { id: ProfileId },
}

/// One profile as a node lists it: no variable's value unless the call
/// asked for them, for the screen that edits it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedProfile {
    pub id: ProfileId,
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    /// The names of its variables, in its order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_keys: Vec<String>,
    /// Its variables with their values, only when the call asked for them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<(String, String)>>,
    pub version: u64,
    /// Being removed: no start may use it (D-30).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removing: bool,
    /// Where its pieces are, and their state, as the kit last judged them;
    /// `None` when the kit could not be asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<ProfilePlace>,
}

/// A machine's profiles.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileList {
    /// Raised by every change, and named by the next
    /// [`crate::protocol::Call::ProfileApply`], which a change in between
    /// refuses as a conflict.
    pub version: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<ListedProfile>,
    /// Each agent's default profile on this machine, where it is not the
    /// built-in one.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, ProfileId>,
    /// Why the kit could not judge the profiles' pieces, for the log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unjudged: Option<String>,
}

/// What a [`ProfileOp`] did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileApplied {
    /// Why nothing changed, under the field the operator fixes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<ProfileRefusal>,
    /// The profile an `add` made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<ProfileId>,
    /// The profiles after the change, or as they are after a refusal.
    pub list: ProfileList,
}

/// A refused change, worded for the field it is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRefusal {
    pub field: ProfileField,
    pub text: String,
    /// The variable it is about, a name that passed the name check (D-29).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_key: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileField {
    Kind,
    Name,
    Account,
    Program,
    Env,
    /// The profile as a whole: one the machine has no room for, one that is
    /// gone or being removed, or one the node's own variables refuse.
    Profile,
}
