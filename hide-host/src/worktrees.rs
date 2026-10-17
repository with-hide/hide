//! One repository's worktrees on the machine that holds it: what Git says
//! about each (the facts the projects tree, the Overview and the deletion
//! gate read), the check a new branch passes before anything is created, and
//! the confirmed removal of one linked worktree (PRD S5.5 B27-B29), forced
//! only as far as the operator accepted.
//!
//! The core calls these in process for this machine; a device's helper
//! answers the same functions (`Call::Worktrees`, `Call::BranchCheck`,
//! `Call::WorktreeRemovalCheck`, `Call::WorktreeRemove`), so a local and a device repository obey one
//! rule set. The policy built on the facts - the deletion gate, pull request
//! bases, disk and agent decoration - stays with the core.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

use hide_node_link::worktrees::{GIT_WATCH_LIMIT, WORKTREES_READ_TIME};

use hide_platform::process::OwnedChild;

use crate::error::{ErrorCode, HostError, HostResult};

/// The most one `git` invocation may take. A repository that cannot answer in
/// this time reports its status unavailable rather than holding every other
/// project's answer behind it; a status over evicted iCloud files ran for
/// minutes before this bound existed.
pub const GIT_DEADLINE: Duration = Duration::from_secs(15);

/// The most of a repository read one worktree's measurement may take, its
/// ignored-folder scan included, so one slow worktree leaves the others of
/// [`WORKTREES_READ_TIME`] to the rest.
pub const WORKTREE_MEASURE_TIME: Duration = Duration::from_secs(30);

/// Reads the repository that holds `path`: its main worktree, default
/// branch, local branches and every listed worktree. `bases` names a branch's
/// pull request base; `base_override` is the operator's chosen base. With
/// `reuse_unchanged`, a worktree whose inputs did not move since `measured`
/// last saw it keeps that measurement.
pub fn read(
    path: &Path,
    bases: &BTreeMap<String, String>,
    base_override: Option<&str>,
    measured: &Measured,
    reuse_unchanged: bool,
) -> Option<RepositoryWorktrees> {
    let until = Instant::now() + WORKTREES_READ_TIME;
    if !path.exists() {
        return Some(RepositoryWorktrees {
            root_path: path.to_string_lossy().into_owned(),
            unavailable_reason: Some(format!("Repository unavailable: {}", path.display())),
            ..RepositoryWorktrees::default()
        });
    }
    // A folder that is not a repository has no worktrees and is not a
    // failure; the card and the tree both present it as a plain folder.
    let root = main_worktree(path, until)?;
    let root_path = root.to_string_lossy().into_owned();
    Some(read_project_until(
        &root,
        root_path,
        bases,
        base_override,
        measured,
        reuse_unchanged,
        until,
    ))
}

/// Reads `root`, already known to be a main worktree, within
/// [`WORKTREES_READ_TIME`].
pub fn read_project(
    root: &Path,
    root_path: String,
    bases: &BTreeMap<String, String>,
    base_override: Option<&str>,
    measured: &Measured,
    reuse_unchanged: bool,
) -> RepositoryWorktrees {
    read_project_until(
        root,
        root_path,
        bases,
        base_override,
        measured,
        reuse_unchanged,
        Instant::now() + WORKTREES_READ_TIME,
    )
}

/// The read's discovery and metadata share its measurement's absolute end;
/// entering with a known main worktree does not renew that time.
fn read_project_until(
    root: &Path,
    root_path: String,
    bases: &BTreeMap<String, String>,
    base_override: Option<&str>,
    measured: &Measured,
    reuse_unchanged: bool,
    until: Instant,
) -> RepositoryWorktrees {
    let default_branch = default_branch(root, until);
    let refs = Refs::read(root, until);
    let branches = refs.as_ref().map(Refs::branches).unwrap_or_default();
    let valid_override = base_override.filter(|base| resolvable_base(root, base, until).is_some());
    let base_branch = valid_override
        .map(str::to_owned)
        .or_else(|| default_branch.clone());
    let base_branch_fallback = base_override
        .filter(|_| valid_override.is_none())
        .map(|base| {
            if default_branch.is_some() {
                format!("Base branch {base} is unavailable; using repository default")
            } else {
                format!("Base branch {base} is unavailable; repository base is unknown")
            }
        });
    let base_source = if valid_override.is_some() {
        "specified"
    } else if default_branch.is_some() {
        "origin_head"
    } else {
        "unknown"
    }
    .to_owned();
    let listed = match listed_worktrees(root, until) {
        Ok(listed) => listed,
        Err(reason) => {
            return RepositoryWorktrees {
                root_path,
                default_branch,
                unavailable_reason: Some(reason),
                ..RepositoryWorktrees::default()
            };
        }
    };
    let shared_git_path = git_until(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        until,
    )
    .ok()
    .map(|value| value.trim().to_owned());
    // Read from the repository's own configuration, not another git process.
    let origin_url = hide_project::git::discover(root)
        .and_then(|repository| repository.config().ok())
        .and_then(|config| config.origin_url().map(str::to_owned));
    // Every worktree shares the repository's last fetch.
    let last_fetch_at_unix_ms = shared_git_path
        .as_deref()
        .and_then(|shared| std::fs::metadata(Path::new(shared).join("FETCH_HEAD")).ok())
        .and_then(|metadata| metadata.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|elapsed| elapsed.as_millis() as u64);
    let bases = Bases {
        by_branch: bases,
        default_branch: base_branch.as_deref(),
        chosen: if base_override.is_some() {
            base_branch.as_deref()
        } else {
            None
        },
    };
    let mut worktrees = measure(
        measured,
        root,
        listed,
        &bases,
        refs.as_ref(),
        reuse_unchanged,
        (until, WORKTREE_MEASURE_TIME),
    );
    let paths: Vec<PathBuf> = worktrees.iter().map(|w| PathBuf::from(&w.path)).collect();
    for worktree in &mut worktrees {
        worktree.last_fetch_at_unix_ms = last_fetch_at_unix_ms;
        worktree.nested = paths
            .iter()
            .any(|p| p != Path::new(&worktree.path) && p.starts_with(&worktree.path));
    }
    // The main worktree leads so the project path and the first row agree.
    if let Some(index) = worktrees.iter().position(|worktree| worktree.is_main)
        && index != 0
    {
        let main = worktrees.remove(index);
        worktrees.insert(0, main);
    }
    RepositoryWorktrees {
        shared_git_path,
        origin_url,
        root_path,
        default_branch,
        branches,
        base_branch,
        base_source,
        base_branch_fallback,
        worktrees,
        unavailable_reason: None,
    }
}

/// The worktrees Git lists for the repository at `root`, its main worktree
/// first; a bare repository is no worktree.
fn listed_worktrees(root: &Path, until: Instant) -> Result<Vec<ListedWorktree>, String> {
    Ok(registered_until(root, until)?
        .into_iter()
        .enumerate()
        .map(|(index, row)| ListedWorktree {
            path: row.path.into(),
            branch: row.branch,
            head_sha: row
                .head
                .filter(|sha| sha.chars().any(|character| character != '0')),
            is_main: index == 0,
            bare: row.bare,
            lock_reason: row.lock_reason,
        })
        .filter(|listed| !listed.bare)
        .collect())
}

/// What a worktree's base is chosen from: its branch's pull request base,
/// the repository's base, and the base the operator chose for every branch.
struct Bases<'a> {
    by_branch: &'a BTreeMap<String, String>,
    default_branch: Option<&'a str>,
    chosen: Option<&'a str>,
}

impl Bases<'_> {
    fn of(&self, branch: Option<&str>) -> Option<String> {
        self.chosen
            .map(str::to_owned)
            .or_else(|| resolve_base(branch, self.by_branch, self.default_branch))
    }
}

/// Measures the listed worktrees, answering in their listed order. A
/// worktree whose inputs did not move keeps its last measurement when the
/// caller accepts that, one its own time cut short included; the others are
/// measured, the longest unmeasured first, while the read has time, so the
/// worktrees one read had no time for are the next read's first. A worktree
/// the read had no time for answers its last measurement only if its inputs
/// did not move, and is otherwise named unmeasured. `time` is when the read
/// ends and how long one worktree may take of it.
fn measure(
    measured: &Measured,
    root: &Path,
    listed: Vec<ListedWorktree>,
    bases: &Bases<'_>,
    refs: Option<&Refs>,
    reuse_unchanged: bool,
    (until, worktree_time): (Instant, Duration),
) -> Vec<WorktreeFacts> {
    let inputs: Vec<Option<Inputs>> = listed
        .iter()
        .map(|worktree| refs.map(|refs| Inputs::of(worktree, bases, refs)))
        .collect();
    let earlier = measured.earlier(root);
    let unchanged = |index: usize| {
        earlier
            .get(&listed[index].path)
            .filter(|(seen, _)| inputs[index].as_ref() == Some(seen))
            .map(|(_, facts)| facts.clone())
    };
    let mut answers: Vec<Option<WorktreeFacts>> = (0..listed.len())
        .map(|index| unchanged(index).filter(|_| reuse_unchanged))
        .collect();
    let mut due: Vec<usize> = (0..listed.len())
        .filter(|index| answers[*index].is_none())
        .collect();
    due.sort_by_key(|index| {
        earlier
            .get(&listed[*index].path)
            .and_then(|(_, facts)| facts.measured_at_unix_ms)
    });
    let mut kept = Kept::new();
    for index in due {
        let now = Instant::now();
        if now >= until {
            break;
        }
        let own = now + worktree_time;
        let facts = describe_within(listed[index].clone(), bases, own.min(until));
        // Cut short by the read's end rather than its own time, a worktree
        // is measured first by the next read, and until then answers its
        // last full measurement if nothing moved.
        if own > until && Instant::now() >= until {
            answers[index] = Some(unchanged(index).unwrap_or(facts));
            continue;
        }
        if let Some(inputs) = inputs[index].clone() {
            kept.insert(listed[index].path.clone(), (inputs, facts.clone()));
        }
        answers[index] = Some(facts);
    }
    let answers = answers
        .into_iter()
        .enumerate()
        .map(|(index, answer)| {
            answer
                .or_else(|| unchanged(index))
                .unwrap_or_else(|| unmeasured(&listed[index]))
        })
        .collect();
    for worktree in &listed {
        if !kept.contains_key(&worktree.path)
            && let Some(entry) = earlier.get(&worktree.path)
        {
            kept.insert(worktree.path.clone(), entry.clone());
        }
    }
    measured.keep(root, kept);
    answers
}

/// A worktree the read ran out of time for before it began.
fn unmeasured(listed: &ListedWorktree) -> WorktreeFacts {
    WorktreeFacts {
        path: listed.path.to_string_lossy().into_owned(),
        branch: listed.branch.clone(),
        is_main: listed.is_main,
        head_sha: listed.head_sha.clone(),
        lock_reason: listed.lock_reason.clone(),
        upstream_state: "unavailable".to_owned(),
        ignored_scan_unavailable: (!listed.is_main)
            .then(|| ignored_scan_failure("the repository read ran out of time before it")),
        unavailable_reason: Some(format!(
            "Worktree {} was not measured within the repository's {} s; refresh to measure it",
            worktree_name(listed),
            WORKTREES_READ_TIME.as_secs()
        )),
        ..WorktreeFacts::default()
    }
}

