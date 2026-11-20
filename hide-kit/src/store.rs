//! The store: the one place Hide keeps the skills and MCP items the
//! operator installs, and the loop that applies them to every agent
//! (PRD skill-mcp-manager).
//!
//! The store is a folder of Hide's own in the account (`layout::store_dir`),
//! so a device keeps its own: `store.json` (the [`StoreRecord`]), each
//! item's files under `items/`, the undo list's under `trash/`, and what a
//! source answered for the confirm screen under `staging/`. The store is the
//! original; an agent's folder or settings file holds a copy (D-31).
//!
//! What differs by kind is one [`Applier`]: where the kind goes in each
//! agent's accounts ([`Place`]), whether an agent takes an item, and how one
//! entry is read, written and taken out. The skill's is `skill`; the MCP
//! one is `mcp`'s. Everything else, the switches, the undo list, adoption,
//! ownership and the report, is this module's, the same for both kinds.
//!
//! Ownership is Hide's own record ([`StoreRecord::written`]): for each place,
//! the entries Hide wrote there with the digest of what it wrote. An entry
//! with no record, or whose content no longer has the recorded digest, is the
//! operator's and is never written over or taken out (B22). A pass that finds
//! everything as recorded writes nothing (B27).
//!
//! The pass runs under the kit's account lock; what reaches the network
//! (`source`) runs before it, so a slow fetch never holds another pass up.

mod avatar;
mod mcp;
mod skill;
mod source;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use hide_agent_hooks::mcp::Server;

use serde::{Deserialize, Serialize};

use crate::agent_kit::enabled;
use crate::agents::{Detection, RETIRED_SKILLS, SKILLS};
use crate::record::Record;
use crate::{KitProfiles, KitTarget, Scope};

pub use mcp::{McpBody, McpPreview, PastedPick, PastedServer, Sealed, SentKeys};
pub(crate) use source::Fetched;
pub use source::SourceSpec;

/// How many deleted items the undo list keeps; past it the oldest is
/// deleted for good (D-25).
pub const UNDO_LIMIT: usize = 20;
/// How many answers of a source the confirm screen can still install from;
/// past it the oldest is dropped with its files.
pub const PREVIEW_LIMIT: usize = 4;
/// How many folder sources the store follows (D-20): an item past it is
/// refused at install, so the follow pass stays bounded.
pub const FOLLOWED_LIMIT: usize = 64;
/// Live items are never evicted. Refuse a new item once this many are kept.
pub const ITEM_LIMIT: usize = 256;
/// How many files and bytes one skill may hold, so a repository that is not
/// a skill does not fill the account.
pub const SKILL_FILE_LIMIT: usize = 1000;
pub const SKILL_BYTE_LIMIT: u64 = 32 * 1024 * 1024;

const FORMAT: u32 = 1;

/// What an item is.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Skill,
    Mcp,
}

impl ItemKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Skill => "skill",
            Self::Mcp => "mcp",
        }
    }
}

/// Where an item came from.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    /// A folder of a git repository (GitHub's, or any the account's git
    /// reaches), fetched at `commit`. `path` is the folder in the
    /// repository, empty for its root; `reference` the branch or tag the
    /// address named, `None` for the default branch.
    Git {
        url: String,
        #[serde(default)]
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reference: Option<String>,
        commit: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
    },
    /// A folder on this machine, followed: a change there reaches every
    /// agent (D-20). For a skill adopted through a link, the folder the link
    /// led to.
    Folder { path: String },
    /// A copy taken from an agent's folder when it was adopted (D-17),
    /// kept in the store alone.
    Copy { from: String },
    /// An MCP the operator pasted; the spec is the item's body.
    Pasted,
}

/// The short version a row shows beside the name (B36).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Version {
    /// A tag of the repository that names the commit.
    Tag(String),
    /// The commit's first seven characters, with no tag naming it.
    Commit(String),
    /// A folder source ('폴더').
    Folder,
    /// A copy adopted from an agent's folder.
    Copy,
    /// A remote MCP ('원격').
    Remote,
    /// An `npx` MCP that pins no version ('npm 최신').
    NpmLatest,
    /// An MCP command that pins a version.
    Pinned(String),
    /// A local MCP that pins nothing and is no npm runner's package (a
    /// program, `uvx`, an image with no tag): the row shows no version.
    Unpinned,
}

impl Version {
    fn of_commit(commit: &str, tag: Option<&str>) -> Self {
        match tag {
            Some(tag) => Self::Tag(tag.to_owned()),
            None => Self::Commit(commit.chars().take(7).collect()),
        }
    }
}

/// One cell column of a kind's rows (B1). A skill's switch is per skill
/// folder, since every agent that reads a folder sees what is in it: Claude
/// (`~/.claude/skills` and each Claude profile's own), the shared folder and
/// agy's. An MCP's is per agent, since each agent has its own settings file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Column {
    /// The skill folder's code (`SkillDir::code`) or the agent's id.
    pub id: String,
    /// The agents the cell's switch reaches.
    pub agents: Vec<String>,
    /// Agents that read this column's folder as well as their own, so an
    /// item on here reaches them too.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<String>,
}

/// One file of a skill, as the confirm screen lists it (B6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Relative to the skill's folder, `/` between names on every system.
    pub path: String,
    pub size: u64,
    /// The system would run it, or it starts with `#!` (▲).
    pub executable: bool,
}

/// A skill's own facts.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SkillBody {
    /// The digest of every file (`skill::digest`), what a place's copy is
    /// compared with.
    pub digest: String,
    pub files: Vec<FileEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// What an item is beyond its name and switches, by kind.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemBody {
    Skill(SkillBody),
    Mcp(McpBody),
}

/// The newest version a check found for a git item (D-33), kept until the
/// next check or the update.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Latest {
    pub commit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// One item of the store.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoreItem {
    /// `<kind>:<name>`, the one id the wire and the record carry.
    pub id: String,
    pub kind: ItemKind,
    /// The skill's folder name, or the MCP server's name, in every agent.
    pub name: String,
    pub source: Source,
    /// The item's own switch ('전체').
    pub on: bool,
    /// The columns switched off for this item while it is on, by
    /// [`Column::id`]; a column not named, and so a new agent or profile it
    /// reaches, takes it (D-13, D-31).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub off_for: BTreeSet<String>,
    pub added_at_unix_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<Latest>,
    /// A folder source that is gone: the last copy stays (D-20).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub folder_missing: bool,
    /// A folder source's last stat fingerprint (`skill::fingerprint`), so a
    /// follow pass reads the files only when something changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
    /// The key of the place the operator's own copy was taken over from on
    /// this machine (B20), with a [`Source::Copy`]: unless the undo list
    /// still holds the folder or entry it replaced there, the store holds
    /// the only original, so a machine leaving Hide leaves a copy where it
    /// was (B30). The key names that place's kind, and an MCP file's agent.
    /// A copy another machine carried has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adopted: Option<String>,
    pub body: ItemBody,
}

impl StoreItem {
    /// The GitHub account a git source belongs to (the icon tile's picture).
    pub(crate) fn owner(&self) -> Option<&str> {
        match &self.source {
            Source::Git { url, .. } => avatar::owner(url),
            _ => None,
        }
    }

    pub fn id_of(kind: ItemKind, name: &str) -> String {
        format!("{}:{name}", kind.code())
    }

    /// Whether the item's switches let the column `column` have it.
    pub fn on_for(&self, column: &str) -> bool {
        self.on && !self.off_for.contains(column)
    }

    /// The digest every copy of the item is compared with.
    pub(crate) fn digest(&self) -> String {
        match &self.body {
            ItemBody::Skill(body) => body.digest.clone(),
            ItemBody::Mcp(body) => body.digest(),
        }
    }

    pub fn version(&self) -> Version {
        // An MCP's version is what its server runs, wherever it came from
        // (B36).
        if let ItemBody::Mcp(body) = &self.body {
            return body.version();
        }
        match &self.source {
            Source::Git { commit, tag, .. } => Version::of_commit(commit, tag.as_deref()),
            Source::Folder { .. } => Version::Folder,
            Source::Copy { .. } | Source::Pasted => Version::Copy,
        }
    }

    /// The newer version a check found, if any (B23).
    pub fn update(&self) -> Option<Version> {
        let Source::Git { commit, .. } = &self.source else {
            return None;
        };
        let latest = self.latest.as_ref()?;
        (latest.commit != *commit)
            .then(|| Version::of_commit(&latest.commit, latest.tag.as_deref()))
    }
}

/// Why an entry went to the undo list.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrashCause {
    /// The operator deleted the item.
    Deleted,
    /// An agent's own copy, moved whole out of `path` (the place's folder)
    /// when the operator adopted another copy of the same name, or the same
    /// one holding something a copy leaves out (B21, B22). A machine leaving
    /// Hide puts it back there (B30).
    Replaced { place: String, path: PathBuf },
}

/// One entry of the undo list; its files are under `trash/<id>`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrashEntry {
    pub id: String,
    pub item: StoreItem,
    pub at_unix_ms: u64,
    pub cause: TrashCause,
}

/// What Hide wrote at one entry of a place.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Written {
    pub item: String,
    pub digest: String,
    /// The keys Hide wrote there beside what the digest covers (an MCP
    /// server's), fingerprinted with the store's secret
    /// (`crate::mcp_keys::KeysSecret`): a key typed into the file by hand
    /// since makes the entry the operator's (B22).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<String>,
}

/// One place Hide wrote into, as the record keeps it: enough to take its
/// entries out once the place is no longer one a pass lists (a profile that
/// went, a device leaving Hide).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaceRecord {
    pub kind: Option<ItemKind>,
    pub path: PathBuf,
    /// A profile account's place, which a pass lists only while the
    /// machine's profiles can be read.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub profile: bool,
    #[serde(default)]
    pub entries: BTreeMap<String, Written>,
}

/// `store.json`.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoreRecord {
    format: u32,
    #[serde(default)]
    pub items: Vec<StoreItem>,
    /// Newest first, at most [`UNDO_LIMIT`].
    #[serde(default)]
    pub trash: Vec<TrashEntry>,
    /// What Hide wrote where, by the place's key.
    #[serde(default)]
    pub written: BTreeMap<String, PlaceRecord>,
    /// When the last update check finished (B36).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at_unix_ms: Option<u64>,
    /// What sources answered, oldest first, at most [`PREVIEW_LIMIT`].
    #[serde(default)]
    pub previews: Vec<Preview>,
    /// What updates would change, oldest first, at most [`PREVIEW_LIMIT`].
    #[serde(default)]
    pub diffs: Vec<UpdateDiff>,
    /// When each agent's copy of each item last changed, by item id and
    /// agent, so the core can tell which open sessions see it only in a
    /// new session (B17).
    #[serde(default)]
    pub changed: BTreeMap<String, BTreeMap<String, u64>>,
    /// What MCP pastes read as, oldest first (`mcp::trim_previews`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_previews: Vec<mcp::McpPreview>,
    /// The places whose copy of an item Hide wrote before the item's keys
    /// changed, by item id: the reconcile writes them again (B12, B26).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stale: BTreeMap<String, BTreeSet<String>>,
}

impl StoreRecord {
    pub fn item(&self, id: &str) -> Option<&StoreItem> {
        self.items.iter().find(|item| item.id == id)
    }

    fn item_mut(&mut self, id: &str) -> Option<&mut StoreItem> {
        self.items.iter_mut().find(|item| item.id == id)
    }
}

