//! Where skills come from: a git repository (GitHub's, or any the account's
//! git reaches) or a folder on this machine (D-08, D-20, D-30).
//!
//! A repository is read with the account's own `git`, as an owned child
//! with a deadline (`crate::process`), so a private repository opens with
//! the machine's own git or gh login and Hide keeps no token. Git is told
//! never to ask a person: a repository the login cannot read answers
//! [`StoreRefusalCause::NoAccess`] (B4), never a prompt.
//!
//! Everything here runs before the kit's account lock is taken, into a
//! staging folder of its own (`FETCHING`), which the pass under the lock
//! names for the confirm screen ([`Fetched::keep_into`]).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::skill::{self, Discovered};
use super::{
    Ctx, FileChange, FileChangeKind, FoundSkill, ItemKind, Latest, Preview, Source, StoreChange,
    StoreItem, StoreRecord, StoreRefusal, StoreRefusalCause, UpdateDiff, Version, folder_name,
    now_unix_ms,
};
use crate::KitTarget;

/// What the operator gave as a source (B3, B5).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceSpec {
    /// A GitHub address (`https://github.com/owner/repo`, with
    /// `/tree/<branch>/<folder>` or not, `owner/repo`, `git@github.com:...`)
    /// or any other git address.
    Git { url: String },
    /// A folder on this machine.
    Folder { path: String },
}

/// The prefix of a staging folder a fetch is still filling.
pub(crate) const FETCHING: &str = ".fetch-";

const CLONE_DEADLINE: Duration = Duration::from_secs(120);
const ASK_DEADLINE: Duration = Duration::from_secs(30);
const SUMMARY_LIMIT: usize = 20;

/// Whether a change reaches a source.
pub(crate) fn reaches_out(change: &StoreChange) -> bool {
    matches!(
        change,
        StoreChange::Inspect { .. } | StoreChange::CheckUpdates | StoreChange::PreviewUpdate { .. }
    )
}

/// What the sources answered for one pass.
#[derive(Default)]
pub(crate) struct Fetched {
    /// The store folder could not be used, so nothing was asked.
    refused: Option<String>,
    previews: Vec<(SourceSpec, Result<Preview, StoreRefusal>)>,
    latest: Option<Result<Vec<(String, Latest)>, StoreRefusal>>,
    diffs: Vec<(String, Result<UpdateDiff, StoreRefusal>)>,
    /// Owners' pictures downloaded for the pass to keep under the lock.
    avatars: Vec<(String, Vec<u8>)>,
}

impl Fetched {
    pub(crate) fn refused(reason: String) -> Self {
        Self {
            refused: Some(reason),
            ..Self::default()
        }
    }

    fn missing(&self) -> StoreRefusal {
        StoreRefusal::new(
            StoreRefusalCause::Failed,
            self.refused
                .clone()
                .unwrap_or_else(|| "the source was not read".to_owned()),
        )
    }

    pub(crate) fn preview_of(&self, source: &SourceSpec) -> Result<&Preview, StoreRefusal> {
        match self.previews.iter().find(|(asked, _)| asked == source) {
            Some((_, Ok(preview))) => Ok(preview),
            Some((_, Err(refusal))) => Err(refusal.clone()),
            None => Err(self.missing()),
        }
    }

    pub(crate) fn checked(&self) -> Result<(), StoreRefusal> {
        match &self.latest {
            Some(Ok(_)) => Ok(()),
            Some(Err(refusal)) => Err(refusal.clone()),
            None => Err(self.missing()),
        }
    }

    pub(crate) fn diff_of(&self, item: &str) -> Result<&UpdateDiff, StoreRefusal> {
        match self.diffs.iter().find(|(asked, _)| asked == item) {
            Some((_, Ok(diff))) => Ok(diff),
            Some((_, Err(refusal))) => Err(refusal.clone()),
            None => Err(self.missing()),
        }
    }