/// The name a worktree is shown by: its branch, else its folder.
fn worktree_name(listed: &ListedWorktree) -> String {
    listed.branch.clone().unwrap_or_else(|| {
        listed.path.file_name().map_or_else(
            || listed.path.display().to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
    })
}

/// What earlier reads measured of each repository's worktrees, kept by the
/// node that measured them, with the inputs each measurement was taken
/// from. It holds the repositories of the last [`GIT_WATCH_LIMIT`] reads,
/// the most the core watches; a repository it let go of is measured whole on
/// its next read, unless its file still holds it.
///
/// A memo with a folder ([`Measured::kept_in`]) also keeps each repository's
/// measurements there, one file a repository, written whole when they
/// change and read for a repository the memo has not held since it started,
/// so a restarted node measures again only the worktrees whose inputs moved.
/// The folder keeps the files of the last [`GIT_WATCH_LIMIT`] repositories
/// written.
#[derive(Debug, Default)]
pub struct Measured {
    repositories: Mutex<BTreeMap<PathBuf, (Instant, Kept)>>,
    folder: Option<PathBuf>,
}

type Kept = BTreeMap<PathBuf, (Inputs, WorktreeFacts)>;

/// The shape of a repository's file; a file of another version, or one that
/// does not read, is taken as nothing kept.
const KEPT_FILE_VERSION: u32 = 1;
/// The largest repository file read back: some thousands of worktrees' facts.
const KEPT_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// One repository's file: the root it was written for, and each kept
/// worktree with the inputs its facts were measured from.
#[derive(serde::Serialize, serde::Deserialize)]
struct KeptFile {
    version: u32,
    root: PathBuf,
    worktrees: Vec<(PathBuf, Inputs, WorktreeFacts)>,
}

impl Measured {
    /// A memo that also keeps its measurements in `folder`
    /// (`hide_kit::layout::worktree_facts`).
    pub fn kept_in(folder: PathBuf) -> Self {
        Self {
            repositories: Mutex::default(),
            folder: Some(folder),
        }
    }

    fn earlier(&self, root: &Path) -> Kept {
        if let Some((_, kept)) = self
            .repositories
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(root)
        {
            return kept.clone();
        }
        self.folder
            .as_deref()
            .map(|folder| read_kept(folder, root))
            .unwrap_or_default()
    }

    fn keep(&self, root: &Path, kept: Kept) {
        let mut repositories = self
            .repositories
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let changed = repositories
            .get(root)
            .is_none_or(|(_, earlier)| *earlier != kept);
        let written = (changed && self.folder.is_some()).then(|| kept.clone());
        repositories.insert(root.to_path_buf(), (Instant::now(), kept));
        while repositories.len() > GIT_WATCH_LIMIT {
            let Some(oldest) = repositories
                .iter()
                .min_by_key(|(_, (at, _))| *at)
                .map(|(root, _)| root.clone())
            else {
                break;
            };
            repositories.remove(&oldest);
        }
        drop(repositories);
        if let (Some(folder), Some(kept)) = (self.folder.as_deref(), written) {
            write_kept(folder, root, kept);
        }
    }
}

/// The file in `folder` that keeps `root`'s measurements.
fn kept_file(folder: &Path, root: &Path) -> PathBuf {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(root.as_os_str().as_encoded_bytes());
    let name: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    folder.join(format!("{name}.json"))
}

/// One record of the memo's file for `root` on the node's error stream.
fn kept_record(kind: &str, root: &Path, reason: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "worktrees",
            "kind": kind,
            "root": root,
            "reason": reason,
        })
    );
}

/// What `root`'s file keeps: nothing when there is none, and nothing with
/// one record when it cannot be used.
fn read_kept(folder: &Path, root: &Path) -> Kept {
    use std::io::Read as _;
    let path = kept_file(folder, root);
    let file = match hide_platform::fs::open_regular(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Kept::new(),
        Err(error) => {
            kept_record("worktrees.facts_unread", root, &error.to_string());
            return Kept::new();
        }
    };
    let mut bytes = Vec::new();
    if let Err(error) = file.take(KEPT_FILE_BYTES + 1).read_to_end(&mut bytes) {
        kept_record("worktrees.facts_unread", root, &error.to_string());
        return Kept::new();
    }
    if bytes.len() as u64 > KEPT_FILE_BYTES {
        kept_record(
            "worktrees.facts_unread",
            root,
            "the file is past its size bound",
        );
        return Kept::new();
    }
    #[derive(serde::Deserialize)]
    struct Version {
        version: u32,
    }
    match serde_json::from_slice::<Version>(&bytes) {
        Ok(Version { version }) if version == KEPT_FILE_VERSION => {}
        Ok(Version { version }) => {
            kept_record(
                "worktrees.facts_unread",
                root,
                &format!("version {version}, not {KEPT_FILE_VERSION}"),
            );
            return Kept::new();
        }
        Err(error) => {
            kept_record("worktrees.facts_unread", root, &error.to_string());
            return Kept::new();
        }
    }
    match serde_json::from_slice::<KeptFile>(&bytes) {
        Ok(file) if file.root == root => file
            .worktrees
            .into_iter()
            .map(|(path, inputs, facts)| (path, (inputs, facts)))
            .collect(),
        Ok(_) => {
            kept_record(
                "worktrees.facts_unread",
                root,
                "the file names another root",
            );
            Kept::new()
        }
        Err(error) => {
            kept_record("worktrees.facts_unread", root, &error.to_string());
            Kept::new()
        }
    }
}

/// Replaces `root`'s file with `kept`, then lets go of the files of the
/// repositories written longest ago past [`GIT_WATCH_LIMIT`].
fn write_kept(folder: &Path, root: &Path, kept: Kept) {
    let file = KeptFile {
        version: KEPT_FILE_VERSION,
        root: root.to_path_buf(),
        worktrees: kept
            .into_iter()
            .map(|(path, (inputs, facts))| (path, inputs, facts))
            .collect(),
    };
    let written = serde_json::to_vec(&file)
        .map_err(|error| error.to_string())
        .and_then(|bytes| {
            hide_platform::fs::private::create_dir_all(folder)
                .and_then(|()| {
                    hide_platform::fs::atomic::write_file(
                        &kept_file(folder, root),
                        &bytes,
                        hide_platform::fs::Access::Private,
                    )
                })
                .map_err(|error| error.to_string())
        });
    if let Err(reason) = written {
        kept_record("worktrees.facts_unsaved", root, &reason);
        return;
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.len() == 37 && name.ends_with(".json"))
        })
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    if files.len() <= GIT_WATCH_LIMIT {
        return;
    }
    files.sort();
    let written = kept_file(folder, root);
    for (_, path) in files
        .iter()
        .filter(|(_, path)| *path != written)
        .take(files.len() - GIT_WATCH_LIMIT)
    {
        let _ = std::fs::remove_file(path);
    }
}

/// What one worktree's facts are measured from, short of its working tree's
/// contents: the listed record (HEAD, branch and lock), the worktree's
/// index, its base, its branch's upstream, and the commits the base, the
/// base's remote-tracking copy and the upstream name. An edit that moves
/// none of these is read when the caller asks for every worktree again.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
struct Inputs {
    listed: ListedWorktree,
    index: Option<(u64, SystemTime)>,
    base: Option<String>,
    upstream: Option<String>,
    commits: [Option<String>; 3],
}

impl Inputs {
    fn of(listed: &ListedWorktree, bases: &Bases<'_>, refs: &Refs) -> Self {
        let index = hide_project::git::discover(&listed.path)
            .and_then(|repository| std::fs::metadata(repository.git_dir.join("index")).ok())
            .and_then(|metadata| Some((metadata.len(), metadata.modified().ok()?)));
        let base = bases.of(listed.branch.as_deref());
        let upstream = listed
            .branch
            .as_deref()
            .and_then(|branch| refs.upstream(&format!("refs/heads/{branch}")));
        let commits = [
            base.as_deref()
                .and_then(|base| refs.commit(&format!("refs/heads/{base}"))),
            base.as_deref()
                .and_then(|base| refs.commit(&format!("refs/remotes/origin/{base}"))),
            upstream
                .as_deref()
                .and_then(|upstream| refs.commit(upstream)),
        ];
        Self {
            listed: listed.clone(),
            index,
            base,
            upstream,
            commits,
        }
    }
}

/// The repository's branches and remote-tracking refs, each with its short
/// name, the commit it names and its upstream, from one `git for-each-ref`.
struct Refs(BTreeMap<String, (String, String, String)>);

impl Refs {
    fn read(root: &Path, until: Instant) -> Option<Self> {
        let output = git_until(
            root,
            &[
                "for-each-ref",
                "--format=%(refname)%00%(refname:short)%00%(objectname)%00%(upstream)",
                "refs/heads",
                "refs/remotes",
            ],
            until,
        )
        .ok()?;
        Some(Self(
            output
                .lines()
                .filter_map(|line| {
                    let mut fields = line.split('\0');
                    let name = fields.next()?.to_owned();
                    let short = fields.next()?.to_owned();
                    let commit = fields.next()?.to_owned();
                    let upstream = fields.next()?.to_owned();
                    Some((name, (short, commit, upstream)))
                })
                .collect(),
        ))
    }

    /// The local branches by their short names, sorted.
    fn branches(&self) -> Vec<String> {
        let mut branches: Vec<String> = self
            .0
            .iter()
            .filter(|(name, _)| name.starts_with("refs/heads/"))
            .map(|(_, (short, _, _))| short.trim().to_owned())
            .filter(|short| !short.is_empty())
            .collect();
        branches.sort();
        branches.dedup();
        branches
    }

    fn commit(&self, name: &str) -> Option<String> {
        self.0.get(name).map(|(_, commit, _)| commit.clone())
    }

    fn upstream(&self, name: &str) -> Option<String> {
        self.0
            .get(name)
            .map(|(_, _, upstream)| upstream.clone())
            .filter(|upstream| !upstream.is_empty())
    }
}

/// One record of `git worktree list --porcelain`, before any counting.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ListedWorktree {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub is_main: bool,
    pub bare: bool,
    pub head_sha: Option<String>,
    pub lock_reason: Option<String>,
}

