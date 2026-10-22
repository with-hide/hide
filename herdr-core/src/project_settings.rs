//! Private Project choices, shared by every checkout of one machine's
//! canonical main-worktree root. An absent field inherits the old choice;
//! an explicit Auto shadows it. No path resolution or I/O belongs here.

use std::collections::BTreeMap;
use std::path::Path;

use hide_agent_hooks::guardrails::PathRefusal;
use serde::{Deserialize, Serialize};

/// The newest record this build reads and writes. A record that carries
/// guardrails is version 2, which an older build reads as newer and keeps
/// read-only; one without them stays version 1 (PRD agent-guardrails D-23).
pub(crate) const VERSION: u32 = 2;
const PLAIN_VERSION: u32 = 1;
pub(crate) const LIMIT: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum RepositoryConnection {
    Auto,
    Repository { repository: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueSourceChoice {
    Auto,
    Github,
    Local,
}

impl IssueSourceChoice {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Github => "github",
            Self::Local => "local",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NewWorkLocation {
    Checkout,
    Worktree,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum BaseBranchChoice {
    Auto,
    Branch { branch: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ProjectSettings {
    pub version: u32,
    pub device_id: String,
    pub root: String,
    #[serde(default)]
    pub repository: Option<RepositoryConnection>,
    #[serde(default)]
    pub issue_source: Option<IssueSourceChoice>,
    #[serde(default)]
    pub new_work: Option<NewWorkLocation>,
    #[serde(default)]
    pub base_branch: Option<BaseBranchChoice>,
    /// Whether Hide fetches the repository's `origin` in the background
    /// (PRD git-freshness D-01); absent, as an older record reads, is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_fetch: Option<bool>,
    /// Bare legacy issue tokens retain the repository they already named.
    /// New explicit links carry their own owner/repo and need no binding.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub link_scopes: BTreeMap<String, LinkScope>,
    /// The protections of Working style; absent is every one off, as a
    /// version 1 record reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guardrails: Option<Rules>,
    /// Fields a newer build wrote, kept so this one's save of the record
    /// loses none of them (a record of a newer version is read-only here).
    #[serde(flatten, default)]
    pub unknown: serde_json::Map<String, serde_json::Value>,
}

pub use hide_agent_hooks::guardrails::Rules;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct LinkScope {
    pub branch: Option<String>,
    pub reference: crate::issues::IssueReference,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Patch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<RepositoryConnection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_source: Option<IssueSourceChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_work: Option<NewWorkLocation>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_branch: Option<BaseBranchChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_fetch: Option<bool>,
    /// The whole set of protections, replacing the record's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrails: Option<Rules>,
}

impl ProjectSettings {
    pub(crate) fn new(device: &str, root: &str) -> Self {
        Self {
            version: PLAIN_VERSION,
            device_id: device.to_owned(),
            root: root.to_owned(),
            repository: None,
            issue_source: None,
            new_work: None,
            base_branch: None,
            auto_fetch: None,
            link_scopes: BTreeMap::new(),
            guardrails: None,
            unknown: serde_json::Map::new(),
        }
    }

    pub(crate) fn link_scope(
        &self,
        path: &str,
        branch: Option<&str>,
        number: u32,
    ) -> Option<&LinkScope> {
        self.link_scopes
            .get(&link_scope_key(path, branch, number))
            .or_else(|| {
                // The first additive record keyed only by checkout path.
                self.link_scopes.get(path).filter(|scope| {
                    scope.branch.as_deref() == branch && scope.reference.number == number
                })
            })
    }

    pub(crate) fn patched(&self, patch: Patch) -> Result<Self, &'static str> {
        if !readable(self.version) {
            return Err("project_settings_newer_version");
        }
        let mut next = self.clone();
        if let Some(repository) = patch.repository {
            if let RepositoryConnection::Repository { repository } = &repository
                && (repository.len() > 256 || !crate::issues::is_repository(repository))
            {
                return Err("project_settings_invalid_repository");
            }
            next.repository = Some(repository);
        }
        if let Some(source) = patch.issue_source {
            next.issue_source = Some(source);
        }
        if let Some(location) = patch.new_work {
            next.new_work = Some(location);
        }
        if let Some(base) = patch.base_branch {
            next.base_branch = Some(base);
        }
        if let Some(auto_fetch) = patch.auto_fetch {
            next.auto_fetch = Some(auto_fetch);
        }
        if let Some(mut rules) = patch.guardrails {
            // A field kept for a newer build is the record's: the shell sends
            // back what it was shown, so the record's own copy is kept, and
            // nothing a change names beyond this build's protections reaches
            // the policy every hook reads.
            rules.unknown = next
                .guardrails
                .as_ref()
                .map(|kept| kept.unknown.clone())
                .unwrap_or_default();
            rules.validate().map_err(PathRefusal::code)?;
            next.guardrails = Some(rules);
            next.version = VERSION;
        }
        Ok(next)
    }

    /// The protections in force: none for a record without them.
    pub(crate) fn rules(&self) -> Option<&Rules> {
        self.guardrails.as_ref().filter(|rules| rules.any())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct WriteStatus {
    pub device_id: String,
    pub root: String,
    pub request_id: String,
    pub state: WriteState,
    pub error: Option<String>,
    pub patch: Patch,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteState {
    Pending,
    Saved,
    Failed,
}

/// Catalog roots already come from the owning node's canonical Project
/// facts. Hash those facts exactly as Memory does, without an fs read under
/// Runtime or interpreting a remote path on this machine.
pub(crate) fn key(device: &str, root: &str) -> String {
    hide_project::project_id(device, Path::new(root))
}

pub(crate) fn get<'a>(
    records: &'a BTreeMap<String, ProjectSettings>,
    device: &str,
    root: &str,
) -> Option<&'a ProjectSettings> {
    records.get(&key(device, root)).filter(|record| {
        readable(record.version) && record.device_id == device && record.root == root
    })
}

fn readable(version: u32) -> bool {
    (PLAIN_VERSION..=VERSION).contains(&version)
}

pub(crate) fn issue_choice<'a>(
    records: &'a BTreeMap<String, ProjectSettings>,
    legacy: &'a BTreeMap<String, String>,
    device: &str,
    root: &str,
) -> Option<&'a str> {
    get(records, device, root)
        .and_then(|record| record.issue_source)
        .map(IssueSourceChoice::as_str)
        .or_else(|| legacy.get(root).map(String::as_str))
}

pub(crate) fn base_choice<'a>(
    records: &'a BTreeMap<String, ProjectSettings>,
    legacy: &'a BTreeMap<String, String>,
    device: &str,
    root: &str,
) -> Option<&'a str> {
    match get(records, device, root).and_then(|record| record.base_branch.as_ref()) {
        Some(BaseBranchChoice::Auto) => None,
        Some(BaseBranchChoice::Branch { branch }) => Some(branch),
        None => legacy.get(root).map(String::as_str),
    }
}

/// Whether Hide fetches this repository in the background: on unless the
/// operator turned it off (D-01).
pub(crate) fn auto_fetch(
    records: &BTreeMap<String, ProjectSettings>,
    device: &str,
    root: &str,
) -> bool {
    get(records, device, root)
        .and_then(|record| record.auto_fetch)
        .unwrap_or(true)
}

/// A path can carry several branches and each branch can acquire a new bare token.
pub(crate) fn link_scope_key(path: &str, branch: Option<&str>, number: u32) -> String {
    format!("{path}\0{}\0{number}", branch.unwrap_or(""))
}