/// The operator's change to a machine's store, one cell or button of the
/// screen or one `hide skill` command. The wire and the device protocol
/// carry it as it is.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StoreChange {
    /// Read a source for the confirm screen (B3, B5): its skills land in
    /// [`StoreReport::previews`].
    Inspect { source: SourceSpec },
    /// Install the named skills of a preview (B7).
    Install { preview: String, names: Vec<String> },
    /// The item's switch ('전체', `column: None`) or one column's (B15,
    /// B16).
    Set {
        item: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        column: Option<String>,
        on: bool,
    },
    /// Take the item out of every agent and into the undo list (B18).
    Delete { item: String },
    /// Put an undo entry back (B18).
    Restore { entry: String },
    /// Delete an undo entry for good (B18).
    Forget { entry: String },
    /// Adopt what agents hold under `name` (B20); `from` names the place
    /// whose copy is used when the copies differ (B21).
    Adopt {
        kind: ItemKind,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
    },
    /// Ask every git item's repository for a newer version (D-33).
    CheckUpdates,
    /// Read what an update would change (B23): lands in
    /// [`StoreReport::diffs`].
    PreviewUpdate { item: String },
    /// Take the update a preview showed, at that commit (B23).
    Update { item: String, commit: String },
    /// Take in an item another machine's store carried, with its content
    /// (B28, D-15): only the core's copy job makes it, never the screen.
    Receive { item: Box<CarriedItem> },
    /// Read what the operator pasted into the MCP add field (B8): its
    /// servers land in [`StoreReport::mcp_previews`] with the names of their
    /// keys, and the values it gave stay in the store.
    ReadMcp { text: Sealed },
    /// Add the MCP server the operator confirmed (B8-B10): `server` as the
    /// fields say, each key typed on the screen, else the value the paste
    /// `from` gave.
    AddMcp {
        server: Server,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        keys: SentKeys,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<PastedPick>,
    },
    /// The operator closed a paste's answer: the values it staged go now,
    /// rather than at the hour's end (D-11). A paste already gone is no
    /// error.
    CancelMcp { preview: String },
    /// Change an MCP item's server and keys (B12); a key not typed keeps
    /// its value.
    SetMcp {
        item: String,
        server: Server,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        keys: SentKeys,
    },
}

impl StoreChange {
    /// The change as an outcome, a log line or the screen may hold it: what
    /// only the store may keep (a carried skill's file bytes, an MCP
    /// server's key values) emptied (D-11, B13). Every variant is named, so
    /// a new one that carries a value has to say how it is emptied.
    pub fn redacted(&self) -> StoreChange {
        match self {
            StoreChange::Receive { item } => StoreChange::Receive {
                item: Box::new(item.redacted()),
            },
            StoreChange::ReadMcp { text } => StoreChange::ReadMcp {
                text: text.emptied(),
            },
            StoreChange::AddMcp { server, keys, from } => StoreChange::AddMcp {
                server: server.clone(),
                keys: mcp::emptied(keys),
                from: from.clone(),
            },
            StoreChange::SetMcp { item, server, keys } => StoreChange::SetMcp {
                item: item.clone(),
                server: server.clone(),
                keys: mcp::emptied(keys),
            },
            StoreChange::Inspect { .. }
            | StoreChange::Install { .. }
            | StoreChange::Set { .. }
            | StoreChange::Delete { .. }
            | StoreChange::Restore { .. }
            | StoreChange::Forget { .. }
            | StoreChange::Adopt { .. }
            | StoreChange::CheckUpdates
            | StoreChange::PreviewUpdate { .. }
            | StoreChange::Update { .. }
            | StoreChange::CancelMcp { .. } => self.clone(),
        }
    }
}

/// An item as one machine's store hands it to another's (B28): what the
/// receiving store keeps of it, and its kind's content.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CarriedItem {
    pub kind: ItemKind,
    pub name: String,
    /// Where it came from; a folder the receiving machine cannot follow
    /// arrives as a copy.
    pub source: Source,
    pub on: bool,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub off_for: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<Latest>,
    pub content: Carried,
}

impl CarriedItem {
    fn redacted(&self) -> Self {
        Self {
            content: self.content.emptied(),
            ..self.clone()
        }
    }
}

/// What a kind carries of an item between machines, written and read by its
/// [`Applier::export`] and [`Applier::import`].
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Carried {
    /// A skill's files.
    Skill { files: Vec<CarriedFile> },
    /// An MCP server's spec and its key values; the MCP half names the keys'
    /// type and how they are emptied.
    Mcp {
        body: McpBody,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        keys: mcp::SentKeys,
    },
}

impl Carried {
    /// The content with every value emptied: a skill's files keep their
    /// paths and no bytes, an MCP server keeps its spec and no key values.
    fn emptied(&self) -> Self {
        match self {
            Carried::Skill { files } => Carried::Skill {
                files: files
                    .iter()
                    .map(|file| CarriedFile {
                        data: String::new(),
                        ..file.clone()
                    })
                    .collect(),
            },
            Carried::Mcp { body, keys } => Carried::Mcp {
                body: body.clone(),
                keys: mcp::emptied(keys),
            },
        }
    }

    /// The bytes it carries, which a copy is capped by.
    fn size(&self) -> u64 {
        match self {
            Carried::Skill { files } => files.iter().map(|file| file.data.len() as u64).sum(),
            Carried::Mcp { keys, .. } => keys
                .iter()
                .map(|(name, value)| (name.len() + value.len()) as u64)
                .sum(),
        }
    }
}

/// One file of a carried skill.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CarriedFile {
    /// Relative to the skill's folder, `/` between names.
    pub path: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub executable: bool,
    /// The file's bytes, base64.
    pub data: String,
}

/// This machine's store as a copy carries it to another (B28), or why it
/// could not be read.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoreExport {
    pub items: Vec<CarriedItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<StoreRefusal>,
}

/// The most a copy carries in encoded bytes: one device call holds it whole
/// under the helper's request limit.
pub const COPY_BYTE_LIMIT: u64 = 30 * 1024 * 1024;

/// Why a change was refused. Most changes refuse before anything is done;
/// a take-over refused partway (an agent's copy it could not set aside)
/// keeps what it had done, which the pass's report shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreRefusalCause {
    /// The item, entry, preview or copy it names is not there.
    NotFound,
    /// The source holds no skill (B3: '이 주소에서 스킬을 찾지 못함').
    NoSkills,
    /// The account's git cannot read the repository (B4).
    NoAccess,
    /// The repository's host did not answer.
    Unreachable,
    /// The account has no `git`.
    NoGit,
    /// A skill is larger than [`SKILL_FILE_LIMIT`] or [`SKILL_BYTE_LIMIT`].
    TooLarge,
    /// An item of that name is already in the store, or the name is Hide's
    /// own skill's (B35).
    NameTaken,
    /// The agents hold different copies; the operator picks one (B21).
    Differs,
    /// The store has [`ITEM_LIMIT`] items or follows [`FOLLOWED_LIMIT`] folders already.
    TooMany,
    /// The kind cannot do this (an MCP has no update).
    Unsupported,
    /// Reading or writing failed; the reason says how.
    Failed,
}

/// How one change of a pass ended.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChangeOutcome {
    pub change: StoreChange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<StoreRefusal>,
    /// What the change made for its asker to show, found by the request and
    /// never by time: an inspect's preview, a paste's MCP preview, an update
    /// preview's diff. Another request's reads never stand in for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoreRefusal {
    pub cause: StoreRefusalCause,
    /// One sentence for the log and the CLI.
    pub reason: String,
}

impl StoreRefusal {
    pub(crate) fn new(cause: StoreRefusalCause, reason: impl Into<String>) -> Self {
        Self {
            cause,
            reason: reason.into(),
        }
    }
}

/// One skill a source holds, as the confirm screen shows it (B3, B6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FoundSkill {
    pub name: String,
    /// Its folder in the repository or the chosen folder, `/` between
    /// names, empty for the root.
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub files: Vec<FileEntry>,
    pub digest: String,
    /// An item of that name is already in the store.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub taken: bool,
}

/// What a source answered (B3-B6).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Preview {
    pub id: String,
    pub source: SourceSpec,
    /// The repository's address as git reads it, `None` for a folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub skills: Vec<FoundSkill>,
    /// The agents an installed skill would reach now (B6).
    pub agents: Vec<String>,
    pub at_unix_ms: u64,
}

/// How a file differs in an update.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub change: FileChangeKind,
    pub added: u32,
    pub removed: u32,
}

/// What an update of a git item would change (B23).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UpdateDiff {
    pub id: String,
    pub item: String,
    pub from: Version,
    pub to: Version,
    pub commit: String,
    pub files: Vec<FileChange>,
    /// Files that are executable in the update and were not before (▲).
    pub new_executables: Vec<String>,
    /// The subjects of the commits between the two versions that touched the
    /// skill, newest first, at most 20.
    pub summary: Vec<String>,
    pub at_unix_ms: u64,
}

/// One agent's cell of an item's row (B1).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellState {
    /// In every account of the agent.
    On,
    /// Switched off.
    Off,
    /// On, but not put in yet: none of the column's agents is found here,
    /// or the agent has not made its folder; it goes in once it is (D-31).
    Waiting,
    /// An entry of that name the operator put there is in the column's
    /// folder or file; Hide left it alone (B22).
    HandWritten,
    /// Writing or taking it out failed; the next pass tries again (B26).
    Failed,
    /// The column's agent does not take this item (B14).
    Unsupported,
    /// Every agent of the column is switched off in Settings › Agents
    /// (B25).
    AgentOff,
}

/// Why a cell failed, for the screen to word in one short line; the
/// kit's sentence is the cell's `reason` and the log's.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CellFailure {
    /// The agent's file or folder is not in a shape Hide reads, so Hide
    /// left it untouched.
    Unreadable,
    /// Writing the copy or taking it out failed.
    Unwritable,
    /// The item's keys could not be read for the write.
    Keys,
    /// The copy is not yet where its switch says; the next pass tries
    /// again.
    Pending,
}

/// Why a place could not be judged or changed.
#[derive(Clone, Debug)]
struct Failure {
    cause: CellFailure,
    reason: String,
}

impl Failure {
    fn new(cause: CellFailure, reason: impl Into<String>) -> Self {
        Self {
            cause,
            reason: reason.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// The [`Column::id`].
    pub column: String,
    pub state: CellState,
    /// Why, for a failed, waiting, hand-written or unsupported cell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// What kind of failure a failed cell's is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<CellFailure>,
    /// When the column's copy last changed (B17).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_at_unix_ms: Option<u64>,
    /// The key of the place the state speaks of, which a change names:
    /// the one holding the operator's own copy of a hand-written cell
    /// (what [`StoreChange::Adopt`] takes over), the one that failed or
    /// waits, otherwise the first the column governs. `None` for a column
    /// with no place on this machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
    /// The real path of `place`, the agent's file or folder, for the
    /// screen to name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A hand-written MCP cell's server as the agent's own file at `place`
    /// holds it, key-less: what taking it over brings in (B20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<Server>,
}

/// One item's row.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ItemReport {
    pub id: String,
    pub kind: ItemKind,
    pub name: String,
    /// The GitHub account a git source belongs to, whose kept picture is
    /// in [`StoreReport::avatars`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    pub source: Source,
    pub version: Version,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<Version>,
    pub on: bool,
    /// The columns switched off while the item is on ([`StoreItem::off_for`]),
    /// so the screen sends a column's own switch, or tries a failed cell
    /// again, with what the operator last set there (B16, B26).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub off_for: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub folder_missing: bool,
    /// A skill's description, as its `SKILL.md` gives it, for the unfolded
    /// row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The skill's files, for the confirm screens.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileEntry>,
    /// The MCP server as the edit screen shows it: how it is reached and
    /// the names of its keys, never a value (B12, D-34).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<Server>,
    pub cells: Vec<Cell>,
}

/// One copy of an entry Hide did not write.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FoundCopy {
    /// The place's key, which [`StoreChange::Adopt`] names.
    pub place: String,
    pub agents: Vec<String>,
    /// The profiles whose account holds it, empty for the default account.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<String>,
    pub path: String,
    /// The folder a link there leads to (D-20).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// Empty when the copy could not be read.
    pub digest: String,
    /// Why this copy cannot be taken over (too large, unreadable); the
    /// folder's other entries are listed as ever (B19).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// An MCP copy's server as this place holds it, key-less: what taking
    /// it over brings in (B20). None for a skill or an unreadable copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server: Option<Server>,
}