/// Splits porcelain worktree records. Records are separated by a blank line
/// and each begins with `worktree <path>`; `branch refs/heads/<name>` names
/// the checked-out branch, and a detached head has no such line.
///
/// The first record is the main worktree, which is git's documented order and
/// what tells a linked worktree apart without a second `rev-parse`.
pub fn parse_worktree_list(output: &str) -> Vec<ListedWorktree> {
    output
        .split("\n\n")
        .filter_map(|record| {
            let path = record
                .lines()
                .find_map(|line| line.strip_prefix("worktree "))?;
            Some(ListedWorktree {
                path: PathBuf::from(path),
                branch: record
                    .lines()
                    .find_map(|line| line.strip_prefix("branch refs/heads/"))
                    .map(str::to_owned),
                head_sha: record
                    .lines()
                    .find_map(|line| line.strip_prefix("HEAD "))
                    .filter(|sha| sha.chars().any(|character| character != '0'))
                    .map(str::to_owned),
                bare: record.lines().any(|line| line == "bare"),
                is_main: false,
                lock_reason: record.lines().find_map(lock_reason),
            })
        })
        .enumerate()
        .map(|(index, mut row)| {
            row.is_main = index == 0;
            row
        })
        .collect()
}

/// The facts of one listed worktree, measured until `until`: no `git`
/// starts after it and one still running then is stopped, the ignored-folder
/// scan included. A worktree measured only in part names itself in its
/// unavailable reason, and what was not measured stays unknown.
fn describe_within(listed: ListedWorktree, bases: &Bases<'_>, until: Instant) -> WorktreeFacts {
    let path = listed.path.to_string_lossy().into_owned();
    if !listed.path.exists() {
        // Nothing can be counted against a path that is not there, and
        // reporting zeros would read as a clean checkout rather than a gone
        // one. The row shows `missing` and stops.
        return WorktreeFacts {
            path,
            branch: listed.branch,
            missing: true,
            is_main: listed.is_main,
            head_sha: listed.head_sha,
            lock_reason: listed.lock_reason,
            ..WorktreeFacts::default()
        };
    }

    let mut unavailable_reason = None;
    let (dirty, changed_file_count) = record_failure(
        working_tree_state(&listed.path, until),
        &mut unavailable_reason,
        (false, 0),
    );
    let base_branch = bases.of(listed.branch.as_deref());
    let (ahead, behind) = base_branch
        .as_deref()
        .map(|base| ahead_behind(&listed.path, base, until))
        .map(|result| record_failure(result, &mut unavailable_reason, (0, 0)))
        .unwrap_or((0, 0));
    let (added_lines, removed_lines) = base_branch
        .as_deref()
        .map(|base| committed_line_delta(&listed.path, base, until))
        .map(|result| record_failure(result, &mut unavailable_reason, (0, 0)))
        .unwrap_or((0, 0));

    let merged = base_branch
        .as_deref()
        .and_then(|base| merged_into(&listed.path, base, until));
    let (upstream_state, unpushed, behind_upstream) = record_failure(
        upstream(&listed.path, listed.branch.as_deref(), until),
        &mut unavailable_reason,
        ("unavailable".into(), None, None),
    );
    // A linked worktree is as old as its `.git/worktrees/<name>` entry, which
    // git writes once at `worktree add` and never touches again. It is read
    // off the gitfile, not asked of git, so the pass forks nothing extra for
    // it; the main worktree has no such entry and carries no time.
    let created_at_unix_ms = if listed.is_main {
        None
    } else {
        hide_project::git::discover(&listed.path)
            .and_then(|repository| std::fs::metadata(repository.git_dir).ok())
            .and_then(|metadata| metadata.created().ok())
            .and_then(|created| created.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_millis() as u64)
    };
    let last_commit = git_until(&listed.path, &["log", "-1", "--format=%ct%x00%s"], until).ok();
    let commit_fields = last_commit
        .as_deref()
        .and_then(|value| value.trim_end().split_once('\0'));
    let last_commit_unix_seconds = commit_fields.and_then(|(time, _)| time.parse().ok());
    let last_commit_subject = commit_fields.map(|(_, subject)| subject.to_owned());
    let (ignored_repositories, ignored_scan_unavailable) = if listed.is_main {
        (Vec::new(), None)
    } else {
        let left = until.saturating_duration_since(Instant::now());
        match scan_ignored_repositories(
            &listed.path,
            &mut WalkBudget::allowing(IGNORED_WALK_ENTRIES, left.min(IGNORED_WALK_TIME)),
        ) {
            Ok(names) => (names, None),
            Err(reason) => (Vec::new(), Some(reason)),
        }
    };
    if Instant::now() >= until {
        unavailable_reason = Some(format!(
            "Worktree {} was not fully measured in the time it had; refresh to measure it again",
            worktree_name(&listed)
        ));
    }
    WorktreeFacts {
        path,
        branch: listed.branch,
        missing: false,
        is_main: listed.is_main,
        lock_reason: listed.lock_reason,
        ignored_repositories,
        ignored_scan_unavailable,
        nested: false,
        dirty,
        changed_file_count,
        base_branch,
        ahead,
        behind,
        added_lines,
        removed_lines,
        unpushed,
        upstream_state,
        behind_upstream,
        created_at_unix_ms,
        merged,
        head_sha: listed.head_sha,
        last_commit_unix_seconds,
        last_commit_subject,
        last_fetch_at_unix_ms: None,
        measured_at_unix_ms: Some(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        ),
        unavailable_reason,
    }
}

fn record_failure<T>(
    result: Result<T, String>,
    unavailable_reason: &mut Option<String>,
    default: T,
) -> T {
    match result {
        Ok(value) => value,
        Err(reason) => {
            *unavailable_reason = Some(reason);
            default
        }
    }
}

/// What a branch is measured against: its pull request's base when `gh` has
/// named one, the repository default branch otherwise.
///
/// The branch a base names is not compared with itself - the default branch's
/// own worktree shows no base row - and a branch with neither a pull request
/// nor a known default gets no comparison rather than a misleading zero.
pub fn resolve_base(
    branch: Option<&str>,
    bases: &BTreeMap<String, String>,
    default_branch: Option<&str>,
) -> Option<String> {
    branch
        .and_then(|branch| bases.get(branch).cloned())
        .or_else(|| default_branch.map(str::to_owned))
        .filter(|base| branch != Some(base.as_str()))
}

/// Whether anything is uncommitted, and how many files that is.
fn working_tree_state(path: &Path, until: Instant) -> Result<(bool, u32), String> {
    let output = git_until(
        path,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--untracked-files=all",
        ],
        until,
    )?;
    let count = output
        .split('\0')
        .filter(|record| record.len() > 3)
        .count()
        .try_into()
        .unwrap_or(u32::MAX);
    Ok((count > 0, count))
}

/// Commits on this branch since the base, and commits on the base this branch
/// does not have. `base...HEAD` is the merge-base comparison the card's
/// `↑A ↓B` states, not a raw two-dot range.
fn ahead_behind(path: &Path, base: &str, until: Instant) -> Result<(u32, u32), String> {
    let base_ref = resolvable_base(path, base, until)
        .ok_or_else(|| format!("Base branch {base} is unavailable"))?;
    let output = git_until(
        path,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base_ref}...HEAD"),
        ],
        until,
    )?;
    Ok(parse_ahead_behind(&output))
}

/// `--left-right --count` prints `<behind>\t<ahead>`: the left side is the
/// base, so its count is what this branch is missing.
pub fn parse_ahead_behind(output: &str) -> (u32, u32) {
    let mut fields = output.split_whitespace();
    let behind = fields
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let ahead = fields
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    (ahead, behind)
}

fn committed_line_delta(path: &Path, base: &str, until: Instant) -> Result<(u32, u32), String> {
    let base_ref = resolvable_base(path, base, until)
        .ok_or_else(|| format!("Base branch {base} is unavailable"))?;
    let output = git_until(
        path,
        &["diff", "--shortstat", &format!("{base_ref}...HEAD")],
        until,
    )?;
    Ok(parse_shortstat(&output))
}

/// `git diff --shortstat` prints, for example,
/// ` 3 files changed, 42 insertions(+), 7 deletions(-)`. Either half may be
/// missing when a change is all additions or all removals.
pub fn parse_shortstat(output: &str) -> (u32, u32) {
    let mut added = 0;
    let mut removed = 0;
    for part in output.split(',') {
        let part = part.trim();
        let Some(count) = part
            .split_whitespace()
            .next()
            .and_then(|value| value.parse::<u32>().ok())
        else {
            continue;
        };
        if part.contains("insertion") {
            added = count;
        } else if part.contains("deletion") {
            removed = count;
        }
    }
    (added, removed)
}

/// The base branch as a ref this worktree can actually resolve.
///
/// A base is a branch name from a pull request or a repository default, so it
/// may exist only as a remote-tracking ref in this clone. The local branch is
/// preferred, the remote-tracking ref is the fallback, and a base that
/// resolves as neither yields no comparison rather than a silent zero.
fn resolvable_base(path: &Path, base: &str, until: Instant) -> Option<String> {
    base_candidates(base).find(|candidate| resolves(path, candidate, until))
}

/// The refs that name `base`: the local branch, then its remote-tracking copy
/// as of the last fetch.
fn base_candidates(base: &str) -> impl Iterator<Item = String> {
    [base.to_owned(), format!("origin/{base}")].into_iter()
}

fn resolves(path: &Path, candidate: &str, until: Instant) -> bool {
    git_until(
        path,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{candidate}^{{commit}}"),
        ],
        until,
    )
    .is_ok()
}

/// Whether HEAD is already in `base`, in the local branch or in its
/// remote-tracking copy, whichever has it; a local base that stopped following
/// its remote is one of two answers, never the only one. `None` when no ref names the base or
/// Git could not answer, which is never read as "not merged".
fn merged_into(path: &Path, base: &str, until: Instant) -> Option<bool> {
    let mut answered = false;
    for candidate in base_candidates(base).filter(|candidate| resolves(path, candidate, until)) {
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        let output = output_within(
            hide_platform::programs::command("git")
                .arg("-C")
                .arg(path)
                .args(["merge-base", "--is-ancestor", "HEAD", &candidate]),
            left.min(GIT_DEADLINE),
        )
        .ok()??;
        match output.status.code() {
            Some(0) => return Some(true),
            Some(1) => answered = true,
            _ => return None,
        }
    }
    answered.then_some(false)
}

/// Commits this branch has that its upstream does not, with the remote's
/// name, and the commits the upstream has that this branch does not.
///
/// A branch with no upstream returns `None` for both: it has nowhere to push
/// and nothing to be behind, which the card states by omitting the row rather
/// than by showing a zero that would read as "fully pushed" or "up to date".
/// One `rev-list --left-right --count @{u}...HEAD` answers both directions,
/// the same shape `ahead_behind` already reads against the base, so the
/// second number costs no second process.
type UpstreamFacts = (String, Option<Unpushed>, Option<u32>);

