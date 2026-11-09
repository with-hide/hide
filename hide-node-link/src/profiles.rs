//! The agent profiles a node keeps for its machine (PRD agent-profiles D-04,
//! D-24, D-29, D-30; docs/ARCHITECTURE.md, The node's profile file).
//!
//! Each machine's node owns one profile file and answers the core's four
//! calls on it: [`crate::protocol::Call::ProfileList`],
//! [`crate::protocol::Call::ProfileApply`],
//! [`crate::protocol::Call::ProfileInstall`] and, for a start,
//! [`crate::protocol::Call::ProfileLaunch`]. A node that predates them
//! refuses them unrun, and the core then offers only each agent's built-in
//! default profile on that machine.

use std::collections::BTreeMap;

use hide_agent_adapter::{AccountKey, AccountRef, ProfileId};
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
    /// Why, as a code a screen words in its own language; `text` stays the
    /// CLI's and the log's. A node before codes sends none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<ProfileReason>,
    /// The bound a `*_long`, `profiles_full` or `env_too_many` reason
    /// crossed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,
}

/// Why a node refused a profile change (`contracts/snapshot-wire-enums.json`:
/// `profile_reason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileReason {
    /// Hide keeps no profiles for the agent, or the profile is another's.
    KindUnknown,
    ProfilesFull,
    NameEmpty,
    NameLong,
    NameControl,
    /// The name is the built-in profile's.
    NameBuiltin,
    /// Another profile of the agent has the name (D-30).
    NameTaken,
    ProgramLong,
    /// The built-in profile is neither changed nor removed.
    ProfileBuiltin,
    ProfileMissing,
    ProfileRemoving,
    /// A default named for one agent is another agent's profile.
    ProfileOtherKind,
    /// The node's own value of the agent's variable is unusable.
    NodeVariable,
    /// The account starts at a home the place has none of.
    AccountHomeMissing,
    /// The agent's account cannot be moved.
    AccountUnsupported,
    /// A folder is needed and a name was given.
    AccountNeedsFolder,
    /// A name is needed and a folder was given.
    AccountNeedsName,
    AccountEmpty,
    AccountRelative,
    /// The folder is where the built-in profile reads.
    AccountIsDefault,
    AccountName,
    /// The account folder could not be made.
    AccountUnprepared,
    EnvTooMany,
    EnvName,
    /// The variable is one the account, the home or Hide owns.
    EnvReserved,
    EnvRepeated,
    EnvValue,
    /// A code a newer node named that this build does not know.
    #[serde(other)]
    Other,
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

/// A node's answer to [`crate::protocol::Call::ProfileLaunch`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProfileLaunch {
    /// The start runs so.
    Ready(ProfileStart),
    /// This machine has no such profile any more.
    Gone,
    /// The profile cannot start now; `text` says why, naming the profile and,
    /// when it is the cause, its program.
    Refused { text: String },
}

/// How a start of one profile's agent runs on its machine, as that
/// machine's node checked it just now: what the start line of `hide agent
/// exec` carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileStart {
    /// The profile's name, which the start line and its failures name.
    pub name: String,
    /// The account the agent runs in (D-15).
    pub account: AccountKey,
    /// The profile's version the answer was made from.
    pub version: u64,
    /// The variables the start sets: the account's, then the profile's own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<(String, String)>,
    /// The profile's own program, found on the machine, run in place of the
    /// agent's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<ProfileProgram>,
    /// The absolute path of the `hide` beside the node, which runs the start
    /// (`hide agent exec`).
    pub exec: String,
}

/// A profile's program as its machine starts it: the file, and the words
/// before the agent's arguments (npm's Windows launcher read as the program
/// it runs, `hide_platform::programs::Start`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileProgram {
    pub file: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub leading: Vec<String>,
}