    /// Names what was fetched in the store, under the lock: a staged folder
    /// takes its final name, the previews and update previews join the
    /// record, and an update check's answers reach their items.
    pub(crate) fn keep_into(&self, ctx: &Ctx<'_>, store: &mut StoreRecord) {
        let staging = ctx.staging();
        let settle = |id: &str| {
            let fetching = staging.join(format!("{FETCHING}{id}"));
            if fetching.exists()
                && let Err(error) = std::fs::rename(&fetching, staging.join(id))
            {
                super::log("store.stage_failed", None, &error.to_string());
            }
        };
        for (_, preview) in &self.previews {
            if let Ok(preview) = preview {
                settle(&preview.id);
                store.previews.push(preview.clone());
            }
        }
        for (_, diff) in &self.diffs {
            if let Ok(diff) = diff {
                settle(&diff.id);
                store.diffs.retain(|kept| kept.item != diff.item);
                store.diffs.push(diff.clone());
            }
        }
        if let Some(Ok(latest)) = &self.latest {
            for (id, found) in latest {
                if let Some(item) = store.item_mut(id) {
                    item.latest = Some(found.clone());
                }
            }
            store.checked_at_unix_ms = Some(now_unix_ms());
        }
        // The pictures are let in here, under the lock, against the folder
        // and the store as they are now.
        let used = super::avatar_owners(store);
        for (owner, bytes) in &self.avatars {
            super::avatar::admit(&ctx.dir, owner, bytes, &used);
        }
    }
}

/// Asks every source `changes` names, outside the account lock.
pub(crate) fn fetch(
    target: &KitTarget,
    dir: &Path,
    record: &StoreRecord,
    changes: &[StoreChange],
) -> Fetched {
    let mut fetched = Fetched::default();
    let staging = dir.join("staging");
    for change in changes {
        match change {
            StoreChange::Inspect { source } => {
                let preview = inspect(target, &staging, source);
                // The owner's picture comes with the source it was read with,
                // and stays while the answer or an item of that owner does.
                if let (SourceSpec::Git { url }, Ok(_)) = (source, &preview)
                    && let Ok(address) = parse(url)
                    && let Some(owner) = super::avatar::owner(&address.url)
                    && let Some(bytes) = super::avatar::download(dir, owner)
                {
                    fetched.avatars.push((owner.to_owned(), bytes));
                }
                fetched.previews.push((source.clone(), preview));
            }
            StoreChange::CheckUpdates => {
                fetched.latest = Some(check(target, record));
                let owners: std::collections::BTreeSet<&str> =
                    record.items.iter().filter_map(StoreItem::owner).collect();
                for owner in owners {
                    if let Some(bytes) = super::avatar::download(dir, owner) {
                        fetched.avatars.push((owner.to_owned(), bytes));
                    }
                }
            }
            StoreChange::PreviewUpdate { item } => {
                let diff = match record.item(item) {
                    Some(found) => preview_update(target, dir, &staging, found),
                    None => Err(StoreRefusal::new(
                        StoreRefusalCause::NotFound,
                        format!("no item {item}"),
                    )),
                };
                fetched.diffs.push((item.clone(), diff));
            }
            _ => {}
        }
    }
    fetched
}

pub(crate) fn fresh_id(prefix: &str) -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "{prefix}{}-{}-{}",
        now_unix_ms(),
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Where the files of `skill` from `preview` are to install from.
pub(crate) fn files_of(ctx: &Ctx<'_>, preview: &Preview, skill: &FoundSkill) -> PathBuf {
    let root = match &preview.source {
        SourceSpec::Git { .. } => ctx.staging().join(&preview.id).join("repo"),
        SourceSpec::Folder { path } => PathBuf::from(path),
    };
    join_relative(&root, &skill.path)
}

fn join_relative(root: &Path, relative: &str) -> PathBuf {
    relative
        .split('/')
        .filter(|part| !part.is_empty())
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

/// The source an installed skill keeps.
pub(crate) fn source_of(preview: &Preview, skill: &FoundSkill, from: &Path) -> Source {
    match &preview.source {
        SourceSpec::Git { .. } => Source::Git {
            url: preview.url.clone().unwrap_or_default(),
            path: skill.path.clone(),
            reference: preview.reference.clone(),
            commit: preview.commit.clone().unwrap_or_default(),
            tag: preview.tag.clone(),
        },
        SourceSpec::Folder { .. } => Source::Folder {
            path: from.display().to_string(),
        },
    }
}

/// Where an update preview staged the skill's new files.
pub(crate) fn staged_update(ctx: &Ctx<'_>, diff: &UpdateDiff) -> PathBuf {
    ctx.staging().join(&diff.id).join("skill")
}

// --- Addresses --------------------------------------------------------------------

/// A git address as Hide reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Address {
    /// What git clones.
    pub(crate) url: String,
    pub(crate) reference: Option<String>,
    /// The folder in the repository, `/` between names, empty for its root.
    pub(crate) path: String,
    /// The repository's name, for a skill at its root with no name of its
    /// own.
    pub(crate) name: String,
}