/// An entry agents hold that Hide does not manage ('에이전트에만 있음', B19).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Unmanaged {
    pub kind: ItemKind,
    pub name: String,
    pub copies: Vec<FoundCopy>,
    /// Every copy was read and has the same content; otherwise adoption
    /// asks which (B21).
    pub same: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrashReport {
    pub id: String,
    pub kind: ItemKind,
    pub name: String,
    pub at_unix_ms: u64,
    pub cause: TrashCause,
}

/// A machine's store after a pass or a read.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoreReport {
    /// Each kind's columns, in the order its cells are.
    #[serde(default)]
    pub columns: BTreeMap<ItemKind, Vec<Column>>,
    pub items: Vec<ItemReport>,
    #[serde(default)]
    pub unmanaged: Vec<Unmanaged>,
    #[serde(default)]
    pub trash: Vec<TrashReport>,
    #[serde(default)]
    pub previews: Vec<Preview>,
    #[serde(default)]
    pub diffs: Vec<UpdateDiff>,
    /// What MCP pastes read as, for the add screen (B8, B9).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_previews: Vec<McpPreview>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at_unix_ms: Option<u64>,
    /// The store follows a folder, so a light pass every few seconds keeps
    /// the agents up with it (D-33).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub follows: bool,
    /// Each change of the pass, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<ChangeOutcome>,
    /// The store could not be read or locked, so nothing was judged or
    /// changed; the sentence says why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
    /// What kind of refusal `refused` is, for the screen to word: the log
    /// keeps the sentence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<StorePassRefusal>,
    /// The machine's home folder, for the screen to spell the paths under
    /// it with `~`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// The kept pictures of the items' GitHub owners ([`ItemReport::owner`]),
    /// as data addresses.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub avatars: BTreeMap<String, String>,
}

/// Why a whole pass did not run (`StoreReport::refused`).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorePassRefusal {
    /// The store's key secret its record needs is gone: every pass is
    /// refused and nothing is taken out, until the operator takes Hide's
    /// MCP servers out of the agents' files and starts the store over
    /// (docs/agent-hooks.md).
    KeysSecretGone,
    /// The store's folder, record or secret could not be read or saved.
    Unreadable,
}

// --- Places and the per-kind seam ------------------------------------------------

/// One place a kind goes into: a skills folder, an MCP settings file, of one
/// account, with the columns whose switches govern it.
#[derive(Clone, Debug)]
pub(crate) struct Place {
    /// `<kind>:<real path>`, stable while the place is: two spellings of
    /// one folder are one place.
    pub(crate) key: String,
    /// The real path, links on the way followed.
    pub(crate) path: PathBuf,
    /// The [`Column::id`]s whose switches govern it: more than one when two
    /// columns' folders are one folder (one a link to the other). The item
    /// is there while any of them wants it.
    pub(crate) columns: Vec<String>,
    /// The profiles whose account this is, empty for the default account.
    pub(crate) profiles: Vec<String>,
    /// Why nothing can be written here now (the agent has not made its
    /// folder, a profile account Hide does not write); the place's agents
    /// wait (`CellState::Waiting`).
    pub(crate) blocked: Option<String>,
    /// Read every entry's content again. False only for the read every few
    /// seconds (`crate::status`), which takes the digest an entry was last
    /// read at while its stat fingerprint has not moved; a pass that may
    /// change the place reads it.
    pub(crate) fresh: bool,
}

impl Place {
    pub(crate) fn governed_by(&self, column: &str) -> bool {
        self.columns.iter().any(|governing| governing == column)
    }
}

/// What a place holds under one name.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Entry {
    pub(crate) digest: String,
    /// The folder a link there leads to.
    pub(crate) link: Option<PathBuf>,
    /// Nothing in it is left out of a copy (a `.git` folder, a link
    /// inside): only then can Hide's copy stand for it, and only then is it
    /// ever Hide's to replace or take out.
    pub(crate) whole: bool,
    /// The keys it holds beside what its digest covers (an MCP server's
    /// variables and headers); none for a kind without keys. A copy is an
    /// item, or the one Hide wrote, only when these agree too.
    pub(crate) keys: Option<mcp::KeyValues>,
    /// An MCP server's own fields as the agent's file holds them, read with
    /// the entry: the command and arguments or the address, and its keys'
    /// names, never a value. None for a skill or an unreadable server.
    pub(crate) server: Option<Server>,
}

/// One entry as a place's listing read it, or why it could not be read.
pub(crate) type Listed = (String, Result<Entry, String>);

/// What differs by kind. Everything else of the store is the same for each.
pub(crate) trait Applier {
    fn kind(&self) -> ItemKind;
    /// The kind's cell columns, in their order.
    fn columns(&self) -> Vec<Column>;
    /// Every place of the kind on the machine.
    fn places(&self, ctx: &Ctx<'_>) -> Vec<Place>;
    /// Whether the column `column` takes `item`, or why not (B14).
    fn supports(&self, item: &StoreItem, column: &Column) -> Result<(), String>;
    /// What `place` holds under `name`.
    fn entry(&self, place: &Place, name: &str) -> Result<Option<Entry>, String>;
    /// Every entry `place` holds, by name, each read on its own: one that
    /// cannot be read answers why and leaves the others listed.
    fn entries(&self, place: &Place) -> Result<Vec<Listed>, String>;
    /// Where the operator finds `place`'s entry `name`: a skill's folder, or
    /// the MCP file that holds the server.
    fn entry_path(&self, place: &Place, name: &str) -> PathBuf {
        place.path.join(name)
    }
    /// The keys `item` keeps in its folder `files` beside its body; none for
    /// a kind without keys.
    fn keys(&self, _item: &StoreItem, _files: &Path) -> Result<Option<mcp::KeyValues>, String> {
        Ok(None)
    }
    /// Writes `item` at `place`, replacing what is there under its name
    /// (only ever Hide's own copy); answers the digest written.
    fn write(&self, ctx: &Ctx<'_>, place: &Place, item: &StoreItem) -> Result<String, String>;
    /// Takes `name` out of `place`.
    fn remove(&self, place: &Place, name: &str) -> Result<(), String>;
    /// Copies what `place` holds under `name` into the store folder `into`,
    /// for an item adopted from it (B20): answers its source and body.
    fn adopt(
        &self,
        ctx: &Ctx<'_>,
        place: &Place,
        name: &str,
        into: &Path,
    ) -> Result<(Source, ItemBody), StoreRefusal>;
    /// Moves the operator's own entry `name` out of `place` whole into the
    /// undo list's folder `into`, nothing left behind or left out, so taking
    /// the place over loses nothing (B21, B22); answers its body.
    fn set_aside(&self, place: &Place, name: &str, into: &Path) -> Result<ItemBody, StoreRefusal>;
    /// Puts back at `place` what [`Applier::set_aside`] moved to `from`,
    /// for a machine leaving Hide (B30): false, with nothing moved, when the
    /// place holds something of the name now.
    fn put_back(&self, place: &Place, name: &str, from: &Path) -> Result<bool, String>;
    /// Writes a copy of the store's own files `files` of an item at
    /// `place` under `name`, where nothing stands, for a machine leaving
    /// Hide while the store holds the operator's only original (B30).
    fn leave_copy(&self, place: &Place, item: &StoreItem, files: &Path) -> Result<(), String>;
    /// The item is deleted for good: anything of it the store keeps beside
    /// its files goes (an MCP's keys, D-11).
    fn purge(&self, ctx: &Ctx<'_>, item: &StoreItem) -> Result<(), String>;
    /// What a copy to another machine carries of `item` (B28): a skill's
    /// files, an MCP server's spec with its key values.
    fn export(&self, ctx: &Ctx<'_>, item: &StoreItem) -> Result<Carried, StoreRefusal>;
    /// Writes what another machine carried into the store folder `into`,
    /// which does not exist yet, and answers the item's body (B28).
    fn import(
        &self,
        ctx: &Ctx<'_>,
        name: &str,
        content: &Carried,
        into: &Path,
    ) -> Result<ItemBody, StoreRefusal>;
}

fn appliers() -> [&'static dyn Applier; 2] {
    [&skill::SkillApplier, &mcp::McpApplier]
}

fn applier(kind: ItemKind) -> &'static dyn Applier {
    match kind {
        ItemKind::Skill => &skill::SkillApplier,
        ItemKind::Mcp => &mcp::McpApplier,
    }
}

/// What every step of a pass reads.
pub(crate) struct Ctx<'a> {
    pub(crate) target: &'a KitTarget,
    pub(crate) dir: PathBuf,
    pub(crate) accounts: &'a crate::targets::Accounts,
    pub(crate) detection: &'a Detection,
    pub(crate) record: &'a Record,
    pub(crate) scope: &'a Scope,
    /// The pass may change places, so every entry is read again
    /// ([`Place::fresh`]).
    pub(crate) fresh: bool,
    /// The store's secret, for the fingerprints of the keys Hide wrote
    /// ([`Written::keys`]); none before anything with keys was written.
    pub(crate) secret: Option<crate::mcp_keys::KeysSecret>,
}

impl Ctx<'_> {
    pub(crate) fn home(&self) -> &Path {
        &self.target.home
    }

    /// Whether the agent is switched on in Settings › Agents.
    fn enabled(&self, agent: &str) -> bool {
        crate::agents::adapter(agent)
            .is_some_and(|adapter| enabled(self.record, self.scope, adapter))
    }

    fn installed(&self, agent: &str) -> bool {
        crate::agents::adapter(agent).is_some_and(|adapter| self.detection.installed(adapter))
    }

    /// The agents an item of `kind` that is on everywhere reaches now (B6).
    fn reaching(&self, kind: ItemKind) -> Vec<String> {
        let mut agents: Vec<String> = Vec::new();
        for column in applier(kind).columns() {
            for agent in column.agents.iter().chain(&column.also) {
                if self.enabled(agent) && self.installed(agent) && !agents.contains(agent) {
                    agents.push(agent.clone());
                }
            }
        }
        agents
    }

    pub(crate) fn item_files(&self, id: &str) -> PathBuf {
        self.dir.join("items").join(folder_name(id))
    }

    fn trash_files(&self, entry: &str) -> PathBuf {
        self.dir.join("trash").join(entry)
    }

    pub(crate) fn staging(&self) -> PathBuf {
        self.dir.join("staging")
    }
}

/// A file-system name for an id: `skill:archify` is `skill-archify`.
pub(crate) fn folder_name(id: &str) -> String {
    id.replace(':', "-")
}

/// Whether `name` is one of Hide's own skills, which the store never lists
/// or installs (B35).
pub(crate) fn hides_own(name: &str) -> bool {
    SKILLS.iter().any(|skill| skill.name == name)
        || RETIRED_SKILLS.iter().any(|skill| skill.name == name)
}

/// Whether `name` may name an item: a folder or server name that is the
/// same on every system.
pub(crate) fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric())
        && name.len() <= 64
        && chars.all(|char| char.is_ascii_alphanumeric() || matches!(char, '-' | '_' | '.'))
}

pub(crate) fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

// --- The record -----------------------------------------------------------------

fn record_path(dir: &Path) -> PathBuf {
    dir.join("store.json")
}

/// The store folder, made private when `create`, or why Hide keeps nothing
/// there (`crate::private_dirs`).
fn store_dir(home: &Path, create: bool) -> Result<PathBuf, String> {
    crate::private_dirs(home, &crate::layout::STORE_PARTS, create).map_err(|refusal| refusal.detail)
}

