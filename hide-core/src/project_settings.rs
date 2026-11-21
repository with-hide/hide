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
/// The longest display name kept, in characters: long enough for any real
/// project name, short enough that a row and a heading can draw it.
pub(crate) const NAME_LENGTH: usize = 64;

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
    /// The name every screen and agent reads for the project in place of its
    /// folder's; absent is the folder's. The folder on disk is never renamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// The profile each agent starts with in this project, by agent id
    /// (PRD agent-profiles D-31): the profile's id on the project's machine.
    /// An agent it does not name, or a profile its machine no longer lists,
    /// starts with the agent's default (`launch::pick`). Absent when empty,
    /// so a record without it is written as before, and an older build keeps
    /// it among `unknown` and reads the record at the same version.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agent_profiles: BTreeMap<String, String>,
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
    /// The display name; an empty one (after trimming) gives the project its
    /// folder's name back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
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
    /// Each agent named here starts with this profile in the project; `null`
    /// gives the agent its default back. Agents not named keep theirs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_profiles: Option<BTreeMap<String, Option<String>>>,
}

impl ProjectSettings {
    pub(crate) fn new(device: &str, root: &str) -> Self {
        Self {
            version: PLAIN_VERSION,
            device_id: device.to_owned(),
            root: root.to_owned(),
            name: None,
            repository: None,
            issue_source: None,
            new_work: None,
            base_branch: None,
            auto_fetch: None,
            link_scopes: BTreeMap::new(),
            guardrails: None,
            agent_profiles: BTreeMap::new(),
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
        if let Some(name) = patch.name {
            next.name = display_name(&name)?;
        }
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
        for (agent, profile) in patch.agent_profiles.unwrap_or_default() {
            if hide_agent_adapter::adapter(&agent).is_none_or(|adapter| adapter.id != agent) {
                return Err("project_settings_invalid_agent");
            }
            match profile {
                Some(profile) if valid_profile_id(&profile) => {
                    next.agent_profiles.insert(agent, profile);
                }
                Some(_) => return Err("project_settings_invalid_profile"),
                None => {
                    next.agent_profiles.remove(&agent);
                }
            }
        }
        Ok(next)
    }

    /// The protections in force: none for a record without them.
    pub(crate) fn rules(&self) -> Option<&Rules> {
        self.guardrails.as_ref().filter(|rules| rules.any())
    }
}

/// A profile id as a node makes them: short, with no control character.
pub(crate) fn valid_profile_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
}

/// The name kept for what the operator typed: trimmed, none when nothing is
/// left. A line break or another control character is refused rather than
/// dropped, since a name the screen would draw differently from what was
/// typed is not the operator's; so is one past [`NAME_LENGTH`]. Another
/// project may carry the same name: the path tells them apart where it matters.
fn display_name(typed: &str) -> Result<Option<String>, &'static str> {
    let name = typed.trim();
    if name.chars().any(char::is_control) || name.chars().count() > NAME_LENGTH {
        return Err("project_settings_name_invalid");
    }
    Ok((!name.is_empty()).then(|| name.to_owned()))
}

/// Gives each project the name the operator set for it on its device and
/// root. A project without one keeps the label the catalog gave it, its
/// registration's or its folder's.
pub(crate) fn apply_names(
    records: &BTreeMap<String, ProjectSettings>,
    projects: &mut [crate::model::WorkspaceSnapshot],
) {
    if records.values().all(|record| record.name.is_none()) {
        return;
    }
    for project in projects.iter_mut() {
        if let Some(name) = get(records, &project.device_id, &project.path)
            .and_then(|record| record.name.as_deref())
        {
            project.label = name.to_owned();
        }
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

/// The profile `agent` starts with in the project at `root` on `device`
/// (D-31), by the id its record keeps; whether the machine still lists it
/// is the start's to judge (`launch::pick`).
pub(crate) fn agent_profile<'a>(
    records: &'a BTreeMap<String, ProjectSettings>,
    device: &str,
    root: &str,
    agent: &str,
) -> Option<&'a str> {
    get(records, device, root)?
        .agent_profiles
        .get(agent)
        .map(String::as_str)
}

/// A path can carry several branches and each branch can acquire a new bare token.
pub(crate) fn link_scope_key(path: &str, branch: Option<&str>, number: u32) -> String {
    format!("{path}\0{}\0{number}", branch.unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RELEASED: &str = include_str!("../tests/fixtures/release-1.2.4/project-settings.json");

    /// The released 1.2.4 record as that build declares it: its own fields,
    /// and every other one kept in `unknown`.
    #[derive(Deserialize, Serialize)]
    struct Released {
        version: u32,
        device_id: String,
        root: String,
        #[serde(flatten)]
        unknown: serde_json::Map<String, serde_json::Value>,
    }

    fn released() -> BTreeMap<String, serde_json::Value> {
        serde_json::from_str(RELEASED).unwrap()
    }

    #[test]
    fn a_record_without_agent_profiles_is_written_as_the_released_build_wrote_it() {
        for (name, written) in released() {
            let record: ProjectSettings = serde_json::from_value(written.clone()).unwrap();
            assert!(record.agent_profiles.is_empty(), "{name}");
            assert_eq!(serde_json::to_value(&record).unwrap(), written, "{name}");
        }
    }

    #[test]
    fn a_record_with_agent_profiles_keeps_its_version_and_the_released_build_keeps_the_field() {
        for (name, written) in released() {
            let record: ProjectSettings = serde_json::from_value(written).unwrap();
            let patched = record
                .patched(Patch {
                    agent_profiles: Some(BTreeMap::from([(
                        "claude-code".to_owned(),
                        Some("p-work".to_owned()),
                    )])),
                    ..Patch::default()
                })
                .unwrap();
            // No version change: 1.2.4 reads versions 1 and 2 (D-31).
            assert_eq!(patched.version, record.version, "{name}");
            let new = serde_json::to_value(&patched).unwrap();
            assert_eq!(new["agent_profiles"]["claude-code"], "p-work", "{name}");
            // 1.2.4 reads it, and its save writes the field back unchanged.
            let old: Released = serde_json::from_value(new.clone()).unwrap();
            assert_eq!(old.version, record.version);
            assert_eq!(serde_json::to_value(&old).unwrap(), new, "{name}");
            // Clearing the agent's choice writes the record as before.
            let cleared = patched
                .patched(Patch {
                    agent_profiles: Some(BTreeMap::from([("claude-code".to_owned(), None)])),
                    ..Patch::default()
                })
                .unwrap();
            assert_eq!(cleared, record, "{name}");
        }
    }

    #[test]
    fn a_choice_for_an_agent_hide_does_not_know_or_an_empty_profile_is_refused() {
        let record = ProjectSettings::new("device", "/root");
        let patch = |agent: &str, profile: &str| Patch {
            agent_profiles: Some(BTreeMap::from([(
                agent.to_owned(),
                Some(profile.to_owned()),
            )])),
            ..Patch::default()
        };
        assert_eq!(
            record.patched(patch("claude", "p")),
            Err("project_settings_invalid_agent"),
            "an alias is not the agent's id"
        );
        assert_eq!(
            record.patched(patch("claude-code", "")),
            Err("project_settings_invalid_profile")
        );
        assert!(record.patched(patch("codex", "p-team")).is_ok());
    }
}