pub(crate) fn parse(text: &str) -> Result<Address, StoreRefusal> {
    let text = text.trim().trim_end_matches('/');
    let unknown = || {
        StoreRefusal::new(
            StoreRefusalCause::NotFound,
            format!("{text} is not a repository address Hide reads"),
        )
    };
    let github = text
        .strip_prefix("https://github.com/")
        .or_else(|| text.strip_prefix("http://github.com/"))
        .or_else(|| text.strip_prefix("github.com/"))
        .or_else(|| text.strip_prefix("git@github.com:"))
        .or_else(|| {
            // `owner/repo`, nothing more.
            let parts: Vec<&str> = text.split('/').collect();
            (parts.len() == 2
                && !text.contains(':')
                && parts.iter().all(|part| super::valid_name(part)))
            .then_some(text)
        });
    if let Some(rest) = github {
        let parts: Vec<&str> = rest.split('/').filter(|part| !part.is_empty()).collect();
        let [owner, repo, more @ ..] = parts.as_slice() else {
            return Err(unknown());
        };
        let repo = repo.trim_end_matches(".git");
        if !super::valid_name(owner) || !super::valid_name(repo) {
            return Err(unknown());
        }
        let (reference, path) = match more {
            [] => (None, String::new()),
            [kind, reference, path @ ..] if matches!(*kind, "tree" | "blob") => {
                (Some((*reference).to_owned()), path.join("/"))
            }
            _ => return Err(unknown()),
        };
        return Ok(Address {
            url: if text.starts_with("git@") {
                format!("git@github.com:{owner}/{repo}.git")
            } else {
                format!("https://github.com/{owner}/{repo}.git")
            },
            reference,
            path,
            name: repo.to_owned(),
        });
    }
    let scheme = ["https://", "http://", "ssh://", "git://", "file://"]
        .iter()
        .any(|scheme| text.starts_with(scheme))
        || (text.contains('@') && text.contains(':'));
    if !scheme {
        return Err(unknown());
    }
    let name = text
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(text)
        .trim_end_matches(".git")
        .to_owned();
    Ok(Address {
        url: text.to_owned(),
        reference: None,
        path: String::new(),
        name: if super::valid_name(&name) {
            name
        } else {
            "skill".to_owned()
        },
    })
}

// --- Git --------------------------------------------------------------------------

/// The account's `git`, run so it never waits on a person.
struct Git<'a> {
    target: &'a KitTarget,
    program: hide_platform::programs::Program,
}

impl<'a> Git<'a> {
    fn find(target: &'a KitTarget) -> Result<Self, StoreRefusal> {
        crate::agents::find(
            &target.home,
            target.login_shell.as_deref(),
            &target.stop,
            "git",
        )
        .map(|program| Self { target, program })
        .ok_or_else(|| {
            StoreRefusal::new(
                StoreRefusalCause::NoGit,
                "git is not found on this machine, so Hide cannot read a repository",
            )
        })
    }

    fn run(&self, args: &[&str], deadline: Duration) -> Result<String, StoreRefusal> {
        let mut env = vec![
            (
                "PATH".to_owned(),
                self.program.search().to_string_lossy().into_owned(),
            ),
            ("GIT_TERMINAL_PROMPT".to_owned(), "0".to_owned()),
            ("GCM_INTERACTIVE".to_owned(), "never".to_owned()),
        ];
        // The machine's own login: an SSH agent the account runs.
        if let Some(socket) = std::env::var_os("SSH_AUTH_SOCK") {
            env.push((
                "SSH_AUTH_SOCK".to_owned(),
                socket.to_string_lossy().into_owned(),
            ));
        }
        let finished = crate::process::run(
            self.program.path(),
            args,
            &env,
            &self.target.home,
            deadline,
            &self.target.stop,
        )
        .map_err(|reason| StoreRefusal::new(StoreRefusalCause::Unreachable, reason))?;
        if finished.succeeded() {
            return Ok(finished.stdout);
        }
        Err(classify(&finished.stderr))
    }
}