/// The store's record, an empty one when there is none, or why it could not
/// be read. An unreadable record is never read as empty: that would read
/// every copy Hide wrote as the operator's.
pub(crate) fn load(dir: &Path) -> Result<StoreRecord, String> {
    let path = record_path(dir);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let record: StoreRecord = serde_json::from_slice(&bytes)
                .map_err(|error| format!("{} could not be read: {error}", path.display()))?;
            if record.format > FORMAT {
                return Err(format!(
                    "{} was written by a newer Hide, so this one changes nothing",
                    path.display()
                ));
            }
            Ok(record)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(StoreRecord {
            format: FORMAT,
            ..StoreRecord::default()
        }),
        Err(error) => Err(format!("{} could not be read: {error}", path.display())),
    }
}

fn save(dir: &Path, record: &StoreRecord) -> Result<(), String> {
    let record = StoreRecord {
        format: FORMAT,
        ..record.clone()
    };
    let mut bytes = serde_json::to_vec_pretty(&record).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    crate::write_atomically(
        &record_path(dir),
        &bytes,
        hide_platform::fs::Access::Private,
    )
}

// --- Entry points ---------------------------------------------------------------

/// What a pass may change.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Apply,
    Read,
}

/// A pass over the store with the changes in `scope`, in order, run inside
/// the kit's account lock by `crate::apply` and `crate::apply_store`.
/// `fetched` is what the sources answered before the lock was taken.
pub(crate) fn pass(
    target: &KitTarget,
    scope: &Scope,
    record: &Record,
    detection: &Detection,
    profiles: Option<&KitProfiles>,
    fetched: source::Fetched,
) -> StoreReport {
    let accounts = crate::targets::accounts(target, profiles);
    run(
        target,
        scope,
        record,
        detection,
        &accounts,
        fetched,
        Mode::Apply,
    )
}

/// The store judged without changing anything (`crate::status`).
pub(crate) fn status(
    target: &KitTarget,
    record: &Record,
    detection: &Detection,
    profiles: Option<&KitProfiles>,
) -> StoreReport {
    let accounts = crate::targets::accounts(target, profiles);
    run(
        target,
        &Scope::default(),
        record,
        detection,
        &accounts,
        source::Fetched::default(),
        Mode::Read,
    )
}

/// What the sources `changes` name answer, asked before the account lock is
/// taken so a slow repository holds no other pass up.
pub(crate) fn fetch(target: &KitTarget, changes: &[StoreChange]) -> source::Fetched {
    if !changes.iter().any(source::reaches_out) {
        return source::Fetched::default();
    }
    let dir = match store_dir(&target.home, true) {
        Ok(dir) => dir,
        Err(reason) => return source::Fetched::refused(reason),
    };
    let record = load(&dir).unwrap_or_default();
    source::fetch(target, &dir, &record, changes)
}

fn run(
    target: &KitTarget,
    scope: &Scope,
    record: &Record,
    detection: &Detection,
    accounts: &crate::targets::Accounts,
    fetched: source::Fetched,
    mode: Mode,
) -> StoreReport {
    // The folder is made only for a change: a machine whose operator never
    // added anything has none.
    let dir = match store_dir(&target.home, mode == Mode::Apply && !scope.store.is_empty()) {
        Ok(dir) => dir,
        Err(reason) => return refused(reason),
    };
    let mut store = match load(&dir) {
        Ok(store) => store,
        Err(reason) => return refused(reason),
    };
    let before = store.clone();
    let secret = match store_secret(&dir, &store, mode == Mode::Apply && dir.is_dir()) {
        Ok(secret) => secret,
        Err((reason, cause)) => return StoreReport::refused_as(reason, cause),
    };
    let ctx = Ctx {
        target,
        dir,
        accounts,
        detection,
        record,
        scope,
        fresh: mode == Mode::Apply,
        secret,
    };
    let mut outcomes = Vec::new();
    if mode == Mode::Apply {
        follow(&ctx, &mut store);
        fetched.keep_into(&ctx, &mut store);
        for change in &scope.store {
            let (made, refused) = match change_one(&ctx, &mut store, change, &fetched) {
                Ok(made) => (made, None),
                Err(refusal) => {
                    log("store.change_refused", None, &refusal.reason);
                    (None, Some(refusal))
                }
            };
            outcomes.push(ChangeOutcome {
                change: change.redacted(),
                refused,
                made,
            });
        }
        trim_staging(&ctx, &mut store);
    }
    let places: BTreeMap<ItemKind, Vec<Place>> = appliers()
        .into_iter()
        .map(|applier| (applier.kind(), applier.places(&ctx)))
        .collect();
    let mut failures = PlaceFailures::default();
    if mode == Mode::Apply {
        release_orphans(&ctx, &mut store, &places);
        for index in 0..store.items.len() {
            let item = store.items[index].clone();
            reconcile(&ctx, &mut store, &item, &places[&item.kind], &mut failures);
        }
        trim_stale(&mut store);
    }
    let report = report(&ctx, &store, &places, &failures, outcomes);
    if mode == Mode::Apply
        && store != before
        && let Err(reason) = save(&ctx.dir, &store)
    {
        log("store.save_failed", None, &reason);
        return StoreReport {
            refused: Some(format!("the store's changes were made, but {reason}")),
            refusal: Some(StorePassRefusal::Unreadable),
            ..report
        };
    }
    report
}

fn refused(reason: String) -> StoreReport {
    StoreReport::refused(reason)
}

impl StoreReport {
    /// The report of a pass that could not read or lock the store.
    pub(crate) fn refused(reason: String) -> Self {
        Self::refused_as(reason, StorePassRefusal::Unreadable)
    }

    pub(crate) fn refused_as(reason: String, cause: StorePassRefusal) -> Self {
        log("store.refused", None, &reason);
        Self {
            refused: Some(reason),
            refusal: Some(cause),
            ..Self::default()
        }
    }
}

/// The items whose only original the store holds (B30), with where each
/// was taken over and that place's key: an [`StoreItem::adopted`] one whose
/// own folder or entry there is not in the undo list, forgotten or past its
/// limit.
fn sole_originals<'a>(
    dir: &Path,
    store: &'a StoreRecord,
) -> Vec<(&'a StoreItem, PathBuf, &'a str)> {
    let set_aside: BTreeSet<PathBuf> = store
        .trash
        .iter()
        .filter(|entry| dir.join("trash").join(&entry.id).exists())
        .filter_map(|entry| match &entry.cause {
            TrashCause::Replaced { path, .. } => Some(path.join(&entry.item.name)),
            TrashCause::Deleted => None,
        })
        .collect();
    store
        .items
        .iter()
        .filter_map(|item| match (&item.source, &item.adopted) {
            (Source::Copy { from }, Some(key)) => Some((item, PathBuf::from(from), key.as_str())),
            _ => None,
        })
        .filter(|(_, from, _)| !set_aside.contains(from))
        .collect()
}

/// The store's secret, made when `make` and the record holds no key
/// fingerprint yet. One the record's fingerprints need that is gone or
/// cannot be read refuses the pass: a new secret, or none, would read every
/// MCP entry Hide wrote with keys as the operator's, and Hide would never
/// take them out.
fn store_secret(
    dir: &Path,
    store: &StoreRecord,
    make: bool,
) -> Result<Option<crate::mcp_keys::KeysSecret>, (String, StorePassRefusal)> {
    let keyed = store
        .written
        .values()
        .any(|place| place.entries.values().any(|written| written.keys.is_some()));
    match crate::mcp_keys::secret(dir, make && !keyed) {
        Ok(None) if keyed => Err((
            format!(
                "{} is gone, so Hide cannot tell the MCP servers it wrote from ones changed by hand; the store is left as it is",
                dir.join(crate::mcp_keys::SECRET_FILE).display()
            ),
            StorePassRefusal::KeysSecretGone,
        )),
        Ok(secret) => Ok(secret),
        // A secret that cannot be read, while servers carry keys, is as
        // gone: the same manual start-over brings the store back
        // (docs/agent-hooks.md, When the MCP keys' secret is lost).
        Err(error) if keyed => Err((error.to_string(), StorePassRefusal::KeysSecretGone)),
        Err(error) => Err((error.to_string(), StorePassRefusal::Unreadable)),
    }
}

/// Takes the copies Hide put into every place out, leaves or writes back a
/// copy of each item whose only original the store holds where the
/// operator had it, puts each folder a take-over set aside back, then
/// removes the store with its keys, for a machine leaving Hide (B30): one
/// outcome per place, and the store kept, with why, while anything of the
/// operator's could not be left where it was.
pub(crate) fn remove(target: &KitTarget) -> Vec<(String, crate::RemoveOutcome)> {
    use crate::RemoveOutcome;
    let dir = match store_dir(&target.home, false) {
        Ok(dir) if dir.is_dir() => dir,
        Ok(_) => return Vec::new(),
        Err(reason) => return vec![("store".to_owned(), RemoveOutcome::Failed { reason })],
    };
    let store = match load(&dir) {
        Ok(store) => store,
        Err(reason) => return vec![("store".to_owned(), RemoveOutcome::Failed { reason })],
    };
    // Without its secret no copy whose keys Hide recorded reads as Hide's,
    // so the store stays rather than leave them in the agents' files.
    let secret = match store_secret(&dir, &store, false) {
        Ok(secret) => secret,
        Err((reason, _)) => {
            log("store.secret_unreadable", None, &reason);
            return vec![("store".to_owned(), RemoveOutcome::Failed { reason })];
        }
    };
    let mut outcomes = Vec::new();
    let mut failed = false;
    let originals = sole_originals(&dir, &store);
    // Each such copy Hide holds where the operator had it stays there.
    let stays: BTreeSet<&Path> = originals
        .iter()
        .map(|(_, from, _)| from.as_path())
        .collect();
    for (key, place) in &store.written {
        let Some(kind) = place.kind else { continue };
        let at = recorded_place(key, place);
        let mut removed = false;
        let mut failure = None;
        for (name, written) in &place.entries {
            if stays.contains(place.path.join(name).as_path()) {
                continue;
            }
            match take_out_if_ours(applier(kind), &at, name, written, secret.as_ref()) {
                Ok(true) => removed = true,
                Ok(false) => {}
                Err(reason) => failure = Some(reason),
            }
        }
        let outcome = match failure {
            Some(reason) => {
                failed = true;
                RemoveOutcome::Failed { reason }
            }
            None if removed => RemoveOutcome::Removed,
            None => RemoveOutcome::Absent,
        };
        outcomes.push((format!("store:{key}"), outcome));
    }
    // An agent's own copy a take-over set aside goes back where it was, now
    // that Hide's copy is out (B30). One that cannot keeps the store, so it
    // is not lost.
    let mut kept_back = None;
    for entry in &store.trash {
        let TrashCause::Replaced { place, path } = &entry.cause else {
            continue;
        };
        let from = dir.join("trash").join(&entry.id);
        if !from.exists() {
            continue;
        }
        let at = Place {
            key: place.clone(),
            path: path.clone(),
            columns: Vec::new(),
            profiles: Vec::new(),
            blocked: None,
            fresh: true,
        };
        let reason = match applier(entry.item.kind).put_back(&at, &entry.item.name, &from) {
            Ok(true) => continue,
            Ok(false) => format!(
                "{} holds another {} now, so the one Hide set aside stays in the store",
                path.join(&entry.item.name).display(),
                entry.item.name
            ),
            Err(reason) => reason,
        };
        log("store.put_back_failed", Some(&entry.item.id), &reason);
        outcomes.push((
            format!("store:{place}/{}", entry.item.name),
            RemoveOutcome::Failed {
                reason: reason.clone(),
            },
        ));
        kept_back = Some(reason);
    }
    // Where a switch took the copy out, the store's copy is written back.
    for (item, from, key) in &originals {
        let (Some(folder), Some(_)) = (from.parent(), from.file_name()) else {
            continue;
        };
        let at = Place {
            key: (*key).to_owned(),
            path: folder.to_path_buf(),
            columns: Vec::new(),
            profiles: Vec::new(),
            blocked: None,
            fresh: true,
        };
        let applier = applier(item.kind);
        let reason = match applier.entry(&at, &item.name) {
            Ok(Some(entry)) if entry.digest == item.digest() => continue,
            Ok(Some(_)) => format!(
                "{} holds another {} now, so the store keeps the one it took over",
                from.display(),
                item.name
            ),
            Ok(None) => {
                let files = dir.join("items").join(folder_name(&item.id));
                match applier.leave_copy(&at, item, &files) {
                    Ok(()) => continue,
                    Err(reason) => reason,
                }
            }
            Err(reason) => reason,
        };
        log("store.leave_copy_failed", Some(&item.id), &reason);
        outcomes.push((
            format!("store:{}", from.display()),
            RemoveOutcome::Failed {
                reason: reason.clone(),
            },
        ));
        kept_back = Some(reason);
    }
    // The store goes with what it applied, keys and all; while a copy Hide
    // wrote is still out there, the store stays so the next removal can
    // tell it is Hide's.
    let store_outcome = if failed {
        RemoveOutcome::Kept {
            reason: "a copy Hide wrote could not be taken out".to_owned(),
        }
    } else if let Some(reason) = kept_back {
        RemoveOutcome::Kept { reason }
    } else {
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => RemoveOutcome::Removed,
            Err(error) => RemoveOutcome::Failed {
                reason: format!("{} could not be removed: {error}", dir.display()),
            },
        }
    };
    outcomes.push(("store".to_owned(), store_outcome));
    outcomes
}