fn upstream(path: &Path, branch: Option<&str>, until: Instant) -> Result<UpstreamFacts, String> {
    let Some(branch) = branch else {
        return Ok(("no_upstream".into(), None, None));
    };
    let configured = git_until(
        path,
        &[
            "for-each-ref",
            "--format=%(upstream)",
            &format!("refs/heads/{branch}"),
        ],
        until,
    )?;
    if configured.trim().is_empty() {
        return Ok(("no_upstream".into(), None, None));
    }
    if let Err(reason) = git_until(path, &["rev-parse", "--verify", "@{u}"], until) {
        // A read out of time knows nothing of the upstream.
        if Instant::now() >= until {
            return Err(reason);
        }
        return Ok(("gone".into(), None, None));
    }
    let counts = git_until(
        path,
        &["rev-list", "--left-right", "--count", "@{u}...HEAD"],
        until,
    )
    .ok()
    .and_then(|output| parse_left_right(&output));
    let Some((behind, count)) = counts else {
        return Err("git rev-list could not determine upstream commit count".into());
    };
    let remote = configured
        .trim()
        .trim_start_matches("refs/remotes/")
        .split('/')
        .next()
        .unwrap_or("")
        .to_owned();
    Ok((
        if count == 0 { "pushed" } else { "unpushed" }.into(),
        Some(Unpushed { remote, count }),
        Some(behind),
    ))
}

/// `--left-right --count` prints `<left>\t<right>`; both fields have to be
/// there and numeric, or the answer is no answer rather than a pair of zeros.
fn parse_left_right(output: &str) -> Option<(u32, u32)> {
    let mut fields = output.split_whitespace();
    let left = fields.next()?.parse().ok()?;
    let right = fields.next()?.parse().ok()?;
    Some((left, right))
}

/// The repository's default branch is only what `origin/HEAD` names.
///
/// The main worktree's current branch is deliberately excluded: using it as
/// a base would make a feature branch look like project policy and hide the
/// very mismatch the migration action exists to explain.
fn default_branch(root: &Path, until: Instant) -> Option<String> {
    if let Ok(output) = git_until(
        root,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        until,
    ) {
        let branch = output.trim().trim_start_matches("origin/").to_owned();
        if !branch.is_empty() {
            return Some(branch);
        }
    }
    None
}

/// The repository's main working tree, which is what identifies a project.
fn main_worktree(path: &Path, until: Instant) -> Option<PathBuf> {
    let common = git_until(
        path,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        until,
    )
    .ok()?
    .trim()
    .to_owned();
    if common.is_empty() {
        return None;
    }
    PathBuf::from(common).parent().map(Path::to_path_buf)
}

/// Every `git` this module runs, by directory and subcommand, from the first
/// [`record_git_calls`] on, so the core's tests can assert that a quiet
/// repository is not read again. Nothing in the product turns it on, so a
/// running daemon records nothing. It was a cargo feature the core's tests
/// enabled, which built this crate and every crate above it twice.
static GIT_CALLS: std::sync::Mutex<Option<Vec<(PathBuf, String)>>> = std::sync::Mutex::new(None);

/// Records every `git` this module runs, for the rest of the process.
pub fn record_git_calls() {
    GIT_CALLS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get_or_insert_with(Vec::new);
}

/// How many `git <command>` ran in `cwd` since [`record_git_calls`].
///
/// # Panics
///
/// When nothing called [`record_git_calls`]: a count of zero would then
/// claim a repository was not read when nothing was looking.
pub fn git_calls(cwd: &Path, command: &str) -> usize {
    GIT_CALLS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .expect("record_git_calls() before counting git calls")
        .iter()
        .filter(|(path, name)| path == cwd && name == command)
        .count()
}

/// Runs `git` in `cwd` within [`GIT_DEADLINE`], answering its stdout or a
/// one-line reason.
pub fn git(cwd: &Path, arguments: &[&str]) -> Result<String, String> {
    git_within(cwd, arguments, GIT_DEADLINE)
}

/// [`git`] that also ends by `until`, and does not start once it has passed.
fn git_until(cwd: &Path, arguments: &[&str], until: Instant) -> Result<String, String> {
    let left = until.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(format!(
            "git {} was not run: the measurement's time was spent",
            arguments[0]
        ));
    }
    git_within(cwd, arguments, left.min(GIT_DEADLINE))
}