/// What git's complaint means for the operator (B4).
fn classify(stderr: &str) -> StoreRefusal {
    let lower = stderr.to_ascii_lowercase();
    let last = stderr
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("git failed")
        .trim()
        .to_owned();
    let any = |needles: &[&str]| needles.iter().any(|needle| lower.contains(needle));
    let cause = if any(&[
        "repository not found",
        "authentication failed",
        "could not read username",
        "could not read password",
        "permission denied",
        "terminal prompts disabled",
        "403",
        "access denied",
        "does not appear to be a git repository",
    ]) {
        StoreRefusalCause::NoAccess
    } else if any(&[
        "could not resolve host",
        "failed to connect",
        "connection timed out",
        "network is unreachable",
        "connection refused",
    ]) {
        StoreRefusalCause::Unreachable
    } else if any(&["remote branch", "not found in upstream"]) {
        StoreRefusalCause::NotFound
    } else {
        StoreRefusalCause::Failed
    };
    StoreRefusal::new(cause, last)
}

/// The commits of a repository's references, from `git ls-remote`: the
/// default branch's, and each tag's (an annotated tag by the commit it
/// names).
struct Remote {
    head: Option<String>,
    heads: BTreeMap<String, String>,
    tags: BTreeMap<String, String>,
}

impl Remote {
    fn ask(git: &Git<'_>, url: &str) -> Result<Self, StoreRefusal> {
        let listed = git.run(&["ls-remote", "--", url], ASK_DEADLINE)?;
        let mut remote = Remote {
            head: None,
            heads: BTreeMap::new(),
            tags: BTreeMap::new(),
        };
        for line in listed.lines() {
            let Some((commit, name)) = line.split_once('\t') else {
                continue;
            };
            if name == "HEAD" {
                remote.head = Some(commit.to_owned());
            } else if let Some(branch) = name.strip_prefix("refs/heads/") {
                remote.heads.insert(branch.to_owned(), commit.to_owned());
            } else if let Some(tag) = name.strip_prefix("refs/tags/") {
                match tag.strip_suffix("^{}") {
                    Some(tag) => {
                        remote.tags.insert(tag.to_owned(), commit.to_owned());
                    }
                    None => {
                        remote
                            .tags
                            .entry(tag.to_owned())
                            .or_insert_with(|| commit.to_owned());
                    }
                }
            }
        }
        Ok(remote)
    }

    /// The commit `reference` names, the default branch's for none.
    fn commit(&self, reference: Option<&str>) -> Option<String> {
        match reference {
            None => self.head.clone(),
            Some(name) => self
                .heads
                .get(name)
                .or_else(|| self.tags.get(name))
                .cloned()
                .or_else(|| {
                    (name.len() >= 7 && name.chars().all(|char| char.is_ascii_hexdigit()))
                        .then(|| name.to_owned())
                }),
        }
    }

    /// A tag that names `commit`, the last in name order when several do.
    fn tag_of(&self, commit: &str) -> Option<String> {
        self.tags
            .iter()
            .filter(|(_, tagged)| *tagged == commit)
            .map(|(tag, _)| tag.clone())
            .next_back()
    }
}