// --- Folder sources --------------------------------------------------------------

/// Whether a folder an item follows changed since the store last took it in
/// (D-33): a stat of each such folder and nothing written. A store that is
/// not there follows nothing, and one that cannot be read is left to the
/// next pass to report.
pub(crate) fn follow_due(target: &KitTarget) -> bool {
    let Ok(dir) = store_dir(&target.home, false) else {
        return false;
    };
    let Ok(store) = load(&dir) else {
        return false;
    };
    store.items.iter().any(|item| {
        let (Source::Folder { path }, ItemKind::Skill) = (&item.source, item.kind) else {
            return false;
        };
        match skill::fingerprint(Path::new(path)) {
            Ok(Some(fingerprint)) => {
                item.folder_missing || item.fingerprint.as_deref() != Some(fingerprint.as_str())
            }
            Ok(None) => !item.folder_missing,
            Err(_) => false,
        }
    })
}

/// Brings each followed folder's changes into the store (D-20, B24): a
/// folder whose stat fingerprint moved is read again, and one that is gone
/// keeps the last copy and says so.
fn follow(ctx: &Ctx<'_>, store: &mut StoreRecord) {
    for item in &mut store.items {
        let (Source::Folder { path }, ItemKind::Skill) = (&item.source, item.kind) else {
            continue;
        };
        let folder = PathBuf::from(path);
        let fingerprint = match skill::fingerprint(&folder) {
            Ok(Some(fingerprint)) => fingerprint,
            Ok(None) => {
                item.folder_missing = true;
                continue;
            }
            Err(reason) => {
                log("store.follow_unreadable", Some(&item.id), &reason);
                continue;
            }
        };
        if item.fingerprint.as_deref() == Some(fingerprint.as_str()) && !item.folder_missing {
            continue;
        }
        match skill::take_in(&folder, &ctx.item_files(&item.id)) {
            Ok(body) => {
                item.folder_missing = false;
                item.fingerprint = Some(fingerprint);
                item.body = ItemBody::Skill(body);
            }
            Err(refusal) => log("store.follow_failed", Some(&item.id), &refusal.reason),
        }
    }
}

// --- Changes --------------------------------------------------------------------

fn not_found(what: impl Into<String>) -> StoreRefusal {
    StoreRefusal::new(StoreRefusalCause::NotFound, what)
}

fn failed(reason: String) -> StoreRefusal {
    StoreRefusal::new(StoreRefusalCause::Failed, reason)
}

/// One change of a pass, and the id of what it made for its asker to show.
fn change_one(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    change: &StoreChange,
    fetched: &source::Fetched,
) -> Result<Option<String>, StoreRefusal> {
    let nothing = |()| None;
    match change {
        StoreChange::Inspect { source } => fetched
            .preview_of(source)
            .map(|preview| Some(preview.id.clone())),
        StoreChange::Install { preview, names } => install(ctx, store, preview, names).map(nothing),
        StoreChange::Set { item, column, on } => {
            let found = store
                .item_mut(item)
                .ok_or_else(|| not_found(format!("no item {item}")))?;
            let columns = applier(found.kind).columns();
            if let Some(column) = column
                && !columns.iter().any(|known| known.id == *column)
            {
                return Err(not_found(format!("no column {column}")));
            }
            set(found, &columns, column.as_deref(), *on);
            Ok(None)
        }
        StoreChange::Delete { item } => delete(ctx, store, item).map(nothing),
        StoreChange::Restore { entry } => restore(ctx, store, entry).map(nothing),
        StoreChange::Forget { entry } => {
            let index = store
                .trash
                .iter()
                .position(|kept| kept.id == *entry)
                .ok_or_else(|| not_found(format!("no undo entry {entry}")))?;
            let gone = store.trash.remove(index);
            purge(ctx, &gone);
            Ok(None)
        }
        StoreChange::Adopt { kind, name, from } => {
            adopt(ctx, store, *kind, name, from.as_deref()).map(nothing)
        }
        StoreChange::CheckUpdates => fetched.checked().map(nothing),
        StoreChange::PreviewUpdate { item } => {
            store
                .item(item)
                .ok_or_else(|| not_found(format!("no item {item}")))?;
            fetched.diff_of(item).map(|diff| Some(diff.id.clone()))
        }
        StoreChange::Update { item, commit } => update(ctx, store, item, commit).map(nothing),
        StoreChange::ReadMcp { text } => mcp::read_paste(ctx, store, text).map(Some),
        StoreChange::AddMcp { server, keys, from } => {
            mcp::add(ctx, store, server, keys, from.as_ref()).map(nothing)
        }
        StoreChange::CancelMcp { preview } => mcp::cancel(ctx, store, preview).map(nothing),
        StoreChange::SetMcp { item, server, keys } => {
            mcp::set(ctx, store, item, server, keys).map(nothing)
        }
        StoreChange::Receive { item } => receive(ctx, store, item).map(nothing),
    }
}

/// One switch of a row (B15, B16, D-40): a column's cell, or the item's
/// own, which from any partial state turns every column on.
fn set(item: &mut StoreItem, columns: &[Column], column: Option<&str>, on: bool) {
    match column {
        None => {
            item.on = on;
            item.off_for.clear();
        }
        Some(column) if on => {
            if !item.on {
                // From all off, one column on: the item is on for that
                // column alone.
                item.on = true;
                item.off_for = columns
                    .iter()
                    .map(|known| known.id.clone())
                    .filter(|id| id != column)
                    .collect();
            } else {
                item.off_for.remove(column);
            }
        }
        Some(column) => {
            item.off_for.insert(column.to_owned());
            if columns.iter().all(|known| item.off_for.contains(&known.id)) {
                item.on = false;
                item.off_for.clear();
            }
        }
    }
}

fn room_for(store: &StoreRecord, additional: usize) -> Result<(), StoreRefusal> {
    if additional > 0 && store.items.len().saturating_add(additional) > ITEM_LIMIT {
        return Err(StoreRefusal::new(
            StoreRefusalCause::TooMany,
            format!("the store holds {ITEM_LIMIT} items already; delete one before adding another"),
        ));
    }
    Ok(())
}

fn name_free(store: &StoreRecord, kind: ItemKind, name: &str) -> Result<(), StoreRefusal> {
    if hides_own(name) || store.item(&StoreItem::id_of(kind, name)).is_some() {
        return Err(StoreRefusal::new(
            StoreRefusalCause::NameTaken,
            format!("an item named {name} is already in the store"),
        ));
    }
    Ok(())
}

fn install(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    preview: &str,
    names: &[String],
) -> Result<(), StoreRefusal> {
    let found = store
        .previews
        .iter()
        .find(|kept| kept.id == preview)
        .cloned()
        .ok_or_else(|| not_found("the source's answer is gone; add it again"))?;
    // Every name is checked before any is installed, so a refusal leaves
    // the store as it was.
    let mut chosen = Vec::new();
    for name in names {
        let skill = found
            .skills
            .iter()
            .find(|skill| skill.name == *name)
            .ok_or_else(|| not_found(format!("{name} is not in that source")))?;
        name_free(store, ItemKind::Skill, name)?;
        chosen.push(skill.clone());
    }
    room_for(store, chosen.len())?;
    let followed = store
        .items
        .iter()
        .filter(|item| matches!(item.source, Source::Folder { .. }))
        .count();
    if matches!(found.source, SourceSpec::Folder { .. }) && followed + chosen.len() > FOLLOWED_LIMIT
    {
        return Err(StoreRefusal::new(
            StoreRefusalCause::TooMany,
            format!("the store follows {FOLLOWED_LIMIT} folders already"),
        ));
    }
    for skill in chosen {
        let from = source::files_of(ctx, &found, &skill);
        let id = StoreItem::id_of(ItemKind::Skill, &skill.name);
        let body = skill::take_in(&from, &ctx.item_files(&id))?;
        let source = source::source_of(&found, &skill, &from);
        store.items.push(StoreItem {
            id,
            kind: ItemKind::Skill,
            name: skill.name.clone(),
            // What was installed is what the confirm screen showed (B6); a
            // folder that changed meanwhile is followed from here on.
            fingerprint: match &source {
                Source::Folder { .. } => skill::fingerprint(&from).ok().flatten(),
                _ => None,
            },
            source,
            on: true,
            off_for: BTreeSet::new(),
            added_at_unix_ms: now_unix_ms(),
            latest: None,
            folder_missing: false,
            adopted: None,
            body: ItemBody::Skill(body),
        });
    }
    Ok(())
}

fn update(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    id: &str,
    commit: &str,
) -> Result<(), StoreRefusal> {
    let diff = store
        .diffs
        .iter()
        .find(|diff| diff.item == id && diff.commit == commit)
        .cloned()
        .ok_or_else(|| not_found("that update's changes are gone; check again"))?;
    let item = store
        .item_mut(id)
        .ok_or_else(|| not_found(format!("no item {id}")))?;
    let body = skill::take_in(&source::staged_update(ctx, &diff), &ctx.item_files(id))?;
    if let Source::Git {
        commit: current,
        tag,
        ..
    } = &mut item.source
    {
        current.clone_from(&diff.commit);
        *tag = match &diff.to {
            Version::Tag(tag) => Some(tag.clone()),
            _ => None,
        };
    }
    item.body = ItemBody::Skill(body);
    store.diffs.retain(|kept| kept.item != id);
    Ok(())
}

/// Takes out the copies before retiring the item (B18). A failed cleanup
/// keeps the row and its files, switched off so later passes cannot put
/// removed copies back; another delete can finish the remaining cleanup.
fn delete(ctx: &Ctx<'_>, store: &mut StoreRecord, id: &str) -> Result<(), StoreRefusal> {
    let index = store
        .items
        .iter()
        .position(|item| item.id == id)
        .ok_or_else(|| not_found(format!("no item {id}")))?;
    let kind = store.items[index].kind;
    let mut refusal = None;
    let kept = store.written.clone();
    for (key, place) in &kept {
        for (name, written) in &place.entries {
            if written.item != id {
                continue;
            }
            match take_out_if_ours(
                applier(kind),
                &recorded_place(key, place),
                name,
                written,
                ctx.secret.as_ref(),
            ) {
                Ok(_) => forget_written(store, key, name),
                Err(reason) => {
                    log("store.release_failed", Some(id), &reason);
                    refusal.get_or_insert_with(|| failed(reason));
                }
            }
        }
    }
    if let Some(refusal) = refusal {
        store.items[index].on = false;
        return Err(refusal);
    }
    let entry_id = format!("{}-{}", folder_name(id), now_unix_ms());
    let files = ctx.item_files(id);
    if files.exists() {
        move_folder(&files, &ctx.trash_files(&entry_id)).map_err(failed)?;
    }
    let item = store.items.remove(index);
    store.changed.remove(id);
    store.stale.remove(id);
    push_trash(
        ctx,
        store,
        TrashEntry {
            id: entry_id,
            item,
            at_unix_ms: now_unix_ms(),
            cause: TrashCause::Deleted,
        },
    );
    Ok(())
}