/// [`git`] with its own bound, for the one command that may legitimately
/// outlast a read: a removal that deletes the folder in place.
fn git_within(cwd: &Path, arguments: &[&str], deadline: Duration) -> Result<String, String> {
    if let Some(calls) = GIT_CALLS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_mut()
    {
        calls.push((cwd.to_owned(), arguments.first().unwrap_or(&"").to_string()));
    }
    let output = output_within(
        hide_platform::programs::command("git")
            .arg("--no-optional-locks")
            .arg("-C")
            .arg(cwd)
            .args(arguments),
        deadline,
    )
    .map_err(|error| format!("git could not be run: {error}"))?;
    let Some(output) = output else {
        return Err(format!(
            "git {} did not finish within {} s",
            arguments[0],
            deadline.as_secs()
        ));
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if stderr.is_empty() {
            format!("git {} exited with {}", arguments[0], output.status)
        } else {
            format!("git {}: {stderr}", arguments[0])
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// The first ignored folder of the worktree that holds a Git repository of
/// its own, relative to the worktree.
pub fn ignored_repository(worktree: &Path) -> Result<Option<String>, String> {
    let mut budget = WalkBudget::worktree();
    let listed = ignored_folders(worktree, &budget)?;
    for folder in listed.split('\0').filter(|entry| entry.ends_with('/')) {
        if holds_repository(&worktree.join(folder), &mut budget)? {
            return Ok(Some(folder.trim_end_matches('/').to_owned()));
        }
    }
    Ok(None)
}

fn ignored_folders(worktree: &Path, budget: &WalkBudget) -> Result<String, String> {
    git_until(
        worktree,
        &[
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--directory",
            "-z",
        ],
        budget.until,
    )
}

/// Every repository boundary in ignored folders, measured off the runtime
/// lock. Git metadata is not traversed, and directory links are not followed.
pub fn ignored_repositories(worktree: &Path) -> Result<Vec<String>, String> {
    scan_ignored_repositories(worktree, &mut WalkBudget::worktree())
}

const IGNORED_REPOSITORY_CAP: usize = 1024;

fn scan_ignored_repositories(
    worktree: &Path,
    budget: &mut WalkBudget,
) -> Result<Vec<String>, String> {
    if Instant::now() >= budget.until {
        return Err(ignored_scan_failure(
            "the ignored-folder scan deadline was reached",
        ));
    }
    let listed = ignored_folders(worktree, budget)
        .map_err(|_| ignored_scan_failure("Git could not list its ignored folders"))?;
    let mut names = std::collections::BTreeSet::new();
    for folder in listed.split('\0').filter(|entry| entry.ends_with('/')) {
        let relative = Path::new(folder);
        if relative.is_absolute()
            || relative
                .components()
                .any(|part| matches!(part, std::path::Component::ParentDir))
        {
            return Err(ignored_scan_failure(
                "Git returned an invalid ignored folder",
            ));
        }
        let mut pending = vec![worktree.join(relative)];
        while let Some(directory) = pending.pop() {
            scan_entry(budget)?;
            let name = hide_platform::path::relative(worktree, &directory)
                .map_err(|_| ignored_scan_failure("an ignored folder name cannot be displayed"))?
                .into_string();
            let metadata = std::fs::symlink_metadata(&directory).map_err(|_| {
                ignored_scan_failure(&format!("ignored folder {name} could not be read"))
            })?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let marker = match std::fs::symlink_metadata(directory.join(".git")) {
                Ok(_) => Some(".git"),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                Err(_) => {
                    return Err(ignored_scan_failure(&format!(
                        "repository marker in {name} could not be read"
                    )));
                }
            };
            let bare =
                marker.is_none()
                    && directory.join("HEAD").try_exists().map_err(|_| {
                        ignored_scan_failure("a repository marker could not be read")
                    })?
                    && directory.join("objects").try_exists().map_err(|_| {
                        ignored_scan_failure("a repository marker could not be read")
                    })?
                    && directory.join("refs").try_exists().map_err(|_| {
                        ignored_scan_failure("a repository marker could not be read")
                    })?;
            if let Some(marker) = marker.or(bare.then_some(".")) {
                let remaining = budget
                    .until
                    .saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    return Err(ignored_scan_failure(
                        "the ignored-folder scan deadline was reached",
                    ));
                }
                git_within(
                    &directory,
                    &["rev-parse", "--resolve-git-dir", marker],
                    remaining.min(GIT_DEADLINE),
                )
                .map_err(|_| {
                    ignored_scan_failure(&format!("Git could not verify repository {name}"))
                })?;
                names.insert(name);
                if names.len() > IGNORED_REPOSITORY_CAP {
                    return Err(ignored_scan_failure(
                        "too many ignored repositories to review",
                    ));
                }
                // Bare repositories have no checkout below their metadata.
                if bare {
                    continue;
                }
            }
            let entries = std::fs::read_dir(&directory)
                .map_err(|_| ignored_scan_failure("an ignored folder could not be read"))?;
            for entry in entries {
                scan_entry(budget)?;
                let entry = entry.map_err(|_| {
                    ignored_scan_failure("an ignored folder entry could not be read")
                })?;
                if entry.file_name() == ".git" {
                    continue;
                }
                let kind = entry.file_type().map_err(|_| {
                    ignored_scan_failure("an ignored folder entry could not be read")
                })?;
                if kind.is_dir() {
                    pending.push(entry.path());
                }
            }
        }
    }
    Ok(names.into_iter().collect())
}

fn ignored_scan_failure(reason: &str) -> String {
    format!(
        "Ignored repository scan unavailable: {reason}. Check folder access and repository metadata, then refresh before deleting."
    )
}

fn scan_entry(budget: &mut WalkBudget) -> Result<(), String> {
    budget.entries = budget
        .entries
        .checked_sub(1)
        .ok_or_else(|| ignored_scan_failure("the ignored-folder entry limit was reached"))?;
    if std::time::Instant::now() >= budget.until {
        return Err(ignored_scan_failure(
            "the ignored-folder scan deadline was reached",
        ));
    }
    Ok(())
}

/// How much of the ignored folders one removal looks through for a nested
/// repository. A walk that runs out, or a folder it cannot read, refuses the
/// removal: `git worktree remove` would delete what was not looked at.
const IGNORED_WALK_ENTRIES: usize = 2_000_000;
const IGNORED_WALK_TIME: Duration = Duration::from_secs(30);

pub struct WalkBudget {
    entries: usize,
    until: std::time::Instant,
}

impl WalkBudget {
    /// A budget of what a caller has left: `entries` more entries, for at
    /// most `time`.
    pub fn allowing(entries: usize, time: Duration) -> Self {
        Self {
            entries,
            until: std::time::Instant::now() + time,
        }
    }

    /// The entries and time still left.
    pub fn left(&self) -> (usize, Duration) {
        (
            self.entries,
            self.until
                .saturating_duration_since(std::time::Instant::now()),
        )
    }

    /// The budget of one worktree removal's look through its ignored folders.
    fn worktree() -> Self {
        Self {
            entries: IGNORED_WALK_ENTRIES,
            until: std::time::Instant::now() + IGNORED_WALK_TIME,
        }
    }
}

/// Whether `folder` or any folder below it holds a `.git`. Links are not
/// followed: removal deletes the link, not what it points to.
fn holds_repository(folder: &Path, budget: &mut WalkBudget) -> Result<bool, String> {
    let unchecked = |reason: String| {
        format!(
            "The worktree was not removed: {reason}, so Hide could not confirm it holds no other Git repository. Remove it in the terminal after checking."
        )
    };
    let mut pending = vec![folder.to_path_buf()];
    while let Some(folder) = pending.pop() {
        if std::fs::symlink_metadata(folder.join(".git")).is_ok() {
            return Ok(true);
        }
        let entries = std::fs::read_dir(&folder).map_err(|error| {
            unchecked(format!("{} could not be read ({error})", folder.display()))
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| {
                unchecked(format!("{} could not be read ({error})", folder.display()))
            })?;
            budget.entries = budget.entries.checked_sub(1).ok_or_else(|| {
                unchecked(format!(
                    "its ignored folders hold more than {IGNORED_WALK_ENTRIES} entries"
                ))
            })?;
            if std::time::Instant::now() >= budget.until {
                return Err(unchecked(format!(
                    "its ignored folders took longer than {} seconds to look through",
                    IGNORED_WALK_TIME.as_secs()
                )));
            }
            let kind = entry.file_type().map_err(|error| {
                unchecked(format!(
                    "{} could not be read ({error})",
                    entry.path().display()
                ))
            })?;
            if kind.is_dir() {
                pending.push(entry.path());
            }
        }
    }
    Ok(false)
}

/// `Command::output` within [`GIT_DEADLINE`], for the branch checks.
trait WithinDeadline {
    fn output_within_deadline(&mut self) -> Result<std::process::Output, String>;
}

impl WithinDeadline for Command {
    fn output_within_deadline(&mut self) -> Result<std::process::Output, String> {
        output_within(self, GIT_DEADLINE)
            .map_err(|error| format!("git could not be run: {error}"))?
            .ok_or_else(|| {
                format!(
                    "git did not finish within {} seconds and was stopped",
                    GIT_DEADLINE.as_secs()
                )
            })
    }
}

/// Runs `command` to completion, or kills it at `deadline` and answers
/// `None`. Both pipes are drained on their own threads the whole time, so a
/// child whose output outgrows the pipe buffer is never left blocked on a
/// write nobody reads. The child is an owned child that leads its own tree and
/// the deadline stops the whole tree, so a hook, filter or fsmonitor Git
/// started cannot keep a pipe open past it; a descendant that left the tree is
/// waited for only a moment and then left behind with its pipe.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
pub fn output_within(
    command: &mut Command,
    deadline: Duration,
) -> std::io::Result<Option<std::process::Output>> {
    use std::io::Read;
    use std::process::Stdio;
    use std::sync::mpsc;

    let mut child = OwnedChild::spawn(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped()),
    )?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buffer = Vec::new();
            if let Some(mut pipe) = pipe {
                let _: std::io::Result<usize> = pipe.read_to_end(&mut buffer);
            }
            let _ = sender.send(buffer);
        });
        receiver
    };
    let stdout = drain(
        child
            .take_stdout()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let stderr = drain(
        child
            .take_stderr()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let started = std::time::Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if started.elapsed() >= deadline {
            child.kill_tree()?;
            child.wait()?;
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let drained = std::time::Instant::now().max(started + deadline) + Duration::from_secs(1);
    let collect = |receiver: mpsc::Receiver<Vec<u8>>| {
        receiver
            .recv_timeout(drained.saturating_duration_since(std::time::Instant::now()))
            .ok()
    };
    let (stdout, stderr) = (collect(stdout), collect(stderr));
    // A pipe still open once git has ended is held by something it started,
    // so what was read may be cut short: the call did not finish, never an
    // empty answer a safety check would read as clean.
    let (Some(status), Some(stdout), Some(stderr)) = (status, stdout, stderr) else {
        // The deadline path has already stopped the group.
        if status.is_some() {
            let _ = child.kill_tree();
        }
        return Ok(None);
    };
    Ok(Some(std::process::Output {
        status,
        stdout,
        stderr,
    }))
}

// --- creation --------------------------------------------------------------

/// Git's own rule for a new branch name, then a refusal when the branch
/// already exists and no worktree holds it, asked before anything is created
/// so a refused name leaves the repository untouched. A branch another
/// worktree holds passes: Herdr's `worktree.create` answers Git's own
/// "already used by worktree" refusal for it.
pub fn check_new_branch(repository_root: &Path, branch: &str) -> HostResult<()> {
    // A leading `-` is refused first because Git would read it as an option.
    let invalid = || {
        HostError::new(
            ErrorCode::InvalidRequest,
            format!("{branch} is not a valid branch name"),
        )
    };
    if branch.starts_with('-') {
        return Err(invalid());
    }
    let checked = hide_platform::programs::command("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(repository_root)
        .args(["check-ref-format", "--branch", branch])
        .output_within_deadline()
        .map_err(io_error)?;
    if !checked.status.success() {
        return Err(invalid());
    }
    let reference = format!("refs/heads/{branch}");
    let exists = hide_platform::programs::command("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(repository_root)
        .args(["show-ref", "--verify", "--quiet", &reference])
        .output_within_deadline()
        .map_err(io_error)?;
    if !exists.status.success() {
        return match exists.status.code() {
            Some(1) => Ok(()),
            _ => Err(io_error(
                String::from_utf8_lossy(&exists.stderr).trim().to_owned(),
            )),
        };
    }
    let worktrees = git(repository_root, &["worktree", "list", "--porcelain"]).map_err(io_error)?;
    if worktrees
        .lines()
        .any(|line| line == format!("branch {reference}"))
    {
        return Ok(());
    }
    // Ask Git itself for the branch-exists diagnostic. This command is
    // side-effect free because the branch was proven to exist above.
    let refusal = hide_platform::programs::command("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(repository_root)
        .args(["branch", "--", branch])
        .output_within_deadline()
        .map_err(io_error)?;
    if refusal.status.success() {
        return Err(io_error(
            "git branch existence check unexpectedly succeeded".into(),
        ));
    }
    let detail = String::from_utf8_lossy(&refusal.stderr).trim().to_owned();
    Err(HostError::new(
        ErrorCode::AlreadyExists,
        if detail.is_empty() {
            format!("git branch exited with {}", refusal.status)
        } else {
            detail
        },
    ))
}

/// The real path of an existing directory, or `None`; how the core confirms
/// that the folder Herdr says it created is there and is the one it listed.
pub fn directory(path: &Path) -> Option<String> {
    let real = hide_platform::fs::identity::canonical(path).ok()?;
    real.is_dir().then(|| real.to_string_lossy().into_owned())
}

// --- removal ---------------------------------------------------------------

pub use hide_node_link::worktrees::Registered;

/// Every worktree Git registers for `root`, the main worktree first.
pub fn registered(root: &Path) -> Result<Vec<Registered>, String> {
    registered_until(root, Instant::now() + GIT_DEADLINE)
}

fn registered_until(root: &Path, until: Instant) -> Result<Vec<Registered>, String> {
    let text = git_until(root, &["worktree", "list", "--porcelain", "-z"], until)?;
    let mut rows = Vec::new();
    for record in text.split("\0\0").filter(|v| !v.is_empty()) {
        let fields: Vec<_> = record.split('\0').collect();
        let path = fields
            .iter()
            .find_map(|v| v.strip_prefix("worktree "))
            .ok_or("Git returned an invalid worktree record. Refresh the review.")?;
        rows.push(Registered {
            path: path.into(),
            branch: fields
                .iter()
                .find_map(|v| v.strip_prefix("branch refs/heads/"))
                .map(str::to_owned),
            head: fields
                .iter()
                .find_map(|v| v.strip_prefix("HEAD "))
                .map(str::to_owned),
            locked: fields.iter().any(|v| lock_reason(v).is_some()),
            lock_reason: fields.iter().find_map(|v| lock_reason(v)),
            bare: fields.contains(&"bare"),
            unavailable: fields
                .iter()
                .any(|v| v.starts_with("prunable") || *v == "bare"),
        });
    }
    if rows.is_empty() {
        return Err("Git returned no main checkout. Refresh the review.".into());
    }
    Ok(rows)
}

fn lock_reason(field: &str) -> Option<String> {
    if field == "locked" {
        Some(String::new())
    } else {
        field.strip_prefix("locked ").map(str::to_owned)
    }
}

/// Removes the worktree folder and its registration, keeping the branch.
/// `force` removes it with uncommitted changes too, which the operator must
/// have accepted.
///
/// Every build cache the checkout owns lives inside it (`target/`,
/// `web/dist/`), so this is the whole cleanup. Deleting a multi-GB build
/// folder takes longer than any Git read may, so the folder is first moved
/// aside into the repository's [`TRASH`] folder, one rename on the same
/// volume; `git worktree remove` then finds it missing and drops only the
/// registration, and the files are deleted in the background
/// ([`sweep_trash`]). The folder is moved only when Git would remove it -
/// registered, not the main worktree, not locked, and clean unless `force` -
/// so every refusal is still Git's own. A folder that cannot be moved (the
/// checkout sits on another volume than its repository) is removed by Git in
/// place, within [`REMOVE_DEADLINE`].
pub fn remove_worktree(
    repository_root: &Path,
    checkout: &Path,
    force: bool,
) -> Result<String, String> {
    let common = git(
        repository_root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )
    .ok()
    .map(|path| PathBuf::from(path.trim()));
    let set_aside = common
        .as_deref()
        .and_then(|common| set_aside(repository_root, common, checkout, force));
    let checkout_arg = checkout.to_string_lossy();
    let mut arguments = vec!["worktree", "remove"];
    if force {
        arguments.push("--force");
    }
    arguments.extend(["--", &checkout_arg]);
    // Moved aside, Git only drops the registration, which is a quick write;
    // only a folder left in place needs the removal's own bound.
    let deadline = if set_aside.is_some() {
        GIT_DEADLINE
    } else {
        REMOVE_DEADLINE
    };
    let answer = git_within(repository_root, &arguments, deadline);
    // A folder moved aside goes back while Git still registers it, so a
    // refused removal leaves the worktree exactly where it was; the same
    // rule decides what a sweep may delete.
    if let Some(aside) = set_aside.as_ref()
        && still_registered(&aside.admin)
    {
        std::fs::rename(&aside.entry, checkout).map_err(|error| {
            format!(
                "Git kept the worktree but its folder could not be moved back from {} ({error}). Move it back to {} before retrying.",
                aside.entry.display(),
                checkout.display()
            )
        })?;
    }
    // Every removal sweeps, so an entry a stopped process left behind goes
    // with the next removal in its repository, including its own retry.
    if let Some(common) = common.as_deref() {
        sweep_trash(&common.join(TRASH));
    }
    // What is on disk decides, so an answer cut short (a held pipe) after
    // Git removed the folder still reads as removed, and a failure whose
    // readback also fails keeps Git's own reason.
    let listed = registered(repository_root)
        .map(|rows| rows.iter().any(|row| Path::new(&row.path) == checkout));
    let remains = match (listed, &answer) {
        (Ok(listed), Err(_)) => listed || checkout.try_exists().unwrap_or(true),
        (Ok(listed), Ok(_)) => {
            listed || checkout.try_exists().map_err(|error| error.to_string())?
        }
        (Err(_), Err(_)) => true,
        (Err(error), Ok(_)) => return Err(error),
    };
    match (answer, remains) {
        (_, false) => {
            forget_agent_trust(checkout);
            Ok("Worktree folder and its build output removed. Branch and Git history kept.".into())
        }
        (Err(error), true) => Err(error),
        (Ok(_), true) => Err(
            "Git acknowledged removal but the folder or registration remains. Review again.".into(),
        ),
    }
}

/// Takes out the folder trust Hide recorded for a removed worktree, in each
/// agent's own settings (docs/agent-hooks.md, Folder trust for the worktrees
/// Hide makes). The worktree is already gone, so a trust that cannot be taken
/// out is only recorded: the operator has nothing to do, its record stays,
/// and switching the agent off still takes it out.
fn forget_agent_trust(checkout: &Path) {
    use hide_agent_hooks::folder_trust::{self, Agent, Tools};
    let Ok(home) = hide_platform::host::home_dir() else {
        return;
    };
    let stop = crate::kit::process_stop();
    for agent in Agent::ALL {
        let codex = (agent == Agent::Codex)
            .then(|| hide_ai::resolve_binary(Path::new("codex")))
            .flatten();
        let tools = Tools {
            codex: codex.as_deref(),
            stop: &stop,
        };
        if let Err(failure) = folder_trust::forget(agent, &home, checkout, tools) {
            eprintln!(
                "{}",
                serde_json::json!({
                    "component": "worktree_removal",
                    "kind": "agent_trust_kept",
                    "agent": agent.label(),
                    "checkout": checkout,
                    "reason": failure.message(),
                })
            );
        }
    }
}

/// The bound on a `git worktree remove` that deletes the folder itself,
/// which only a checkout that could not be moved aside needs. It stays under
/// the core's wait for the removal's answer, so a removal that runs out is
/// reported by the host rather than left unknown.
pub const REMOVE_DEADLINE: Duration = Duration::from_secs(60);

/// The folder under the repository's shared Git directory where removed
/// worktree folders wait to be deleted. Git ignores it, the operator never
/// browses it, and it is on the checkout's volume in the usual layout.
pub const TRASH: &str = "hide-removed";

/// A worktree folder moved into its repository's [`TRASH`]: the entry the
/// folder now is, and Git's administrative folder that registers it.
struct SetAside {
    entry: PathBuf,
    admin: PathBuf,
}

/// Moves `checkout` into the repository's [`TRASH`] when Git would remove
/// it. The entry is named `<nanos>-<pid>-<admin id>`, so [`sweep_trash`] can
/// tell from Git's own registration whether it is still a worktree. `None`
/// leaves the folder where it is for Git to remove or refuse in place.
fn set_aside(root: &Path, common: &Path, checkout: &Path, force: bool) -> Option<SetAside> {
    let rows = registered(root).ok()?;
    let index = rows
        .iter()
        .position(|row| Path::new(&row.path) == checkout)?;
    if index == 0 || rows[index].locked || rows[index].unavailable {
        return None;
    }
    if !std::fs::symlink_metadata(checkout).ok()?.is_dir() {
        return None;
    }
    // Git's own check before it deletes a folder it was not forced to.
    if !force
        && !git(
            checkout,
            &["status", "--porcelain", "--ignore-submodules=none"],
        )
        .ok()?
        .trim()
        .is_empty()
    {
        return None;
    }
    let id = admin_id(checkout)?;
    let admin = common.join("worktrees").join(&id);
    if !admin.is_dir() {
        return None;
    }
    let trash = common.join(TRASH);
    std::fs::create_dir_all(&trash).ok()?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    let entry = trash.join(format!("{nanos}-{}-{id}", std::process::id()));
    if let Err(error) = std::fs::rename(checkout, &entry) {
        // Git removes the folder in place from here; record why the cheap
        // path was not taken, since the removal's own result will not.
        let attempt = entry
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        eprintln!(
            "{}",
            serde_json::json!({
                "component": "worktree_removal",
                "kind": "set_aside_refused",
                "checkout": checkout,
                "attempt": attempt,
                "os_error": error.raw_os_error(),
                "message": error.to_string(),
            })
        );
        #[cfg(windows)]
        record_holders(checkout, attempt);
        return None;
    }
    Some(SetAside { entry, admin })
}

/// The one scan for holders that may run at a time (see [`record_holders`]).
#[cfg(windows)]
static HOLDER_SCAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What [`spawn_exclusive`] did with the work it was handed.
#[cfg(any(windows, test))]
#[derive(Debug)]
enum Spawned {
    /// The work runs on the thread this handle joins.
    Started(std::thread::JoinHandle<()>),
    /// The slot was taken, so the work did not run.
    Busy,
    Failed(std::io::Error),
}

/// Runs `work` on its own thread unless the thread of an earlier call still
/// holds `slot`, which the thread's end frees on every exit, a panic included.
/// The caller never waits for the work: a call that blocks forever keeps the
/// slot and costs one thread, and every later call answers [`Spawned::Busy`]
/// instead of adding another. A caller that has no use for the thread drops
/// the handle, and a test joins it to know the slot is free.
#[cfg(any(windows, test))]
fn spawn_exclusive(
    slot: &'static std::sync::atomic::AtomicBool,
    name: &str,
    work: impl FnOnce() + Send + 'static,
) -> Spawned {
    use std::sync::atomic::Ordering;
    if slot
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return Spawned::Busy;
    }
    struct Frees(&'static std::sync::atomic::AtomicBool);
    impl Drop for Frees {
        fn drop(&mut self) {
            self.0.store(false, Ordering::Release);
        }
    }
    // The guard moves into the thread, so a thread that never starts frees the
    // slot when the closure is dropped, and one that ends frees it as it ends.
    let frees = Frees(slot);
    match std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let _frees = frees;
            work();
        }) {
        Ok(thread) => Spawned::Started(thread),
        Err(error) => Spawned::Failed(error),
    }
}

/// One `set_aside_holders` line: the refusal's `checkout` and `attempt`, which
/// make it read with the `set_aside_refused` line, and what is known of the
/// holders in `detail`.
#[cfg(windows)]
fn holders_line(checkout: &Path, attempt: &Option<String>, detail: serde_json::Value) {
    let mut record = serde_json::json!({
        "component": "worktree_removal",
        "kind": "set_aside_holders",
        "checkout": checkout,
        "attempt": attempt,
    });
    if let (Some(record), Some(detail)) = (record.as_object_mut(), detail.as_object()) {
        record.extend(detail.clone());
    }
    eprintln!("{record}");
}

/// Records who holds the folder a rename was refused for, as a line of its own
/// (`set_aside_holders`) carrying the refusal's `attempt`. The scan reads the
/// system's table of open handles, and a handle of some kinds can block the
/// questions asked of it, so it never runs on the removal's thread: the removal
/// goes on to Git's own removal at once, and the line is written when the scan
/// ends. Only Windows has the question, so no other system writes the line.
#[cfg(windows)]
fn record_holders(checkout: &Path, attempt: Option<String>) {
    let folder = checkout.to_path_buf();
    let (scan_folder, scan_attempt) = (folder.clone(), attempt.clone());
    let spawned = spawn_exclusive(&HOLDER_SCAN, "set-aside-holders", move || {
        let started = std::time::Instant::now();
        let detail = match hide_platform::fs::holders::holders_of(&scan_folder, 256) {
            Ok(holders) => serde_json::json!({
                "self_pid": std::process::id(),
                "scan_ms": started.elapsed().as_millis() as u64,
                "processes": holders
                    .iter()
                    .map(|holder| serde_json::json!({
                        "pid": holder.pid,
                        "process": holder.process,
                        "path": holder.path,
                        "access": format!("{:#x}", holder.access),
                    }))
                    .collect::<Vec<_>>(),
            }),
            Err(error) => serde_json::json!({"unavailable": error.to_string()}),
        };
        holders_line(&scan_folder, &scan_attempt, detail);
    });
    match spawned {
        Spawned::Started(_) => {}
        Spawned::Busy => holders_line(&folder, &attempt, serde_json::json!({"scan_busy": true})),
        Spawned::Failed(error) => holders_line(
            &folder,
            &attempt,
            serde_json::json!({"unavailable": error.to_string()}),
        ),
    }
}

/// The name of the worktree's administrative folder, from the `gitdir:`
/// line of its `.git` file.
fn admin_id(checkout: &Path) -> Option<String> {
    let text = std::fs::read_to_string(checkout.join(".git")).ok()?;
    let gitdir = text
        .lines()
        .find_map(|line| line.strip_prefix("gitdir: "))?;
    Path::new(gitdir.trim())
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// Moves a folder that is not a worktree into the repository's [`TRASH`] with
/// one rename, so it is gone from its checkout at once and [`sweep_trash`]
/// deletes it in the background. The rename stays on one volume, so a folder
/// on another volume than the repository's Git directory is an error and is
/// left where it is. The entry's name has no registered worktree id, so a
/// sweep always treats it as deletable.
pub fn set_aside_folder(common: &Path, folder: &Path) -> Result<PathBuf, std::io::Error> {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let trash = common.join(TRASH);
    std::fs::create_dir_all(&trash)?;
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let entry = trash.join(format!("{nanos}-{}-~folder{sequence}", std::process::id()));
    std::fs::rename(folder, &entry)?;
    Ok(entry)
}

/// The names of the entries waiting in the repository's [`TRASH`] now.
pub fn trash_entries(common: &Path) -> std::collections::BTreeSet<PathBuf> {
    std::fs::read_dir(common.join(TRASH))
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .collect()
}

/// Starts the deletion of everything waiting in the repository's [`TRASH`]
/// and waits up to `wait` for the `ours` entries to finish, sweeping again
/// every few seconds so a deletion that stopped is retried. Entries another
/// process put there are swept too but never waited for. Returns how many of
/// `ours` remain.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
pub fn drain_trash(
    common: &Path,
    ours: &std::collections::BTreeSet<PathBuf>,
    wait: Duration,
) -> usize {
    let trash = common.join(TRASH);
    let until = std::time::Instant::now() + wait;
    let mut swept = None;
    loop {
        if swept.is_none_or(|at: std::time::Instant| at.elapsed() >= Duration::from_secs(5)) {
            sweep_trash(&trash);
            swept = Some(std::time::Instant::now());
        }
        let remaining = ours
            .iter()
            .filter(|entry| std::fs::symlink_metadata(entry).is_ok())
            .count();
        if remaining == 0 || std::time::Instant::now() >= until {
            return remaining;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Whether `folder` or any folder below it holds a `.git`, looked through
/// within `budget`, which the caller shares across a whole run.
pub fn contains_repository(folder: &Path, budget: &mut WalkBudget) -> Result<bool, String> {
    holds_repository(folder, budget)
}

/// Entries of a trash folder being deleted by this process, so two sweeps
/// never delete the same folder at once.
static DELETING: std::sync::Mutex<std::collections::BTreeSet<PathBuf>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Deletes, each on its own thread, every entry of `trash` that is no longer
/// a worktree: Git has no administrative folder for it, or that folder now
/// belongs to another worktree whose `.git` exists. An entry Git still
/// registers - a removal that has not reached Git yet, or one whose folder
/// could not be moved back - is kept.
pub fn sweep_trash(trash: &Path) {
    let Some(common) = trash.parent() else {
        return;
    };
    // The trash is a real folder of this repository, never a link that leads
    // a sweep somewhere else.
    if !std::fs::symlink_metadata(trash).is_ok_and(|metadata| metadata.is_dir()) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(trash) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        // Only what a removal named: `<nanos>-<pid>-<id>`.
        let Some(admin) = trash_entry_id(&name) else {
            continue;
        };
        if still_registered(&common.join("worktrees").join(admin)) {
            continue;
        }
        if !DELETING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(path.clone())
        {
            continue;
        }
        let spawned = std::thread::Builder::new()
            .name("hide-worktree-trash".into())
            .spawn({
                let path = path.clone();
                move || {
                    let result = std::fs::remove_dir_all(&path);
                    DELETING
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&path);
                    if let Err(error) = result
                        && error.kind() != std::io::ErrorKind::NotFound
                    {
                        eprintln!(
                            "{}",
                            serde_json::json!({
                                "component": "worktree_trash",
                                "kind": "delete_failed",
                                "entry": path,
                                "message": error.to_string(),
                            })
                        );
                    }
                }
            });
        if spawned.is_err() {
            DELETING
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&path);
        }
    }
}

/// The id after `<digits>-<digits>-` in a trash entry's name, or `None` for a
/// name no removal of ours would have made.
fn trash_entry_id(name: &str) -> Option<&str> {
    let mut parts = name.splitn(3, '-');
    let (nanos, pid, id) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    (digits(nanos) && digits(pid) && !id.is_empty()).then_some(id)
}

/// Whether Git's administrative folder `admin` still registers the folder
/// that was moved away from it: it exists and the `.git` it names is gone.
fn still_registered(admin: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(admin.join("gitdir")) else {
        // No readable registration: missing means removed; anything else is
        // kept rather than guessed at.
        return admin.exists();
    };
    let named = Path::new(text.trim());
    let named = if named.is_absolute() {
        named.to_path_buf()
    } else {
        admin.join(named)
    };
    !named.exists()
}

pub use hide_node_link::worktrees::{
    ConfirmedRemoval, RemovalOutcome, RepositoryWorktrees, Unpushed, WorktreeFacts,
};

/// Measures the accepted deletion without changing files or Git registration.
/// The core asks before closing any pane; removal repeats the same checks.
pub fn check_removal(request: &ConfirmedRemoval) -> Result<(), String> {
    check_confirmed_removal(request, false)
}

fn check_confirmed_removal(request: &ConfirmedRemoval, panes_closed: bool) -> Result<(), String> {
    let root = Path::new(&request.repository_root);
    let target = Path::new(&request.checkout_path);
    let stopped = |detail: String| {
        let detail = detail.trim_end_matches('.');
        format!(
            "Worktree removal stopped: {detail}. {}",
            if panes_closed {
                "The worktree remains; panes already closed stay closed."
            } else {
                "The worktree and its panes are kept. Review again before deleting."
            }
        )
    };
    let rows = registered(root).map_err(|error| {
        stopped(format!(
            "could not re-read the worktree registration: {error}"
        ))
    })?;
    let Some(index) = rows.iter().position(|row| Path::new(&row.path) == target) else {
        return Err(stopped(
            "the worktree is no longer registered at this path".into(),
        ));
    };
    if index == 0 {
        return Err(stopped("the main worktree cannot be deleted".into()));
    }
    let current = &rows[index];
    if let Some(reason) = current.lock_reason.as_deref() {
        let name = current
            .branch
            .as_deref()
            .or_else(|| target.file_name().and_then(|name| name.to_str()))
            .unwrap_or("selected checkout");
        return Err(stopped(hide_node_link::worktrees::locked_removal_reason(
            name, reason,
        )));
    }
    if current.head != request.expected_head_sha || current.branch != request.expected_branch {
        return Err(stopped(
            "the worktree identity changed after confirmation".into(),
        ));
    }
    if current.branch.is_some() && current.branch == request.protected_base_branch {
        return Err(stopped(
            "the worktree now holds the protected base branch".into(),
        ));
    }
    if !request.discard_changes
        && rows
            .iter()
            .any(|row| Path::new(&row.path) != target && Path::new(&row.path).starts_with(target))
    {
        return Err(stopped(
            "the worktree now contains a nested worktree".into(),
        ));
    }
    if target
        .try_exists()
        .map_err(|error| stopped(error.to_string()))?
    {
        if !request.discard_changes {
            let status = git(
                target,
                &["status", "--porcelain=v1", "--untracked-files=all"],
            )
            .map_err(|error| stopped(format!("could not recheck the worktree state: {error}")))?;
            let changed = status
                .lines()
                .filter(|line| !line.trim().is_empty())
                .count();
            if changed > 0 {
                return Err(if panes_closed {
                    format!(
                        "Not deleted: {} changed after you confirmed, likely written by an agent as it stopped. \
                 The worktree and its files are kept; the panes already closed stay closed. \
                 Delete again and tick Discard to remove it anyway.",
                        if changed == 1 {
                            "1 file".to_owned()
                        } else {
                            format!("{changed} files")
                        }
                    )
                } else {
                    stopped(format!(
                        "{changed} files changed after confirmation; review again and explicitly choose Discard"
                    ))
                });
            }
        }
        // Git leaves ignored folders out of the status above, and removing
        // the worktree deletes them, including a repository cloned into one
        // (B28): such a nested repository stops the removal.
        let nested = ignored_repositories(target).map_err(stopped)?;
        if !nested.is_empty() && !request.discard_changes {
            return Err(stopped(format!(
                "ignored repositories would be deleted: {}. Review and explicitly choose Discard",
                nested.join(", ")
            )));
        }
        if nested != request.expected_ignored_repositories {
            return Err(stopped("the ignored repository list changed after confirmation; refresh and review every repository before choosing Discard".to_owned()));
        }
    }
    Ok(())
}

/// Repeats the preflight after pane closure, then applies the existing
/// guarded removal and only the branch policy the operator accepted.
pub fn remove_confirmed(request: &ConfirmedRemoval) -> Result<String, String> {
    check_confirmed_removal(request, true)?;
    let root = Path::new(&request.repository_root);
    let target = Path::new(&request.checkout_path);
    remove_worktree(root, target, request.discard_changes).map_err(|error| {
        format!("git worktree remove failed: {error}. The worktree remains; panes already closed stay closed.")
    })?;
    let path = target.display();
    let Some(branch) = request.delete_branch.as_deref() else {
        return Ok(format!("Deleted {path}. Its local branch was kept."));
    };
    let flag = if request.force_delete_branch {
        "-D"
    } else {
        "-d"
    };
    match git(root, &["branch", flag, "--", branch]) {
        Ok(_) => Ok(format!("Deleted {path} and local branch {branch}.")),
        Err(detail) => Ok(format!("Deleted {path}; branch {branch} remains: {detail}")),
    }
}

fn io_error(message: String) -> HostError {
    HostError::new(ErrorCode::Io, message)
}

#[cfg(test)]
mod ignored_repository_tests {
    use super::*;

    fn run(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    #[test]
    fn an_incomplete_ignored_scan_never_claims_the_repository_list_is_empty() {
        let directory = tempfile::tempdir().unwrap();
        let repo = directory.path();
        run(repo, &["init", "-q"]);
        std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
        std::fs::create_dir_all(repo.join("target/deep")).unwrap();
        let mut exhausted = WalkBudget {
            entries: 1,
            until: std::time::Instant::now() + IGNORED_WALK_TIME,
        };
        assert!(
            scan_ignored_repositories(repo, &mut exhausted)
                .unwrap_err()
                .contains("entry limit")
        );
        let mut expired = WalkBudget {
            entries: IGNORED_WALK_ENTRIES,
            until: std::time::Instant::now(),
        };
        assert!(
            scan_ignored_repositories(repo, &mut expired)
                .unwrap_err()
                .contains("deadline")
        );
        std::fs::write(repo.join("target/.git"), "gitdir: unavailable\n").unwrap();
        assert!(
            ignored_repositories(repo)
                .unwrap_err()
                .contains("Git could not verify repository target")
        );
    }

    /// D-15: the deadline stops Git's descendants too, so one that still
    /// holds the output pipe cannot keep the helper's worker past it.
    #[cfg(unix)]
    #[test]
    fn a_descendant_holding_the_pipe_does_not_outlast_the_deadline() {
        let started = std::time::Instant::now();
        let output = output_within(
            Command::new("sh").args(["-c", "sleep 30 & sleep 30"]),
            Duration::from_millis(200),
        )
        .unwrap();
        assert!(output.is_none());
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    /// B28: a pipe something git started still holds after git ended is not
    /// read as an empty, successful answer, which a dirty or nested-repository
    /// check would take for clean; the call did not finish.
    #[cfg(unix)]
    #[test]
    fn output_held_open_after_the_command_ended_is_not_an_empty_success() {
        let started = std::time::Instant::now();
        let output = output_within(
            Command::new("sh").args(["-c", "sleep 30 & echo partial"]),
            Duration::from_millis(200),
        )
        .unwrap();
        assert!(output.is_none(), "{output:?}");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "took {:?}",
            started.elapsed()
        );
    }

    /// B28: a repository cloned into an ignored folder is found, because a
    /// worktree removal would delete it; ignored build output is not.
    #[test]
    fn a_repository_inside_an_ignored_folder_is_found() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path();
        run(repo, &["init", "-q"]);
        std::fs::write(repo.join(".gitignore"), "vendor/\nbuild/\n").unwrap();
        std::fs::create_dir_all(repo.join("build/out")).unwrap();
        std::fs::write(repo.join("build/out/a.o"), "x").unwrap();
        assert_eq!(ignored_repository(repo).unwrap(), None);

        std::fs::create_dir_all(repo.join("vendor/lib")).unwrap();
        run(&repo.join("vendor/lib"), &["init", "-q"]);
        assert_eq!(ignored_repository(repo).unwrap(), Some("vendor".to_owned()));
        std::fs::remove_dir_all(repo.join("vendor")).unwrap();

        // Deeper than any fixed limit would look (GOPATH style).
        let deep = repo.join("build/src/github.com/org/repo");
        std::fs::create_dir_all(&deep).unwrap();
        run(&deep, &["init", "-q"]);
        assert_eq!(ignored_repository(repo).unwrap(), Some("build".to_owned()));
        std::fs::remove_dir_all(deep.join(".git")).unwrap();

        // A folder that cannot be read refuses rather than passing unseen.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let sealed = repo.join("build/src/sealed");
            std::fs::create_dir_all(&sealed).unwrap();
            std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o000)).unwrap();
            let refused = ignored_repository(repo);
            std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert!(refused.unwrap_err().contains("could not confirm"));
        }
    }
}

#[cfg(test)]
mod holder_scan_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    /// A scan that never ends costs one thread and no one waits for it: the call
    /// that started it returns at once, a later call is told the slot is taken
    /// instead of adding a thread, and the slot comes back when the scan ends.
    #[test]
    fn a_scan_that_blocks_holds_the_slot_and_nothing_waits_for_it() {
        static SLOT: AtomicBool = AtomicBool::new(false);
        let (release, blocked) = mpsc::channel::<()>();

        let first = spawn_exclusive(&SLOT, "test-scan", move || {
            // Blocks until the test lets go, as a handle that never answers would.
            let _ = blocked.recv();
        });
        let thread = match first {
            Spawned::Started(thread) => thread,
            Spawned::Busy => panic!("the slot was free"),
            Spawned::Failed(error) => panic!("the scan thread did not start: {error}"),
        };
        assert!(
            SLOT.load(Ordering::Acquire),
            "the running scan holds the slot"
        );

        let second = spawn_exclusive(&SLOT, "test-scan", || {
            panic!("must not run while the slot is taken")
        });
        assert!(matches!(second, Spawned::Busy));

        release.send(()).unwrap();
        // The thread frees the slot as it ends, so once it is joined it is free.
        thread.join().expect("the scan ends once released");
        assert!(
            !SLOT.load(Ordering::Acquire),
            "the slot comes back when the scan ends"
        );
        assert!(matches!(
            spawn_exclusive(&SLOT, "test-scan", || {}),
            Spawned::Started(_)
        ));
    }

    #[test]
    fn a_scan_that_panics_still_frees_the_slot() {
        static SLOT: AtomicBool = AtomicBool::new(false);
        let started = spawn_exclusive(&SLOT, "test-scan", || panic!("a scan that fails"));
        let Spawned::Started(thread) = started else {
            panic!("the slot was free and the thread should start");
        };
        assert!(thread.join().is_err(), "the scan panicked");
        assert!(!SLOT.load(Ordering::Acquire), "a panic frees the slot too");
    }
}