/// Clones `address` into `into` at its reference, `depth` commits deep,
/// and answers the commit it holds.
fn clone(
    git: &Git<'_>,
    address: &Address,
    into: &Path,
    depth: u32,
) -> Result<String, StoreRefusal> {
    let into_text = into.display().to_string();
    let depth = depth.to_string();
    let mut args = vec!["clone", "--quiet", "--no-tags", "--depth", depth.as_str()];
    // The files as the commit holds them, whatever the account's git says
    // about line endings (Git for Windows turns LF into CRLF by default):
    // the same commit is then the same skill, digest and all, on every
    // machine. A repository's own `.gitattributes` still applies.
    args.extend(["--config", "core.autocrlf=false", "--config", "core.eol=lf"]);
    // A `/tree/<commit>` address names a commit, which `--branch` does not
    // take: the clone fetches it by itself after.
    let pinned = address.reference.as_deref().filter(|reference| {
        matches!(reference.len(), 40 | 64) && reference.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if let Some(reference) = &address.reference
        && pinned.is_none()
    {
        args.extend(["--branch", reference.as_str()]);
    }
    args.extend(["--", address.url.as_str(), into_text.as_str()]);
    git.run(&args, CLONE_DEADLINE)?;
    if let Some(commit) = pinned {
        let into = into_text.as_str();
        git.run(
            &[
                "-C",
                into,
                "fetch",
                "--quiet",
                "--depth",
                depth.as_str(),
                "origin",
                commit,
            ],
            CLONE_DEADLINE,
        )?;
        git.run(
            &["-C", into, "checkout", "--quiet", "--detach", "FETCH_HEAD"],
            ASK_DEADLINE,
        )?;
    }
    let commit = git.run(
        &["-C", into_text.as_str(), "rev-parse", "HEAD"],
        ASK_DEADLINE,
    )?;
    Ok(commit.trim().to_owned())
}

// --- Inspect ----------------------------------------------------------------------

fn found_skills(found: Vec<Discovered>, prefix: &str) -> Result<Vec<FoundSkill>, StoreRefusal> {
    let mut skills = Vec::new();
    for skill in found {
        let body = skill::body_of(&skill.path)?;
        let path = [prefix, skill.relative.as_str()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("/");
        skills.push(FoundSkill {
            name: skill.name,
            path,
            description: body.description.clone(),
            files: body.files,
            digest: body.digest,
            taken: false,
        });
    }
    if skills.is_empty() {
        return Err(StoreRefusal::new(
            StoreRefusalCause::NoSkills,
            "no skill was found there",
        ));
    }
    Ok(skills)
}

fn inspect(
    target: &KitTarget,
    staging: &Path,
    source: &SourceSpec,
) -> Result<Preview, StoreRefusal> {
    let id = fresh_id("p");
    match source {
        SourceSpec::Folder { path } => {
            let root =
                hide_platform::fs::identity::canonical(Path::new(path)).map_err(|error| {
                    StoreRefusal::new(
                        StoreRefusalCause::NotFound,
                        format!("{path} could not be read: {error}"),
                    )
                })?;
            if !root.is_dir() {
                return Err(StoreRefusal::new(
                    StoreRefusalCause::NotFound,
                    format!("{path} is not a folder"),
                ));
            }
            let fallback = root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "skill".to_owned());
            let found = skill::discover(&root, &fallback)
                .map_err(|reason| StoreRefusal::new(StoreRefusalCause::Failed, reason))?;
            Ok(Preview {
                id,
                source: SourceSpec::Folder {
                    path: root.display().to_string(),
                },
                url: None,
                commit: None,
                tag: None,
                reference: None,
                skills: found_skills(found, "")?,
                agents: Vec::new(),
                at_unix_ms: now_unix_ms(),
            })
        }
        SourceSpec::Git { url } => {
            let address = parse(url)?;
            let git = Git::find(target)?;
            let folder = staging.join(format!("{FETCHING}{id}"));
            hide_platform::fs::private::create_dir_all(&folder).map_err(|error| {
                StoreRefusal::new(
                    StoreRefusalCause::Failed,
                    format!("{} could not be created: {error}", folder.display()),
                )
            })?;
            let answer = (|| {
                let repo = folder.join("repo");
                let commit = clone(&git, &address, &repo, 1)?;
                let tag = Remote::ask(&git, &address.url)
                    .ok()
                    .and_then(|remote| remote.tag_of(&commit));
                let root = join_relative(&repo, &address.path);
                if !root.is_dir() {
                    return Err(StoreRefusal::new(
                        StoreRefusalCause::NoSkills,
                        format!("{} is not a folder of the repository", address.path),
                    ));
                }
                let found = skill::discover(&root, &address.name)
                    .map_err(|reason| StoreRefusal::new(StoreRefusalCause::Failed, reason))?;
                Ok(Preview {
                    id: id.clone(),
                    source: source.clone(),
                    url: Some(address.url.clone()),
                    commit: Some(commit),
                    tag,
                    reference: address.reference.clone(),
                    skills: found_skills(found, &address.path)?,
                    agents: Vec::new(),
                    at_unix_ms: now_unix_ms(),
                })
            })();
            if answer.is_err() {
                let _ = std::fs::remove_dir_all(&folder);
            }
            answer
        }
    }
}

// --- Updates ----------------------------------------------------------------------

/// Asks each git item's repository for the commit its reference names now
/// (D-33); an item whose repository did not answer keeps its last answer.
fn check(target: &KitTarget, record: &StoreRecord) -> Result<Vec<(String, Latest)>, StoreRefusal> {
    let items: Vec<&StoreItem> = record
        .items
        .iter()
        .filter(|item| matches!(item.source, Source::Git { .. }))
        .collect();
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let git = Git::find(target)?;
    let mut answers: BTreeMap<String, Result<Remote, StoreRefusal>> = BTreeMap::new();
    let mut latest = Vec::new();
    for item in items {
        let Source::Git { url, reference, .. } = &item.source else {
            continue;
        };
        let remote = answers
            .entry(url.clone())
            .or_insert_with(|| Remote::ask(&git, url));
        match remote {
            Ok(remote) => {
                if let Some(commit) = remote.commit(reference.as_deref()) {
                    let tag = remote.tag_of(&commit);
                    latest.push((item.id.clone(), Latest { commit, tag }));
                }
            }
            Err(refusal) => super::log("store.check_failed", Some(&item.id), &refusal.reason),
        }
    }
    Ok(latest)
}

/// Fetches the item's repository as it is now and says what an update
/// would change (B23).
fn preview_update(
    target: &KitTarget,
    dir: &Path,
    staging: &Path,
    item: &StoreItem,
) -> Result<UpdateDiff, StoreRefusal> {
    let Source::Git {
        url,
        path,
        reference,
        commit: current,
        tag,
    } = &item.source
    else {
        return Err(StoreRefusal::new(
            StoreRefusalCause::Unsupported,
            format!("{} has no repository to update from", item.name),
        ));
    };
    if item.kind != ItemKind::Skill {
        return Err(StoreRefusal::new(
            StoreRefusalCause::Unsupported,
            "only a skill updates from a repository",
        ));
    }
    let git = Git::find(target)?;
    let id = fresh_id("u");
    let folder = staging.join(format!("{FETCHING}{id}"));
    hide_platform::fs::private::create_dir_all(&folder).map_err(|error| {
        StoreRefusal::new(
            StoreRefusalCause::Failed,
            format!("{} could not be created: {error}", folder.display()),
        )
    })?;
    let answer = (|| {
        let repo = folder.join("repo");
        let address = Address {
            url: url.clone(),
            reference: reference.clone(),
            path: path.clone(),
            name: item.name.clone(),
        };
        let commit = clone(&git, &address, &repo, 50)?;
        let new_tag = Remote::ask(&git, url)
            .ok()
            .and_then(|remote| remote.tag_of(&commit));
        let skill_folder = join_relative(&repo, path);
        if !skill_folder.join("SKILL.md").is_file() {
            return Err(StoreRefusal::new(
                StoreRefusalCause::NoSkills,
                format!("{} no longer holds the skill at {path}", url),
            ));
        }
        let staged = folder.join("skill");
        skill::take_in(&skill_folder, &staged)?;
        let repo_text = repo.display().to_string();
        let reachable = git
            .run(
                &[
                    "-C",
                    repo_text.as_str(),
                    "cat-file",
                    "-e",
                    &format!("{current}^{{commit}}"),
                ],
                ASK_DEADLINE,
            )
            .is_ok();
        let range = format!("{current}..HEAD");
        let limit = SUMMARY_LIMIT.to_string();
        let mut args = vec![
            "-C",
            repo_text.as_str(),
            "log",
            "--format=%s",
            "-n",
            limit.as_str(),
        ];
        if reachable {
            args.push(range.as_str());
        }
        args.push("--");
        if !path.is_empty() {
            args.push(path.as_str());
        }
        let summary = git
            .run(&args, ASK_DEADLINE)
            .map(|log| log.lines().map(str::to_owned).collect())
            .unwrap_or_default();
        let old = dir.join("items").join(folder_name(&item.id));
        let (files, new_executables) = compare(&old, &staged)?;
        let _ = std::fs::remove_dir_all(&repo);
        Ok(UpdateDiff {
            id: id.clone(),
            item: item.id.clone(),
            from: Version::of_commit(current, tag.as_deref()),
            to: Version::of_commit(&commit, new_tag.as_deref()),
            commit,
            files,
            new_executables,
            summary,
            at_unix_ms: now_unix_ms(),
        })
    })();
    if answer.is_err() {
        let _ = std::fs::remove_dir_all(&folder);
    }
    answer
}

/// What differs between two skill folders, file by file, with lines added
/// and removed counted as a multiset of lines (cheap, and what a row needs),
/// and the files that run in `new` and did not in `old`.
fn compare(old: &Path, new: &Path) -> Result<(Vec<FileChange>, Vec<String>), StoreRefusal> {
    let before = skill::body_of(old).ok();
    let after = skill::body_of(new)?;
    let before_files = before.map(|body| body.files).unwrap_or_default();
    let mut changes = Vec::new();
    let mut executables = Vec::new();
    for file in &after.files {
        let was = before_files.iter().find(|old| old.path == file.path);
        if file.executable && !was.is_some_and(|was| was.executable) {
            executables.push(file.path.clone());
        }
        let new_text = std::fs::read(join_relative(new, &file.path)).unwrap_or_default();
        match was {
            None => changes.push(FileChange {
                path: file.path.clone(),
                change: FileChangeKind::Added,
                added: lines(&new_text),
                removed: 0,
            }),
            Some(_) => {
                let old_text = std::fs::read(join_relative(old, &file.path)).unwrap_or_default();
                if old_text != new_text {
                    let (added, removed) = line_counts(&old_text, &new_text);
                    changes.push(FileChange {
                        path: file.path.clone(),
                        change: FileChangeKind::Modified,
                        added,
                        removed,
                    });
                }
            }
        }
    }
    for file in &before_files {
        if !after.files.iter().any(|new| new.path == file.path) {
            let old_text = std::fs::read(join_relative(old, &file.path)).unwrap_or_default();
            changes.push(FileChange {
                path: file.path.clone(),
                change: FileChangeKind::Removed,
                added: 0,
                removed: lines(&old_text),
            });
        }
    }
    changes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((changes, executables))
}

fn lines(bytes: &[u8]) -> u32 {
    String::from_utf8_lossy(bytes).lines().count() as u32
}

fn line_counts(old: &[u8], new: &[u8]) -> (u32, u32) {
    let mut counts: BTreeMap<String, i64> = BTreeMap::new();
    for line in String::from_utf8_lossy(old).lines() {
        *counts.entry(line.to_owned()).or_default() -= 1;
    }
    for line in String::from_utf8_lossy(new).lines() {
        *counts.entry(line.to_owned()).or_default() += 1;
    }
    let added = counts.values().filter(|count| **count > 0).sum::<i64>();
    let removed = -counts.values().filter(|count| **count < 0).sum::<i64>();
    (added as u32, removed as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> (String, Option<String>, String, String) {
        let address = parse(text).unwrap_or_else(|refusal| panic!("{text}: {refusal:?}"));
        (address.url, address.reference, address.path, address.name)
    }

    #[test]
    fn a_github_address_is_read_in_each_form_the_operator_pastes() {
        let plain = (
            "https://github.com/owner/archify.git".to_owned(),
            None,
            String::new(),
            "archify".to_owned(),
        );
        for text in [
            "https://github.com/owner/archify",
            "https://github.com/owner/archify.git",
            "https://github.com/owner/archify/",
            "github.com/owner/archify",
            "owner/archify",
        ] {
            assert_eq!(read(text), plain, "{text}");
        }
        assert_eq!(
            read("https://github.com/owner/kit/tree/main/skills/archify"),
            (
                "https://github.com/owner/kit.git".to_owned(),
                Some("main".to_owned()),
                "skills/archify".to_owned(),
                "kit".to_owned()
            )
        );
        assert_eq!(
            read("git@github.com:owner/kit.git").0,
            "git@github.com:owner/kit.git"
        );
        assert_eq!(
            read("https://gitlab.com/group/kit.git").0,
            "https://gitlab.com/group/kit.git"
        );
        for text in [
            "archify",
            "/Users/me/skills",
            "https://github.com/owner",
            "a b/c",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
    }

    #[test]
    fn git_complaints_read_as_no_access_or_an_unreachable_host() {
        assert_eq!(
            classify("remote: Repository not found.\nfatal: repository 'x' not found").cause,
            StoreRefusalCause::NoAccess
        );
        assert_eq!(
            classify(
                "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
            )
            .cause,
            StoreRefusalCause::NoAccess
        );
        assert_eq!(
            classify("fatal: unable to access 'https://github.com/x/': Could not resolve host: github.com")
                .cause,
            StoreRefusalCause::Unreachable
        );
    }
}