/// Takes in an item another machine's store carried (B28). Its content is
/// written beside the store first; it replaces an item of the same id only
/// when they differ, the old one going to the undo list, and the same item
/// again changes nothing (engineering rule 11). A folder source arrives as a
/// copy, since this machine cannot follow another's folder.
fn receive(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    carried: &CarriedItem,
) -> Result<(), StoreRefusal> {
    if hides_own(&carried.name) || !valid_name(&carried.name) {
        return Err(StoreRefusal::new(
            StoreRefusalCause::NameTaken,
            format!("{} cannot name an item", carried.name),
        ));
    }
    let id = StoreItem::id_of(carried.kind, &carried.name);
    room_for(store, usize::from(store.item(&id).is_none()))?;
    let staged = ctx.staging().join(source::fresh_id("received-"));
    let body = applier(carried.kind)
        .import(ctx, &carried.name, &carried.content, &staged)
        .inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&staged);
        })?;
    let item = StoreItem {
        id: id.clone(),
        kind: carried.kind,
        name: carried.name.clone(),
        source: match &carried.source {
            Source::Folder { path } => Source::Copy { from: path.clone() },
            other => other.clone(),
        },
        on: carried.on,
        off_for: carried.off_for.clone(),
        added_at_unix_ms: now_unix_ms(),
        latest: carried.latest.clone(),
        folder_missing: false,
        fingerprint: None,
        adopted: None,
        body,
    };
    if let Some(existing) = store.item(&id) {
        if existing.digest() == item.digest()
            && existing.source == item.source
            && existing.on == item.on
            && existing.off_for == item.off_for
        {
            // An MCP's digest holds no key value, so the keys it carried are
            // compared on their own.
            let keys = match carried.kind {
                ItemKind::Mcp => mcp::take_carried_keys(ctx, store, &id, &staged).map(drop),
                ItemKind::Skill => Ok(()),
            };
            let _ = std::fs::remove_dir_all(&staged);
            return keys;
        }
        delete(ctx, store, &id).inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&staged);
        })?;
    }
    move_folder(&staged, &ctx.item_files(&id)).map_err(failed)?;
    store.items.push(item);
    Ok(())
}

/// This machine's store as a copy carries it to another (B28): every item
/// with its kind's content, refused whole past [`COPY_BYTE_LIMIT`] or when an
/// item cannot be carried, before anything is sent.
pub(crate) fn export(
    target: &KitTarget,
    record: &Record,
    detection: &Detection,
    profiles: Option<&KitProfiles>,
) -> StoreExport {
    let refused = |refusal: StoreRefusal| {
        log("store.export_refused", None, &refusal.reason);
        StoreExport {
            items: Vec::new(),
            refused: Some(refusal),
        }
    };
    let dir = match store_dir(&target.home, false) {
        Ok(dir) => dir,
        Err(reason) => return refused(failed(reason)),
    };
    let store = match load(&dir) {
        Ok(store) => store,
        Err(reason) => return refused(failed(reason)),
    };
    let accounts = crate::targets::accounts(target, profiles);
    let scope = Scope::default();
    let ctx = Ctx {
        target,
        dir,
        accounts: &accounts,
        detection,
        record,
        scope: &scope,
        fresh: true,
        secret: None,
    };
    let mut items = Vec::new();
    let mut size = 0;
    for item in &store.items {
        let content = match applier(item.kind).export(&ctx, item) {
            Ok(content) => content,
            Err(refusal) => return refused(refusal),
        };
        size += content.size();
        if size > COPY_BYTE_LIMIT {
            return refused(StoreRefusal::new(
                StoreRefusalCause::TooLarge,
                format!(
                    "the store holds more than {} MiB to copy",
                    COPY_BYTE_LIMIT / 1024 / 1024
                ),
            ));
        }
        items.push(CarriedItem {
            kind: item.kind,
            name: item.name.clone(),
            source: item.source.clone(),
            on: item.on,
            off_for: item.off_for.clone(),
            latest: item.latest.clone(),
            content,
        });
    }
    StoreExport {
        items,
        refused: None,
    }
}

fn push_trash(ctx: &Ctx<'_>, store: &mut StoreRecord, entry: TrashEntry) {
    store.trash.insert(0, entry);
    while store.trash.len() > UNDO_LIMIT {
        let oldest = store.trash.pop().expect("longer than the limit");
        purge(ctx, &oldest);
    }
}

/// An undo entry is gone for good (D-25, D-39): its files, and what its
/// kind keeps beside them.
fn purge(ctx: &Ctx<'_>, entry: &TrashEntry) {
    let files = ctx.trash_files(&entry.id);
    if let Err(error) = std::fs::remove_dir_all(&files)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        log(
            "store.purge_failed",
            Some(&entry.item.id),
            &error.to_string(),
        );
    }
    if let Err(reason) = applier(entry.item.kind).purge(ctx, &entry.item) {
        log("store.purge_failed", Some(&entry.item.id), &reason);
    }
}

fn restore(ctx: &Ctx<'_>, store: &mut StoreRecord, entry: &str) -> Result<(), StoreRefusal> {
    let index = store
        .trash
        .iter()
        .position(|kept| kept.id == entry)
        .ok_or_else(|| not_found(format!("no undo entry {entry}")))?;
    let kept = store.trash[index].clone();
    name_free(store, kept.item.kind, &kept.item.name)?;
    room_for(store, 1)?;
    let files = ctx.trash_files(&kept.id);
    if files.exists() {
        move_folder(&files, &ctx.item_files(&kept.item.id)).map_err(failed)?;
    }
    store.trash.remove(index);
    store.items.push(kept.item);
    Ok(())
}

/// Moves `from` to `to`, replacing what is there.
pub(crate) fn move_folder(from: &Path, to: &Path) -> Result<(), String> {
    if let Some(parent) = to.parent() {
        hide_platform::fs::private::create_dir_all(parent)
            .map_err(|error| format!("{} could not be created: {error}", parent.display()))?;
    }
    if to.exists() {
        std::fs::remove_dir_all(to)
            .map_err(|error| format!("{} could not be cleared: {error}", to.display()))?;
    }
    std::fs::rename(from, to)
        .map_err(|error| format!("{} could not be moved: {error}", from.display()))
}

/// The copies of `name` agents hold that Hide did not write, by place.
fn copies_of<'a>(
    store: &StoreRecord,
    applier: &dyn Applier,
    places: &'a [Place],
    name: &str,
    secret: Option<&crate::mcp_keys::KeysSecret>,
) -> Result<Vec<(&'a Place, Entry)>, StoreRefusal> {
    let mut copies = Vec::new();
    for place in places {
        let written = store
            .written
            .get(&place.key)
            .and_then(|kept| kept.entries.get(name));
        match applier.entry(place, name) {
            Ok(Some(entry)) if !ours(written, &entry, secret) => copies.push((place, entry)),
            Ok(_) => {}
            Err(reason) => return Err(failed(reason)),
        }
    }
    Ok(copies)
}

/// Takes over what agents hold under `name` (B20, B21, B22's [가져오기]).
/// With `from`, that place's copy becomes the item, replacing the store's
/// own item of the name, which goes to the undo list. Without it, an item of
/// the name in the store stays the item; with none, the copies must agree.
/// Every copy with the item's content becomes Hide's where it is; a link,
/// or a copy with other content, which goes to the undo list first, is
/// replaced by Hide's copy in the reconcile that follows.
fn adopt(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    kind: ItemKind,
    name: &str,
    from: Option<&str>,
) -> Result<(), StoreRefusal> {
    if hides_own(name) || !valid_name(name) {
        return Err(StoreRefusal::new(
            StoreRefusalCause::NameTaken,
            format!("{name} is not a name Hide manages"),
        ));
    }
    let applier = applier(kind);
    let places = applier.places(ctx);
    let id = StoreItem::id_of(kind, name);
    let copies = copies_of(store, applier, &places, name, ctx.secret.as_ref())?;
    let existing = store.item(&id).is_some();
    room_for(store, usize::from(!existing))?;
    let chosen = match from {
        Some(key) => Some(
            copies
                .iter()
                .find(|(place, _)| place.key == key)
                .ok_or_else(|| not_found(format!("no copy of {name} at {key}")))?,
        ),
        None if existing => None,
        None => {
            let first = copies
                .first()
                .ok_or_else(|| not_found(format!("no agent holds {name}")))?;
            if copies
                .iter()
                .any(|(_, entry)| entry.digest != first.1.digest || entry.keys != first.1.keys)
            {
                return Err(StoreRefusal::new(
                    StoreRefusalCause::Differs,
                    format!("the agents hold different copies of {name}; pick one"),
                ));
            }
            Some(first)
        }
    };
    if let Some((place, _)) = chosen {
        if existing {
            delete(ctx, store, &id)?;
        }
        let (source, body) = applier.adopt(ctx, place, name, &ctx.item_files(&id))?;
        store.items.push(StoreItem {
            id: id.clone(),
            kind,
            name: name.to_owned(),
            fingerprint: match &source {
                Source::Folder { path } => skill::fingerprint(Path::new(path)).ok().flatten(),
                _ => None,
            },
            adopted: matches!(source, Source::Copy { .. }).then(|| place.key.clone()),
            source,
            on: true,
            off_for: BTreeSet::new(),
            added_at_unix_ms: now_unix_ms(),
            latest: None,
            folder_missing: false,
            body,
        });
    }
    let item = store
        .item(&id)
        .cloned()
        .ok_or_else(|| not_found(format!("no item {id}")))?;
    let digest = item.digest();
    // A copy is the item only with the item's keys too: one holding another
    // key is the operator's other server, and goes to the undo list whole.
    let keys = applier
        .keys(&item, &ctx.item_files(&item.id))
        .map_err(failed)?;
    for (place, entry) in &copies {
        if entry.digest == digest && entry.keys == keys && entry.link.is_none() && entry.whole {
            let mark = keys_mark(keys.as_ref(), ctx.secret.as_ref());
            record_written(store, place, &item, &digest, mark);
        } else if entry.link.is_some() {
            // A link goes as a link; what it leads to stays (D-20).
            applier.remove(place, name).map_err(failed)?;
        } else {
            // Other content, or a folder holding what Hide's copy leaves
            // out (a `.git` folder): it moves whole to the undo list.
            set_aside(ctx, store, place, &item)?;
        }
    }
    Ok(())
}

/// Moves an agent's own copy of `item`'s name whole into the undo list, so
/// taking that place over loses nothing (B21, B22).
fn set_aside(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    place: &Place,
    item: &StoreItem,
) -> Result<(), StoreRefusal> {
    let id = format!(
        "{}-{}-{}",
        folder_name(&item.id),
        now_unix_ms(),
        store.trash.len()
    );
    let body = applier(item.kind).set_aside(place, &item.name, &ctx.trash_files(&id))?;
    let copy = StoreItem {
        source: Source::Copy {
            from: place.path.join(&item.name).display().to_string(),
        },
        body,
        on: true,
        off_for: BTreeSet::new(),
        fingerprint: None,
        latest: None,
        folder_missing: false,
        added_at_unix_ms: now_unix_ms(),
        // Put back as an item, it is the operator's original again.
        adopted: Some(place.key.clone()),
        ..item.clone()
    };
    push_trash(
        ctx,
        store,
        TrashEntry {
            id,
            item: copy,
            at_unix_ms: now_unix_ms(),
            cause: TrashCause::Replaced {
                place: place.key.clone(),
                path: place.path.clone(),
            },
        },
    );
    Ok(())
}