#[cfg(test)]
mod measure_tests {
    use super::*;

    /// A worktree git lists but disk does not have reports `missing` and no
    /// counts, so an absent checkout never reads as a clean one.
    #[test]
    fn a_missing_worktree_reports_no_counts() {
        let described = describe_within(
            ListedWorktree {
                path: PathBuf::from("/definitely/not/here/hide-test"),
                branch: Some("gone".to_owned()),
                is_main: false,
                bare: false,
                head_sha: None,
                lock_reason: None,
            },
            &Bases {
                by_branch: &BTreeMap::new(),
                default_branch: Some("main"),
                chosen: None,
            },
            Instant::now() + WORKTREE_MEASURE_TIME,
        );
        assert!(described.missing);
        assert!(!described.dirty);
        assert_eq!(described.ahead, 0);
        assert_eq!(described.behind, 0);
        assert_eq!(described.unpushed, None);
    }

    fn run(cwd: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(cwd)
            .args([
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
            ])
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A repository with a commit on `main` and a linked worktree for each
    /// of `linked`, all inside one folder removed with the fixture.
    struct Fixture {
        folder: tempfile::TempDir,
        root: PathBuf,
    }

    impl Fixture {
        fn new(linked: &[&str]) -> Self {
            let folder = tempfile::tempdir().unwrap();
            let root = std::fs::canonicalize(folder.path()).unwrap().join("repo");
            std::fs::create_dir_all(&root).unwrap();
            run(&root, &["init", "-q", "-b", "main"]);
            run(&root, &["commit", "-q", "--allow-empty", "-m", "initial"]);
            for name in linked {
                let path = root.with_file_name(name);
                run(
                    &root,
                    &["worktree", "add", "-q", "-b", name, path.to_str().unwrap()],
                );
            }
            Self { folder, root }
        }

        fn linked(&self, name: &str) -> PathBuf {
            self.root.with_file_name(name)
        }

        /// One read of every listed worktree measured against `main`.
        fn measure(
            &self,
            measured: &Measured,
            reuse_unchanged: bool,
            time: (Instant, Duration),
        ) -> BTreeMap<String, WorktreeFacts> {
            let bases = BTreeMap::new();
            let bases = Bases {
                by_branch: &bases,
                default_branch: Some("main"),
                chosen: None,
            };
            measure(
                measured,
                &self.root,
                listed_worktrees(&self.root, Instant::now() + GIT_DEADLINE).unwrap(),
                &bases,
                Refs::read(&self.root, Instant::now() + GIT_DEADLINE).as_ref(),
                reuse_unchanged,
                time,
            )
            .into_iter()
            .map(|facts| (facts.branch.clone().unwrap(), facts))
            .collect()
        }
    }

    /// One worktree that does not answer is stopped at its own limit and
    /// named, and the read still measures the worktree after it: before the
    /// limit, a slow worktree spent the whole repository's time and the core
    /// gave up on every worktree's answer.
    #[test]
    fn a_worktree_that_does_not_answer_is_named_and_the_next_is_still_measured() {
        let fixture = Fixture::new(&["stuck", "after"]);
        // The subject of the test: a file system monitor that never answers
        // holds `git status` in the stuck worktree until it is stopped.
        let hook = fixture.folder.path().join("never-answers");
        std::fs::write(&hook, "#!/bin/sh\nexec sleep 600\n").unwrap();
        #[cfg(unix)]
        std::fs::set_permissions(&hook, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        run(
            &fixture.root,
            &["config", "extensions.worktreeConfig", "true"],
        );
        run(
            &fixture.linked("stuck"),
            &[
                "config",
                "--worktree",
                "core.fsmonitor",
                hook.to_str().unwrap(),
            ],
        );
        let facts = fixture.measure(
            &Measured::default(),
            false,
            (
                Instant::now() + Duration::from_secs(120),
                Duration::from_secs(3),
            ),
        );
        let stuck = &facts["stuck"];
        assert!(
            stuck
                .unavailable_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("Worktree stuck was not fully measured")),
            "{stuck:?}"
        );
        assert!(
            stuck.ignored_scan_unavailable.is_some(),
            "an unscanned worktree never claims it holds no repository"
        );
        for name in ["main", "after"] {
            assert_eq!(facts[name].unavailable_reason, None, "{name}");
        }
        assert_eq!(facts["after"].ignored_scan_unavailable, None);
    }

    /// A read with no time left measures nothing: a worktree whose inputs
    /// did not move answers its last measurement, and one that moved is
    /// named, never shown with the facts of its old commit.
    #[test]
    fn a_read_out_of_time_keeps_what_did_not_move_and_names_what_did() {
        let fixture = Fixture::new(&["moved", "quiet"]);
        let measured = Measured::default();
        let ample = || {
            (
                Instant::now() + Duration::from_secs(120),
                WORKTREE_MEASURE_TIME,
            )
        };
        let first = fixture.measure(&measured, false, ample());
        run(
            &fixture.linked("moved"),
            &["commit", "-q", "--allow-empty", "-m", "moved"],
        );
        let spent = fixture.measure(&measured, true, (Instant::now(), WORKTREE_MEASURE_TIME));
        assert_eq!(spent["quiet"], first["quiet"]);
        assert_eq!(spent["main"], first["main"]);
        let moved = &spent["moved"];
        assert!(
            moved
                .unavailable_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("Worktree moved was not measured")),
            "{moved:?}"
        );
        assert_eq!(moved.ahead, 0);
        // The next read with time measures it, and only it.
        let next = fixture.measure(&measured, true, ample());
        assert_eq!(next["moved"].unavailable_reason, None);
        assert_eq!(next["moved"].ahead, 1);
        assert_eq!(next["quiet"], first["quiet"]);
    }