// --- Reconcile ------------------------------------------------------------------

/// Whether a place's entry is the copy Hide wrote, unedited and holding
/// nothing a copy leaves out.
/// The fingerprint [`Written::keys`] keeps of `keys`, none without keys or
/// before the store has a secret.
fn keys_mark(
    keys: Option<&mcp::KeyValues>,
    secret: Option<&crate::mcp_keys::KeysSecret>,
) -> Option<String> {
    keys.zip(secret)
        .map(|(keys, secret)| keys.fingerprint(secret))
}

fn ours(
    written: Option<&Written>,
    entry: &Entry,
    secret: Option<&crate::mcp_keys::KeysSecret>,
) -> bool {
    written.is_some_and(|written| {
        written.digest == entry.digest && written.keys == keys_mark(entry.keys.as_ref(), secret)
    }) && entry.link.is_none()
        && entry.whole
}

/// Records Hide's copy of `item` at `place`, with the fingerprint of the
/// keys written with it.
fn record_written(
    store: &mut StoreRecord,
    place: &Place,
    item: &StoreItem,
    digest: &str,
    keys: Option<String>,
) {
    let kept = store
        .written
        .entry(place.key.clone())
        .or_insert_with(|| PlaceRecord {
            kind: Some(item.kind),
            path: place.path.clone(),
            profile: !place.profiles.is_empty(),
            entries: BTreeMap::new(),
        });
    let written = Written {
        item: item.id.clone(),
        digest: digest.to_owned(),
        keys,
    };
    if kept.entries.get(&item.name) != Some(&written) {
        kept.entries.insert(item.name.clone(), written);
    }
}

fn forget_written(store: &mut StoreRecord, key: &str, name: &str) {
    if let Some(kept) = store.written.get_mut(key) {
        kept.entries.remove(name);
        if kept.entries.is_empty() {
            store.written.remove(key);
        }
    }
}

/// What a place holds of an item, as the reconcile judges it.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Held {
    Missing,
    /// Hide's copy of the item as it is now.
    Current,
    /// Hide's copy of an earlier content of the item.
    Older,
    /// The operator's, or Hide's edited by them, with an MCP server's own
    /// fields as that copy holds them.
    Theirs(Option<Server>),
}

/// Why a place could not be judged or changed, by item id and place key.
#[derive(Default)]
struct PlaceFailures(BTreeMap<(String, String), Failure>);

/// What a column's switch and agents ask of its places for an item.
struct Want {
    /// The column's switch is on for the item and an agent it reaches is on
    /// in Settings › Agents: the item stays where it is.
    kept: bool,
    /// And one of those agents is found here: the item goes in.
    now: bool,
}

fn want(ctx: &Ctx<'_>, applier: &dyn Applier, item: &StoreItem, column: &Column) -> Want {
    let on = item.on_for(&column.id) && applier.supports(item, column).is_ok();
    let agents: Vec<&String> = column
        .agents
        .iter()
        .filter(|agent| ctx.enabled(agent))
        .collect();
    Want {
        kept: on && !agents.is_empty(),
        now: on && agents.iter().any(|agent| ctx.installed(agent)),
    }
}

/// Puts the item in or takes it out of each place, by its column's switch.
fn reconcile(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    item: &StoreItem,
    places: &[Place],
    failures: &mut PlaceFailures,
) {
    let applier = applier(item.kind);
    let columns = applier.columns();
    let digest = item.digest();
    // The keys every write of this pass puts in with the item.
    let keys = match applier.keys(item, &ctx.item_files(&item.id)) {
        Ok(keys) => keys_mark(keys.as_ref(), ctx.secret.as_ref()),
        Err(reason) => {
            log("store.keys_unreadable", Some(&item.id), &reason);
            for place in places {
                failures.0.insert(
                    (item.id.clone(), place.key.clone()),
                    Failure::new(CellFailure::Keys, reason.clone()),
                );
            }
            return;
        }
    };
    for place in places {
        let governing: Vec<&Column> = columns
            .iter()
            .filter(|column| place.governed_by(&column.id))
            .collect();
        if governing.is_empty() {
            continue;
        }
        // One folder two columns name holds the item while either wants it.
        let want = governing
            .iter()
            .map(|column| want(ctx, applier, item, column))
            .fold(
                Want {
                    kept: false,
                    now: false,
                },
                |all, one| Want {
                    kept: all.kept || one.kept,
                    now: all.now || one.now,
                },
            );
        let held = match judge(store, applier, place, item, &digest, ctx.secret.as_ref()) {
            Ok(held) => held,
            Err(reason) => {
                log("store.place_unreadable", Some(&item.id), &reason);
                failures.0.insert(
                    (item.id.clone(), place.key.clone()),
                    Failure::new(CellFailure::Unreadable, reason),
                );
                continue;
            }
        };
        let stale = store
            .stale
            .get(&item.id)
            .is_some_and(|places| places.contains(&place.key));
        let result = if want.now && place.blocked.is_none() {
            match held {
                Held::Missing | Held::Older => applier.write(ctx, place, item).map(|written| {
                    record_written(store, place, item, &written, keys.clone());
                    true
                }),
                // Hide's copy, written before the item's keys changed.
                Held::Current if stale => applier.write(ctx, place, item).map(|written| {
                    record_written(store, place, item, &written, keys.clone());
                    true
                }),
                Held::Current => {
                    record_written(store, place, item, &digest, keys.clone());
                    Ok(false)
                }
                Held::Theirs(_) => Ok(false),
            }
        } else if !want.kept {
            match held {
                Held::Current | Held::Older => applier.remove(place, &item.name).map(|()| {
                    forget_written(store, &place.key, &item.name);
                    true
                }),
                // An edited copy is the operator's now, and the record lets
                // it go.
                Held::Missing | Held::Theirs(_) => {
                    forget_written(store, &place.key, &item.name);
                    Ok(false)
                }
            }
        } else {
            Ok(false)
        };
        if stale && (matches!(result, Ok(true)) || !written_at(store, place, item)) {
            unstale(store, &item.id, &place.key);
        }
        match result {
            Ok(true) => {
                let changed = store.changed.entry(item.id.clone()).or_default();
                for column in &place.columns {
                    changed.insert(column.clone(), now_unix_ms());
                }
            }
            Ok(false) => {}
            Err(reason) => {
                log("store.place_failed", Some(&item.id), &reason);
                failures.0.insert(
                    (item.id.clone(), place.key.clone()),
                    Failure::new(CellFailure::Unwritable, reason),
                );
            }
        }
    }
}

/// Whether the record holds Hide's copy of `item` at `place`.
fn written_at(store: &StoreRecord, place: &Place, item: &StoreItem) -> bool {
    store
        .written
        .get(&place.key)
        .and_then(|kept| kept.entries.get(&item.name))
        .is_some_and(|written| written.item == item.id)
}

fn unstale(store: &mut StoreRecord, id: &str, key: &str) {
    if let Some(places) = store.stale.get_mut(id) {
        places.remove(key);
        if places.is_empty() {
            store.stale.remove(id);
        }
    }
}

/// Lets a stale mark go once the record no longer holds Hide's copy of the
/// item there: an item that went, a place a pass no longer lists.
fn trim_stale(store: &mut StoreRecord) {
    let written = store.written.clone();
    store.stale.retain(|id, places| {
        places.retain(|key| {
            written
                .get(key)
                .is_some_and(|kept| kept.entries.values().any(|entry| entry.item == *id))
        });
        !places.is_empty()
    });
}

fn judge(
    store: &StoreRecord,
    applier: &dyn Applier,
    place: &Place,
    item: &StoreItem,
    digest: &str,
    secret: Option<&crate::mcp_keys::KeysSecret>,
) -> Result<Held, String> {
    let written = store
        .written
        .get(&place.key)
        .and_then(|kept| kept.entries.get(&item.name))
        .filter(|written| written.item == item.id);
    Ok(match applier.entry(place, &item.name)? {
        None => Held::Missing,
        Some(entry) if !ours(written, &entry, secret) => Held::Theirs(entry.server),
        Some(entry) if entry.digest == digest => Held::Current,
        Some(_) => Held::Older,
    })
}

/// The place a record kept, rebuilt to take its entries out.
fn recorded_place(key: &str, place: &PlaceRecord) -> Place {
    Place {
        key: key.to_owned(),
        path: place.path.clone(),
        columns: Vec::new(),
        profiles: Vec::new(),
        blocked: None,
        fresh: true,
    }
}

fn take_out_if_ours(
    applier: &dyn Applier,
    place: &Place,
    name: &str,
    written: &Written,
    secret: Option<&crate::mcp_keys::KeysSecret>,
) -> Result<bool, String> {
    match applier.entry(place, name)? {
        Some(entry) if ours(Some(written), &entry, secret) => {
            applier.remove(place, name).map(|()| true)
        }
        _ => Ok(false),
    }
}

/// Takes Hide's copies out of a place no pass lists any more (a profile
/// that went), and out of every place for an item no longer in the store
/// (a delete).
fn release_orphans(
    ctx: &Ctx<'_>,
    store: &mut StoreRecord,
    places: &BTreeMap<ItemKind, Vec<Place>>,
) {
    let listed: BTreeSet<&str> = places
        .values()
        .flatten()
        .map(|place| place.key.as_str())
        .collect();
    let items: BTreeSet<&str> = store.items.iter().map(|item| item.id.as_str()).collect();
    let mut gone = Vec::new();
    for (key, place) in &store.written {
        let Some(kind) = place.kind else { continue };
        let place_listed = listed.contains(key.as_str());
        // A profile's place is unlisted while the profiles cannot be read,
        // which is no reason to take anything out.
        if !place_listed && place.profile && !ctx.accounts.known() {
            continue;
        }
        for (name, written) in &place.entries {
            if !place_listed || !items.contains(written.item.as_str()) {
                gone.push((
                    key.clone(),
                    kind,
                    name.clone(),
                    written.clone(),
                    place.clone(),
                ));
            }
        }
    }
    for (key, kind, name, written, kept) in gone {
        let at = places
            .get(&kind)
            .and_then(|places| places.iter().find(|place| place.key == key))
            .cloned()
            .unwrap_or_else(|| recorded_place(&key, &kept));
        match take_out_if_ours(applier(kind), &at, &name, &written, ctx.secret.as_ref()) {
            Ok(_) => forget_written(store, &key, &name),
            Err(reason) => log("store.release_failed", Some(&written.item), &reason),
        }
    }
}

// --- The report -------------------------------------------------------------------

fn report(
    ctx: &Ctx<'_>,
    store: &StoreRecord,
    places: &BTreeMap<ItemKind, Vec<Place>>,
    failures: &PlaceFailures,
    outcomes: Vec<ChangeOutcome>,
) -> StoreReport {
    StoreReport {
        columns: appliers()
            .into_iter()
            .map(|applier| (applier.kind(), applier.columns()))
            .collect(),
        items: store
            .items
            .iter()
            .map(|item| item_report(ctx, store, item, &places[&item.kind], failures))
            .collect(),
        unmanaged: unmanaged(store, places),
        trash: store
            .trash
            .iter()
            .map(|entry| TrashReport {
                id: entry.id.clone(),
                kind: entry.item.kind,
                name: entry.item.name.clone(),
                at_unix_ms: entry.at_unix_ms,
                cause: entry.cause.clone(),
            })
            .collect(),
        previews: store
            .previews
            .iter()
            .map(|preview| Preview {
                agents: ctx.reaching(ItemKind::Skill),
                skills: preview
                    .skills
                    .iter()
                    .map(|skill| FoundSkill {
                        taken: name_free(store, ItemKind::Skill, &skill.name).is_err(),
                        ..skill.clone()
                    })
                    .collect(),
                ..preview.clone()
            })
            .collect(),
        diffs: store.diffs.clone(),
        mcp_previews: mcp::previews(ctx, store),
        home: Some(ctx.home().display().to_string()),
        avatars: avatar::reported(
            store_dir(ctx.home(), false).ok().as_deref(),
            store.items.iter().filter_map(StoreItem::owner),
        ),
        checked_at_unix_ms: store.checked_at_unix_ms,
        follows: store
            .items
            .iter()
            .any(|item| matches!(item.source, Source::Folder { .. })),
        outcomes,
        refused: None,
        refusal: None,
    }
}

fn item_report(
    ctx: &Ctx<'_>,
    store: &StoreRecord,
    item: &StoreItem,
    places: &[Place],
    failures: &PlaceFailures,
) -> ItemReport {
    let applier = applier(item.kind);
    let digest = item.digest();
    let held: Vec<(&Place, Result<Held, Failure>)> = places
        .iter()
        .map(|place| {
            let held = match failures.0.get(&(item.id.clone(), place.key.clone())) {
                Some(failure) => Err(failure.clone()),
                None => judge(store, applier, place, item, &digest, ctx.secret.as_ref())
                    .map_err(|reason| Failure::new(CellFailure::Unreadable, reason)),
            };
            (place, held)
        })
        .collect();
    let cells = applier
        .columns()
        .iter()
        .map(|column| {
            let mine: Vec<&(&Place, Result<Held, Failure>)> = held
                .iter()
                .filter(|(place, _)| place.governed_by(&column.id))
                .collect();
            let judged = cell_state(ctx, item, applier, column, &mine);
            // A hand-written cell carries the server its place holds, the
            // one a take-over of that place brings in.
            let theirs = match judged.state {
                CellState::HandWritten => mine.iter().find_map(|(place, held)| match held {
                    Ok(Held::Theirs(server))
                        if judged.about.is_some_and(|about| about.key == place.key) =>
                    {
                        server.clone()
                    }
                    _ => None,
                }),
                _ => None,
            };
            let place = judged
                .about
                .or_else(|| mine.first().map(|(place, _)| *place));
            Cell {
                column: column.id.clone(),
                state: judged.state,
                reason: judged.reason,
                failure: judged.failure,
                place: place.map(|place| place.key.clone()),
                path: place.map(|place| place.path.display().to_string()),
                changed_at_unix_ms: store
                    .changed
                    .get(&item.id)
                    .and_then(|changed| changed.get(&column.id))
                    .copied(),
                server: theirs,
            }
        })
        .collect();
    ItemReport {
        id: item.id.clone(),
        kind: item.kind,
        name: item.name.clone(),
        owner: item.owner().map(str::to_owned),
        source: item.source.clone(),
        version: item.version(),
        update: item.update(),
        on: item.on,
        off_for: item.off_for.clone(),
        folder_missing: item.folder_missing,
        description: match &item.body {
            ItemBody::Skill(body) => body.description.clone(),
            ItemBody::Mcp(_) => None,
        },
        files: match &item.body {
            ItemBody::Skill(body) => body.files.clone(),
            ItemBody::Mcp(_) => Vec::new(),
        },
        server: match &item.body {
            ItemBody::Mcp(body) => Some(body.server.clone()),
            ItemBody::Skill(_) => None,
        },
        cells,
    }
}

/// A cell as judged: its state, why, the failure's kind for a failed one,
/// and the place the state speaks of when it is one of the column's places
/// in particular.
struct Judged<'p> {
    state: CellState,
    reason: Option<String>,
    failure: Option<CellFailure>,
    about: Option<&'p Place>,
}

impl<'p> Judged<'p> {
    fn plain(state: CellState, reason: Option<String>, about: Option<&'p Place>) -> Self {
        Self {
            state,
            reason,
            failure: None,
            about,
        }
    }

    fn failed(place: &'p Place, failure: Failure) -> Self {
        Self {
            state: CellState::Failed,
            reason: Some(failure.reason),
            failure: Some(failure.cause),
            about: Some(place),
        }
    }
}

fn cell_state<'p>(
    ctx: &Ctx<'_>,
    item: &StoreItem,
    applier: &dyn Applier,
    column: &Column,
    places: &[&(&'p Place, Result<Held, Failure>)],
) -> Judged<'p> {
    let failed = places
        .iter()
        .find_map(|(place, held)| held.as_ref().err().map(|failure| (*place, failure.clone())));
    if let Err(reason) = applier.supports(item, column) {
        return Judged::plain(CellState::Unsupported, Some(reason), None);
    }
    if !column.agents.iter().any(|agent| ctx.enabled(agent)) {
        return Judged::plain(CellState::AgentOff, None, None);
    }
    if !item.on_for(&column.id) {
        let still: Vec<&Place> = places
            .iter()
            .filter(|(_, held)| matches!(held, Ok(Held::Current | Held::Older)))
            .map(|(place, _)| *place)
            .collect();
        // A folder another column's switch keeps the item in is that
        // column's to take it out of.
        let keeping: BTreeSet<&str> = still
            .iter()
            .flat_map(|place| place.columns.iter())
            .filter(|other| **other != column.id && item.on_for(other))
            .map(String::as_str)
            .collect();
        // A copy Hide could not take out is still read: the cell is not off.
        return match (failed, still.first()) {
            (Some((place, failure)), _) => Judged::failed(place, failure),
            (None, Some(place)) if !keeping.is_empty() => Judged::plain(
                CellState::Off,
                Some(format!(
                    "its folder is also the {} column's, whose switch keeps the {} there",
                    keeping.into_iter().collect::<Vec<_>>().join(", "),
                    item.kind.code()
                )),
                Some(*place),
            ),
            (None, Some(place)) => Judged::failed(
                place,
                Failure::new(
                    CellFailure::Pending,
                    "Hide's copy is still in place; the next pass takes it out",
                ),
            ),
            (None, None) => Judged::plain(CellState::Off, None, None),
        };
    }
    if let Some((place, failure)) = failed {
        return Judged::failed(place, failure);
    }
    if let Some((place, _)) = places
        .iter()
        .find(|(_, held)| matches!(held, Ok(Held::Theirs(_))))
    {
        return Judged::plain(
            CellState::HandWritten,
            Some(format!(
                "{} already holds a {} named {} that Hide did not write; Hide left it alone",
                place.path.display(),
                item.kind.code(),
                item.name
            )),
            Some(*place),
        );
    }
    if !column
        .agents
        .iter()
        .any(|agent| ctx.enabled(agent) && ctx.installed(agent))
    {
        return Judged::plain(
            CellState::Waiting,
            Some("none of its agents is found on this machine".to_owned()),
            None,
        );
    }
    if let Some((place, _)) = places.iter().find(|(place, _)| place.blocked.is_some()) {
        return Judged::plain(CellState::Waiting, place.blocked.clone(), Some(*place));
    }
    match places
        .iter()
        .find(|(_, held)| !matches!(held, Ok(Held::Current)))
    {
        None => Judged::plain(CellState::On, None, None),
        Some((place, _)) => Judged::failed(
            place,
            Failure::new(
                CellFailure::Pending,
                "the copy is not in place yet; the next pass tries again",
            ),
        ),
    }
}

/// The entries agents hold that are not Hide's and whose name is no
/// item's, grouped by name ('에이전트에만 있음', B19). A name that is an item's
/// shows in that item's cell instead (B22).
fn unmanaged(store: &StoreRecord, places: &BTreeMap<ItemKind, Vec<Place>>) -> Vec<Unmanaged> {
    let mut found: BTreeMap<(ItemKind, String), Vec<FoundCopy>> = BTreeMap::new();
    // What tells the copies apart, a digest and an MCP server's keys as
    // fingerprinted for this process alone, kept beside them and never
    // reported.
    type Sameness = (String, Option<mcp::KeyValues>);
    let mut sameness: BTreeMap<(ItemKind, String), Vec<Sameness>> = BTreeMap::new();
    for (kind, places) in places {
        let applier = applier(*kind);
        let columns = applier.columns();
        for place in places {
            let entries = match applier.entries(place) {
                Ok(entries) => entries,
                Err(reason) => {
                    log("store.scan_failed", None, &reason);
                    continue;
                }
            };
            let agents: Vec<String> = columns
                .iter()
                .filter(|column| place.governed_by(&column.id))
                .flat_map(|column| column.agents.iter().cloned())
                .collect();
            for (name, read) in entries {
                if hides_own(&name)
                    || !valid_name(&name)
                    || store.item(&StoreItem::id_of(*kind, &name)).is_some()
                {
                    continue;
                }
                // One entry that cannot be read is listed with why, and the
                // folder's others as ever.
                let (digest, link, refused, server) = match read {
                    Ok(entry) => {
                        sameness
                            .entry((*kind, name.clone()))
                            .or_default()
                            .push((entry.digest.clone(), entry.keys));
                        (
                            entry.digest,
                            entry.link.as_ref().map(|link| link.display().to_string()),
                            None,
                            entry.server,
                        )
                    }
                    Err(reason) => (String::new(), None, Some(reason), None),
                };
                found
                    .entry((*kind, name.clone()))
                    .or_default()
                    .push(FoundCopy {
                        place: place.key.clone(),
                        agents: agents.clone(),
                        profiles: place.profiles.clone(),
                        path: applier.entry_path(place, &name).display().to_string(),
                        link,
                        digest,
                        refused,
                        server,
                    });
            }
        }
    }
    found
        .into_iter()
        .map(|((kind, name), copies)| Unmanaged {
            kind,
            same: copies.iter().all(|copy| copy.refused.is_none())
                && sameness
                    .get(&(kind, name.clone()))
                    .is_none_or(|read| read.windows(2).all(|pair| pair[0] == pair[1])),
            name,
            copies,
        })
        .collect()
}

/// The owners whose pictures the store keeps: its items' and the source
/// answers' it still holds.
pub(crate) fn avatar_owners(store: &StoreRecord) -> BTreeSet<&str> {
    store
        .items
        .iter()
        .filter_map(StoreItem::owner)
        .chain(
            store
                .previews
                .iter()
                .filter_map(|preview| preview.url.as_deref().and_then(avatar::owner)),
        )
        .collect()
}

/// Drops the previews and update previews past their limits, with their
/// staged files and the pictures only they used, and any staged folder no
/// record names. A folder a fetch is still filling outside the lock
/// (`source::FETCHING`) is left for an hour.
fn trim_staging(ctx: &Ctx<'_>, store: &mut StoreRecord) {
    while store.previews.len() > PREVIEW_LIMIT {
        store.previews.remove(0);
    }
    if let Ok(dir) = store_dir(ctx.home(), false) {
        avatar::keep_only(&dir, &avatar_owners(store));
    }
    while store.diffs.len() > PREVIEW_LIMIT {
        store.diffs.remove(0);
    }
    mcp::trim_previews(store);
    let named: BTreeSet<&str> = store
        .previews
        .iter()
        .map(|preview| preview.id.as_str())
        .chain(store.diffs.iter().map(|diff| diff.id.as_str()))
        .chain(store.mcp_previews.iter().map(|preview| preview.id.as_str()))
        .collect();
    let Ok(entries) = std::fs::read_dir(ctx.staging()) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if named.contains(name.as_str()) {
            continue;
        }
        if name.starts_with(source::FETCHING) {
            let fresh = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age.as_secs() < 3600);
            if fresh {
                continue;
            }
        }
        let _ = std::fs::remove_dir_all(entry.path());
    }
}

pub(crate) fn log(kind: &str, item: Option<&str>, reason: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "kit",
            "kind": kind,
            "item": item,
            "reason": reason,
        })
    );
}