    fn ample() -> (Instant, Duration) {
        (
            Instant::now() + Duration::from_secs(120),
            WORKTREE_MEASURE_TIME,
        )
    }

    fn spent() -> (Instant, Duration) {
        (Instant::now(), WORKTREE_MEASURE_TIME)
    }

    /// A node that started again reads what it measured before from its
    /// folder: a worktree whose inputs did not move answers its last
    /// measurement with no time to measure it, and one that moved is not
    /// answered from it. Without the folder the same read has nothing.
    #[test]
    fn a_restarted_node_reuses_from_its_folder_only_what_did_not_move() {
        let fixture = Fixture::new(&["moved", "quiet"]);
        let folder = fixture.folder.path().join("worktree-facts");
        let first = fixture.measure(&Measured::kept_in(folder.clone()), false, ample());
        run(
            &fixture.linked("moved"),
            &["commit", "-q", "--allow-empty", "-m", "moved"],
        );

        let restarted = fixture.measure(&Measured::kept_in(folder), true, spent());
        assert_eq!(restarted["quiet"], first["quiet"]);
        assert_eq!(restarted["main"], first["main"]);
        assert!(
            restarted["moved"]
                .unavailable_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("Worktree moved was not measured")),
            "{:?}",
            restarted["moved"]
        );
        let forgotten = fixture.measure(&Measured::default(), true, spent());
        assert!(forgotten["quiet"].unavailable_reason.is_some());
    }

    /// A file of another version, or one that does not read, keeps nothing
    /// and fails no read; the next read with time writes it again.
    #[test]
    fn a_file_of_another_version_or_that_does_not_read_keeps_nothing() {
        let fixture = Fixture::new(&["quiet"]);
        let folder = fixture.folder.path().join("worktree-facts");
        fixture.measure(&Measured::kept_in(folder.clone()), false, ample());
        let file = kept_file(&folder, &fixture.root);
        let kept: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let mut older = kept.clone();
        older["version"] = serde_json::json!(KEPT_FILE_VERSION + 1);
        for broken in [older.to_string(), "{\"version\":".to_owned()] {
            std::fs::write(&file, broken).unwrap();
            let read = fixture.measure(&Measured::kept_in(folder.clone()), true, spent());
            assert!(
                read["quiet"].unavailable_reason.is_some(),
                "nothing was kept"
            );
        }
        fixture.measure(&Measured::kept_in(folder.clone()), true, ample());
        let read = fixture.measure(&Measured::kept_in(folder), true, spent());
        assert_eq!(read["quiet"].unavailable_reason, None);
    }

    /// The folder holds the files of the last [`GIT_WATCH_LIMIT`]
    /// repositories written, the one just written always among them.
    #[test]
    fn the_folder_keeps_the_files_of_the_last_repositories_written() {
        let folder = tempfile::tempdir().unwrap();
        let measured = Measured::kept_in(folder.path().to_path_buf());
        for n in 0..=GIT_WATCH_LIMIT {
            measured.keep(&PathBuf::from(format!("/repository-{n}")), Kept::new());
        }
        let files = std::fs::read_dir(folder.path()).unwrap().count();
        assert_eq!(files, GIT_WATCH_LIMIT);
        let last = PathBuf::from(format!("/repository-{GIT_WATCH_LIMIT}"));
        assert!(kept_file(folder.path(), &last).is_file());
    }
}
