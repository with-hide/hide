//! One durable project identity shared by the catalog, sessions, memory, and hooks.
//!
//! Git linked worktrees resolve to the main worktree. Plain folders retain their
//! canonical path. A failed resolution is an error, never permission to search a
//! display name or a neighbouring project.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectIdentity {
    pub id: String,
    /// Canonical main-worktree root used for durable Project identity.
    pub root: PathBuf,
    /// Canonical root of the checkout that contained the resolved path.
    ///
    /// This differs from `root` for linked worktrees and lets callers map a
    /// checkout-relative path into the durable Project namespace without
    /// treating the worktree itself as another Project.
    pub checkout_root: PathBuf,
    pub device_id: String,
    pub kind: ProjectKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    Git,
    Folder,
}

#[derive(Debug)]
pub enum ResolveError {
    EmptyDevice,
    MissingPath(PathBuf),
    Canonicalize {
        path: PathBuf,
        source: std::io::Error,
    },
    InvalidGitLink {
        path: PathBuf,
        reason: String,
    },
}

impl Display for ResolveError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyDevice => formatter.write_str("project_device_empty"),
            Self::MissingPath(path) => write!(formatter, "project_path_missing:{}", path.display()),
            Self::Canonicalize { path, source } => {
                write!(
                    formatter,
                    "project_path_unreadable:{}:{source}",
                    path.display()
                )
            }
            Self::InvalidGitLink { path, reason } => {
                write!(
                    formatter,
                    "project_git_link_invalid:{}:{reason}",
                    path.display()
                )
            }
        }
    }
}

impl Error for ResolveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Canonicalize { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// What the machine that owns `path` knows about its project: the durable
/// root, the checkout that contains it, its kind and the checked-out branch.
/// It carries no id: an id also names the device, which only the caller knows
/// (a device's helper answers these facts for the daemon that asked).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectFacts {
    pub root: PathBuf,
    pub checkout_root: PathBuf,
    pub kind: ProjectKind,
    pub branch: Option<String>,
    /// The checkout is a linked worktree of the repository at `root`.
    pub linked_worktree: bool,
}

pub fn facts(path: &Path) -> Result<ProjectFacts, ResolveError> {
    facts_and_repository(path).map(|(facts, _)| facts)
}

/// [`facts`] with the repository it was read from, so a caller that also
/// needs the repository's own files walks the folders up to `.git` once.
pub fn facts_and_repository(
    path: &Path,
) -> Result<(ProjectFacts, Option<git::Repository>), ResolveError> {
    if !path.exists() {
        return Err(ResolveError::MissingPath(path.to_path_buf()));
    }
    let canonical = fs::canonicalize(path).map_err(|source| ResolveError::Canonicalize {
        path: path.to_path_buf(),
        source,
    })?;
    if let Some(repository) = git::discover_checked(&canonical)? {
        let root = repository.main_root();
        let facts = ProjectFacts {
            linked_worktree: root != repository.root,
            branch: repository.branch(),
            checkout_root: repository.root.clone(),
            root,
            kind: ProjectKind::Git,
        };
        return Ok((facts, Some(repository)));
    }
    let folder = if canonical.is_dir() {
        canonical
    } else {
        canonical
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| ResolveError::MissingPath(path.to_path_buf()))?
    };
    Ok((
        ProjectFacts {
            root: folder.clone(),
            checkout_root: folder,
            kind: ProjectKind::Folder,
            branch: None,
            linked_worktree: false,
        },
        None,
    ))
}

/// The durable id of the project at `root` on `device_id`: the same root on
/// two devices is two projects (PRD S5.5 B2).
pub fn project_id(device_id: &str, root: &Path) -> String {
    let material = format!("{}\0{}", device_id, root.to_string_lossy());
    format!("project:{:x}", Sha256::digest(material.as_bytes()))
}

pub fn resolve(path: &Path, device_id: &str) -> Result<ProjectIdentity, ResolveError> {
    if device_id.trim().is_empty() {
        return Err(ResolveError::EmptyDevice);
    }
    let facts = facts(path)?;
    Ok(ProjectIdentity {
        id: project_id(device_id, &facts.root),
        root: facts.root,
        checkout_root: facts.checkout_root,
        device_id: device_id.to_owned(),
        kind: facts.kind,
    })
}

pub mod git {
    use super::ResolveError;
    use serde::{Deserialize, Serialize};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::io::{BufRead, BufReader, Read};
    use std::path::{Path, PathBuf};

    /// The most of `packed-refs` `head_oid` reads.
    const PACKED_REFS_LIMIT: u64 = 8 * 1024 * 1024;
    /// The most of a repository's config read; past it the config is refused
    /// rather than read in part, which also bounds how many branch notes one
    /// answer can carry.
    const CONFIG_LIMIT: u64 = 1024 * 1024;
    /// The most of a pointer file (`HEAD`, a loose ref, `.git`, `gitdir`,
    /// `commondir`), each a line long when Git writes it.
    const POINTER_LIMIT: u64 = 64 * 1024;

    /// The regular file a repository's name resolves to. A repository's files
    /// are written by whatever runs in it, so a pipe or a device is refused
    /// without blocking the reader. A link is followed, as Git follows it: a
    /// config a dotfile manager links in still names its branches. Discovery
    /// still takes only a repository whose `HEAD` is a regular file.
    fn open_file(path: &Path) -> std::io::Result<std::fs::File> {
        hide_platform::fs::open_regular(&std::fs::canonicalize(path)?)
    }

    /// The text of the file `open_file` opens at `path`, refused past `limit`
    /// bytes rather than read whole.
    fn read_small(path: &Path, limit: u64) -> std::io::Result<String> {
        let mut text = String::new();
        open_file(path)?.take(limit + 1).read_to_string(&mut text)?;
        if text.len() as u64 > limit {
            return Err(std::io::Error::new(
                std::io::ErrorKind::FileTooLarge,
                format!("larger than {limit} bytes"),
            ));
        }
        Ok(text)
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct Repository {
        pub root: PathBuf,
        pub git_dir: PathBuf,
        pub common_dir: PathBuf,
    }

    impl Repository {
        pub fn main_root(&self) -> PathBuf {
            self.common_dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.root.clone())
        }

        pub fn branch(&self) -> Option<String> {
            let head = read_small(&self.git_dir.join("HEAD"), POINTER_LIMIT).ok()?;
            let reference = head.trim().strip_prefix("ref:")?.trim();
            let name = reference.strip_prefix("refs/heads/").unwrap_or(reference);
            (!name.is_empty()).then(|| name.to_owned())
        }

        /// The commit HEAD points at, read from Git's own files and never from
        /// a Git process: the detached HEAD's object name, else the commit
        /// the checked-out branch's loose ref holds in the common directory
        /// (where a linked worktree's branches live), else its line in
        /// `packed-refs`. `None` when HEAD cannot be read, the branch has no
        /// commit yet or a ref holds something that is not an object name;
        /// a wrong commit is never made up.
        pub fn head_oid(&self) -> Option<String> {
            let head = read_small(&self.git_dir.join("HEAD"), POINTER_LIMIT).ok()?;
            let head = head.trim();
            let Some(reference) = head.strip_prefix("ref:") else {
                return object_name(head);
            };
            let reference = reference.trim();
            // Git forbids these in a ref name, and `\` and `:` would leave
            // the directory on Windows.
            let safe = reference.starts_with("refs/")
                && !reference
                    .chars()
                    .any(|character| character.is_control() || matches!(character, '\\' | ':'))
                && reference
                    .split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..");
            if !safe {
                return None;
            }
            if let Ok(loose) = read_small(&self.common_dir.join(reference), POINTER_LIMIT) {
                return object_name(loose.trim());
            }
            // Streamed and capped: a repository that has packed many refs
            // can hold a file of megabytes, and a missing line in the first
            // `PACKED_REFS_LIMIT` bytes is no answer rather than a guess.
            let packed = open_file(&self.common_dir.join("packed-refs")).ok()?;
            BufReader::new(packed.take(PACKED_REFS_LIMIT))
                .lines()
                .map_while(Result::ok)
                .filter(|line| !line.starts_with('#') && !line.starts_with('^'))
                .find_map(|line| {
                    let (oid, name) = line.split_once(' ')?;
                    (name.trim() == reference)
                        .then(|| object_name(oid))
                        .flatten()
                })
        }

        pub fn branch_description(&self, branch: &str) -> Result<Option<String>, String> {
            let path = self.common_dir.join("config");
            let text = read_small(&path, CONFIG_LIMIT)
                .map_err(|error| format!("repository config could not be read: {error}"))?;
            Ok(parse_branch_description(&text, branch))
        }

        pub fn branch_issue(&self, branch: &str) -> Result<Option<String>, String> {
            let path = self.common_dir.join("config");
            let text = read_small(&path, CONFIG_LIMIT)
                .map_err(|error| format!("repository config could not be read: {error}"))?;
            Ok(parse_branch_value(&text, branch, "issue"))
        }

        /// The default branch a repository with no remote names itself,
        /// the last step of the default branch rule (PRD git-freshness
        /// D-07): the branch its main worktree has out, which a clone of it
        /// would take as its `origin/HEAD`. `None` unless the repository has
        /// no remote at all (none in a config that names every one, nothing
        /// under `refs/remotes` and no remote-tracking ref packed), the
        /// main worktree is on a
        /// branch rather than a detached HEAD, and that branch holds a
        /// commit. Read from Git's own files, never from a Git process.
        pub fn own_default_branch(&self) -> Option<String> {
            let config = self.config().ok()?;
            if !config.remotes_complete || !config.remotes.is_empty() || self.remote_refs()? {
                return None;
            }
            let main = discover(&self.main_root())?;
            let branch = main.branch()?;
            main.head_oid()?;
            Some(branch)
        }

        /// Whether the repository keeps anything under `refs/remotes`, or a
        /// remote-tracking ref in `packed-refs`; `None` when that cannot be
        /// read whole.
        fn remote_refs(&self) -> Option<bool> {
            match fs::read_dir(self.common_dir.join("refs/remotes")) {
                Ok(mut entries) => {
                    if entries.next().is_some() {
                        return Some(true);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return None,
            }
            let packed = match open_file(&self.common_dir.join("packed-refs")) {
                Ok(packed) => packed,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(false),
                Err(_) => return None,
            };
            let mut read = 0;
            for line in BufReader::new(packed.take(PACKED_REFS_LIMIT + 1)).lines() {
                let line = line.ok()?;
                read += line.len() as u64 + 1;
                if read > PACKED_REFS_LIMIT {
                    return None;
                }
                if line
                    .split_once(' ')
                    .is_some_and(|(_, name)| name.trim().starts_with("refs/remotes/"))
                {
                    return Some(true);
                }
            }
            Some(false)
        }

        /// What the repository's config says, from one read of it: every
        /// branch's note and the URL of every remote.
        pub fn config(&self) -> Result<Config, String> {
            let path = self.common_dir.join("config");
            let text = read_small(&path, CONFIG_LIMIT)
                .map_err(|error| format!("repository config could not be read: {error}"))?;
            let (remotes, remotes_complete) = parse_remotes(&text);
            Ok(Config {
                notes: parse_branch_notes(&text),
                remotes,
                remotes_complete,
            })
        }
    }

    /// One read of a repository's config.
    #[derive(Clone, Debug, Default, Eq, PartialEq)]
    pub struct Config {
        /// Every branch's description and issue, each read as
        /// [`Repository::branch_description`] and [`Repository::branch_issue`]
        /// read one branch; a branch with neither is not listed.
        pub notes: BTreeMap<String, BranchNote>,
        /// Each remote's `url`, in the order the config names them. Remotes
        /// set only outside the repository (the account's `~/.gitconfig`) are
        /// not seen.
        pub remotes: Vec<Remote>,
        /// Whether `remotes` is every remote the repository has: false when
        /// the config includes another file, which may name more, or a
        /// section header never closes (the lines after it are not read).
        pub remotes_complete: bool,
    }

    impl Config {
        /// The first `url` the config's own `origin` remote names, found
        /// whether or not the list is complete: an `include` cannot take away
        /// an origin the file itself names. The name matches exactly, so
        /// `"Origin"` is another remote.
        pub fn origin_url(&self) -> Option<&str> {
            self.remotes
                .iter()
                .find(|remote| remote.name == "origin")
                .map(|remote| remote.url.as_str())
        }

        /// Every remote URL, or `None` when the list may be missing some.
        pub fn remote_urls(&self) -> Option<Vec<String>> {
            self.remotes_complete.then(|| {
                self.remotes
                    .iter()
                    .map(|remote| remote.url.clone())
                    .collect()
            })
        }
    }

    /// One `url` of a remote, by the remote's name.
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct Remote {
        pub name: String,
        pub url: String,
    }

    /// What a repository's config says of one branch.
    #[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
    pub struct BranchNote {
        pub description: Option<String>,
        pub issue: Option<String>,
    }

    pub fn discover(path: &Path) -> Option<Repository> {
        discover_checked(path).ok().flatten()
    }

    pub fn discover_checked(path: &Path) -> Result<Option<Repository>, ResolveError> {
        let start = fs::canonicalize(path).map_err(|error| ResolveError::InvalidGitLink {
            path: path.to_path_buf(),
            reason: format!("discovery_path:{error}"),
        })?;
        let mut directory = if start.is_dir() {
            start.as_path()
        } else {
            start.parent().ok_or_else(|| ResolveError::InvalidGitLink {
                path: start.clone(),
                reason: "path_has_no_parent".to_owned(),
            })?
        };
        loop {
            if let Some((git_dir, common_dir)) = git_dir_at(directory)? {
                return Ok(Some(Repository {
                    root: directory.to_path_buf(),
                    git_dir,
                    common_dir,
                }));
            }
            let Some(parent) = directory.parent() else {
                return Ok(None);
            };
            directory = parent;
        }
    }

    fn git_dir_at(directory: &Path) -> Result<Option<(PathBuf, PathBuf)>, ResolveError> {
        let dotgit = directory.join(".git");
        let metadata = match fs::symlink_metadata(&dotgit) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(invalid(&dotgit, format!("metadata:{error}"))),
        };
        if metadata.file_type().is_symlink() {
            return Err(invalid(&dotgit, "dotgit_symlink"));
        }
        if metadata.is_dir() {
            let candidate = canonical(&dotgit, "git_directory")?;
            require_regular_file(&candidate.join("HEAD"), "head")?;
            return Ok(Some((candidate.clone(), candidate)));
        }
        if !metadata.is_file() {
            return Err(invalid(&dotgit, "dotgit_not_file_or_directory"));
        }

        let text = read_small(&dotgit, POINTER_LIMIT)
            .map_err(|error| invalid(&dotgit, format!("pointer_read:{error}")))?;
        let named = text
            .trim()
            .strip_prefix("gitdir:")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid(&dotgit, "pointer_format"))?;
        let candidate = canonical(&absolute(directory, Path::new(named)), "git_directory")?;
        require_regular_file(&candidate.join("HEAD"), "head")?;
        let common_dir = common_dir_of(&candidate)?;

        let candidate_parent = candidate
            .parent()
            .ok_or_else(|| invalid(&candidate, "worktree_entry_has_no_parent"))?;
        if canonical(candidate_parent, "worktrees_directory")?
            != canonical(
                &common_dir.join("worktrees"),
                "registered_worktrees_directory",
            )?
        {
            return Err(invalid(&dotgit, "unregistered_worktree_entry"));
        }

        let reciprocal = candidate.join("gitdir");
        require_regular_file(&reciprocal, "worktree_gitdir")?;
        let reciprocal_text = read_small(&reciprocal, POINTER_LIMIT)
            .map_err(|error| invalid(&reciprocal, format!("worktree_gitdir_read:{error}")))?;
        let reciprocal_target = canonical(
            &absolute(&candidate, Path::new(reciprocal_text.trim())),
            "worktree_checkout_pointer",
        )?;
        if reciprocal_target != canonical(&directory.join(".git"), "checkout_pointer")? {
            return Err(invalid(&reciprocal, "worktree_pointer_not_reciprocal"));
        }
        Ok(Some((candidate, common_dir)))
    }

    fn common_dir_of(git_dir: &Path) -> Result<PathBuf, ResolveError> {
        let path = git_dir.join("commondir");
        require_regular_file(&path, "commondir")?;
        let text = read_small(&path, POINTER_LIMIT)
            .map_err(|error| invalid(&path, format!("commondir_read:{error}")))?;
        canonical(
            &absolute(git_dir, Path::new(text.trim())),
            "common_directory",
        )
    }

    fn canonical(path: &Path, label: &str) -> Result<PathBuf, ResolveError> {
        fs::canonicalize(path).map_err(|error| invalid(path, format!("{label}:{error}")))
    }

    fn require_regular_file(path: &Path, label: &str) -> Result<(), ResolveError> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|error| invalid(path, format!("{label}_metadata:{error}")))?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(invalid(path, format!("{label}_not_regular_file")));
        }
        Ok(())
    }

    fn invalid(path: &Path, reason: impl Into<String>) -> ResolveError {
        ResolveError::InvalidGitLink {
            path: path.to_path_buf(),
            reason: reason.into(),
        }
    }

    fn absolute(base: &Path, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            base.join(path)
        }
    }

    fn parse_branch_description(config: &str, wanted_branch: &str) -> Option<String> {
        parse_branch_value(config, wanted_branch, "description")
    }

    fn parse_branch_value(config: &str, wanted_branch: &str, wanted_key: &str) -> Option<String> {
        let mut selected = false;
        for raw_line in config.lines() {
            let line = raw_line.trim();
            if line.starts_with('[') {
                selected = parse_branch_section(line).as_deref() == Some(wanted_branch);
                continue;
            }
            if !selected || line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if !key.trim().eq_ignore_ascii_case(wanted_key) {
                continue;
            }
            let value = parse_config_value(value.trim());
            let first = value.lines().next().unwrap_or_default().trim_end();
            return (!first.is_empty()).then(|| first.to_owned());
        }
        None
    }

    /// Each branch's first `description` and first `issue`, in the order
    /// [`parse_branch_value`] would find them.
    fn parse_branch_notes(config: &str) -> BTreeMap<String, BranchNote> {
        let mut notes = BTreeMap::<String, BranchNote>::new();
        let mut seen = BTreeSet::<(String, bool)>::new();
        let mut branch = None;
        for raw_line in config.lines() {
            let line = raw_line.trim();
            if line.starts_with('[') {
                branch = parse_branch_section(line);
                continue;
            }
            let Some(branch) = branch.as_ref() else {
                continue;
            };
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let description = if key.eq_ignore_ascii_case("description") {
                true
            } else if key.eq_ignore_ascii_case("issue") {
                false
            } else {
                continue;
            };
            if !seen.insert((branch.clone(), description)) {
                continue;
            }
            let value = parse_config_value(value.trim());
            let first = value.lines().next().unwrap_or_default().trim_end();
            let value = (!first.is_empty()).then(|| first.to_owned());
            let note = notes.entry(branch.clone()).or_default();
            if description {
                note.description = value;
            } else {
                note.issue = value;
            }
        }
        notes.retain(|_, note| note.description.is_some() || note.issue.is_some());
        notes
    }

    /// Each `remote.<name>.url` value, and whether the list is complete: not
    /// when an `include` or `includeIf` section may bring in more (the rest of
    /// the file is still read), nor when a section header never closes (the
    /// read stops there). Section names are matched without case, in both
    /// `[remote "origin"]` and the older `[remote.origin]` spelling, and a
    /// key may follow its header on the same line, as Git reads it.
    fn parse_remotes(config: &str) -> (Vec<Remote>, bool) {
        let mut remotes = Vec::new();
        let mut complete = true;
        let mut name: Option<String> = None;
        for raw_line in config.lines() {
            let mut line = raw_line.trim();
            if let Some(body) = line.strip_prefix('[') {
                let Some((header, rest)) = split_section_header(body) else {
                    return (remotes, false);
                };
                let section_end = header
                    .find(|character: char| {
                        character.is_whitespace() || matches!(character, '"' | '.')
                    })
                    .unwrap_or(header.len());
                let (section, subsection) = header.split_at(section_end);
                if section.eq_ignore_ascii_case("include")
                    || section.eq_ignore_ascii_case("includeIf")
                {
                    complete = false;
                }
                name = section
                    .eq_ignore_ascii_case("remote")
                    .then(|| remote_name(subsection.trim()))
                    .flatten();
                line = rest.trim();
            }
            let Some(remote) = &name else {
                continue;
            };
            if line.is_empty() || line.starts_with(['#', ';']) {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim().eq_ignore_ascii_case("url") {
                let value = parse_config_value(value.trim());
                if !value.is_empty() {
                    remotes.push(Remote {
                        name: remote.clone(),
                        url: value,
                    });
                }
            }
        }
        (remotes, complete)
    }

    /// The remote a `[remote ...]` header names: the quoted subsection as
    /// written, or, in the older `[remote.name]` spelling, the name in lower
    /// case as Git folds it.
    fn remote_name(subsection: &str) -> Option<String> {
        match subsection.strip_prefix('.') {
            Some(name) => Some(name.to_ascii_lowercase()),
            None => parse_quoted(subsection),
        }
    }

    /// A section header after its `[`: the text up to the `]` outside its
    /// quoted subsection name, and what follows on the line; `None` for a
    /// header that never closes.
    fn split_section_header(body: &str) -> Option<(&str, &str)> {
        let mut quoted = false;
        let mut escaped = false;
        for (index, character) in body.char_indices() {
            match character {
                _ if escaped => escaped = false,
                '\\' if quoted => escaped = true,
                '"' => quoted = !quoted,
                ']' if !quoted => return Some((&body[..index], &body[index + 1..])),
                _ => {}
            }
        }
        None
    }

    fn parse_branch_section(line: &str) -> Option<String> {
        let body = line.strip_prefix('[')?.strip_suffix(']')?.trim();
        let (section, rest) = body.split_once(char::is_whitespace)?;
        section.eq_ignore_ascii_case("branch").then_some(())?;
        parse_quoted(rest.trim())
    }

    fn parse_config_value(value: &str) -> String {
        if value.starts_with('"') {
            parse_quoted(value).unwrap_or_default()
        } else {
            decode_config_escapes(
                value
                    .split(['#', ';'])
                    .next()
                    .unwrap_or_default()
                    .trim_end(),
            )
        }
    }

    fn decode_config_escapes(input: &str) -> String {
        let mut value = String::new();
        let mut escaped = false;
        for character in input.chars() {
            if escaped {
                value.push(match character {
                    'n' => '\n',
                    't' => '\t',
                    'b' => '\u{0008}',
                    other => other,
                });
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else {
                value.push(character);
            }
        }
        if escaped {
            value.push('\\');
        }
        value
    }

    /// A full object name: 40 hexadecimal digits (SHA-1) or 64 (SHA-256).
    fn object_name(text: &str) -> Option<String> {
        let valid =
            matches!(text.len(), 40 | 64) && text.bytes().all(|byte| byte.is_ascii_hexdigit());
        valid.then(|| text.to_ascii_lowercase())
    }

    fn parse_quoted(input: &str) -> Option<String> {
        let mut chars = input.chars();
        (chars.next()? == '"').then_some(())?;
        let mut value = String::new();
        let mut escaped = false;
        for character in chars {
            if escaped {
                value.push(match character {
                    'n' => '\n',
                    't' => '\t',
                    'b' => '\u{0008}',
                    other => other,
                });
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                return Some(value);
            } else {
                value.push(character);
            }
        }
        None
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const FIRST: &str = "1111111111111111111111111111111111111111";
        const SECOND: &str = "2222222222222222222222222222222222222222";

        /// A repository folder laid out as Git leaves it, with HEAD on `head`.
        fn repository(head: &str) -> (tempfile::TempDir, Repository) {
            let temp = tempfile::tempdir().unwrap();
            let git_dir = temp.path().join(".git");
            fs::create_dir_all(git_dir.join("refs/heads")).unwrap();
            fs::write(git_dir.join("HEAD"), format!("{head}\n")).unwrap();
            let repository = Repository {
                root: temp.path().to_path_buf(),
                common_dir: git_dir.clone(),
                git_dir,
            };
            (temp, repository)
        }

        #[test]
        fn head_is_the_commit_in_the_branchs_loose_ref() {
            let (_temp, repository) = repository("ref: refs/heads/topic");
            fs::write(
                repository.common_dir.join("refs/heads/topic"),
                format!("{FIRST}\n"),
            )
            .unwrap();
            assert_eq!(repository.head_oid().as_deref(), Some(FIRST));
        }

        #[test]
        fn head_falls_back_to_packed_refs_when_the_ref_is_not_loose() {
            let (_temp, repository) = repository("ref: refs/heads/topic");
            fs::write(
                repository.common_dir.join("packed-refs"),
                format!(
                    "# pack-refs with: peeled fully-peeled sorted\n{SECOND} refs/heads/other\n{FIRST} refs/heads/topic\n^{SECOND}\n"
                ),
            )
            .unwrap();
            assert_eq!(repository.head_oid().as_deref(), Some(FIRST));
        }

        #[test]
        fn a_loose_ref_wins_over_an_older_packed_one() {
            let (_temp, repository) = repository("ref: refs/heads/topic");
            fs::write(
                repository.common_dir.join("packed-refs"),
                format!("{FIRST} refs/heads/topic\n"),
            )
            .unwrap();
            fs::write(repository.common_dir.join("refs/heads/topic"), SECOND).unwrap();
            assert_eq!(repository.head_oid().as_deref(), Some(SECOND));
        }

        /// Whatever runs in a repository writes its files, so a pipe at the
        /// config's name answers at once as unreadable instead of holding
        /// the reader until something writes into it, and an outsized
        /// config is refused rather than read whole.
        #[cfg(unix)]
        #[test]
        fn a_config_that_is_a_pipe_or_outsized_is_refused_at_once() {
            let (_temp, repository) = repository("ref: refs/heads/main");
            let config = repository.common_dir.join("config");
            let made = std::process::Command::new("mkfifo")
                .arg(&config)
                .status()
                .unwrap();
            assert!(made.success());
            assert!(repository.config().is_err());

            fs::remove_file(&config).unwrap();
            fs::write(&config, vec![b'#'; CONFIG_LIMIT as usize + 1]).unwrap();
            assert!(repository.config().is_err());
            fs::write(&config, "[branch \"main\"]\n\tdescription = kept\n").unwrap();
            assert_eq!(
                repository.config().unwrap().notes["main"]
                    .description
                    .as_deref(),
                Some("kept")
            );
        }

        /// A linked branch ref and config are followed, as Git follows them.
        #[cfg(unix)]
        #[test]
        fn a_linked_ref_and_config_are_read_through_the_link() {
            let (temp, repository) = repository("ref: refs/heads/main");
            let elsewhere = temp.path().join("dotfiles");
            fs::create_dir_all(&elsewhere).unwrap();
            for (name, text) in [
                ("refs/heads/main", FIRST),
                ("config", "[branch \"main\"]\n\tdescription = linked\n"),
            ] {
                let target = elsewhere.join(name.replace('/', "-"));
                fs::write(&target, text).unwrap();
                let at = repository.common_dir.join(name);
                fs::remove_file(&at).ok();
                std::os::unix::fs::symlink(&target, &at).unwrap();
            }
            assert_eq!(repository.head_oid().as_deref(), Some(FIRST));
            assert_eq!(
                repository.config().unwrap().notes["main"]
                    .description
                    .as_deref(),
                Some("linked")
            );
        }

        /// A `HEAD` that becomes a link after discovery is read through it,
        /// as Git reads it, for the branch and its commit alike.
        #[cfg(unix)]
        #[test]
        fn a_head_linked_after_discovery_is_read_through_the_link() {
            let (temp, repository) = repository("ref: refs/heads/main");
            fs::write(repository.common_dir.join("refs/heads/topic"), SECOND).unwrap();
            let target = temp.path().join("linked-HEAD");
            fs::write(&target, "ref: refs/heads/topic\n").unwrap();
            let head = repository.git_dir.join("HEAD");
            fs::remove_file(&head).unwrap();
            std::os::unix::fs::symlink(&target, &head).unwrap();
            assert_eq!(repository.branch().as_deref(), Some("topic"));
            assert_eq!(repository.head_oid().as_deref(), Some(SECOND));
        }

        #[test]
        fn a_detached_head_is_its_own_commit() {
            let (_temp, repository) = repository(FIRST);
            assert_eq!(repository.head_oid().as_deref(), Some(FIRST));
        }

        #[test]
        fn a_linked_worktree_reads_its_branch_from_the_common_directory() {
            let (temp, mut repository) = repository("ref: refs/heads/main");
            fs::write(repository.common_dir.join("refs/heads/feature"), FIRST).unwrap();
            let linked = repository.common_dir.join("worktrees/feature");
            fs::create_dir_all(&linked).unwrap();
            fs::write(linked.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
            repository.root = temp.path().join("feature-checkout");
            repository.git_dir = linked;
            assert_ne!(repository.git_dir, repository.common_dir);
            assert_eq!(repository.head_oid().as_deref(), Some(FIRST));
        }

        #[test]
        fn a_branch_with_no_commit_or_a_damaged_ref_has_no_head() {
            let (_temp, repository) = repository("ref: refs/heads/unborn");
            assert_eq!(
                repository.head_oid(),
                None,
                "no loose ref and no packed-refs"
            );
            fs::write(
                repository.common_dir.join("packed-refs"),
                format!("{FIRST} refs/heads/other\n"),
            )
            .unwrap();
            assert_eq!(repository.head_oid(), None, "not in packed-refs either");
            fs::write(
                repository.common_dir.join("refs/heads/unborn"),
                "not a commit\n",
            )
            .unwrap();
            assert_eq!(repository.head_oid(), None, "a ref that is no object name");
            fs::write(repository.common_dir.join("escape"), format!("{FIRST}\n")).unwrap();
            fs::write(
                repository.git_dir.join("HEAD"),
                "ref: refs/heads/../../escape\n",
            )
            .unwrap();
            assert_eq!(
                repository.head_oid(),
                None,
                "a ref that leaves the repository"
            );
            fs::remove_file(repository.git_dir.join("HEAD")).unwrap();
            assert_eq!(repository.head_oid(), None, "no HEAD");
        }

        /// Review finding 14: a repository without a remote names its own
        /// default, the branch its main worktree has out once it holds a
        /// commit, whatever that branch is called; a remote in the config,
        /// one an include may name, a remote-tracking ref or a detached
        /// HEAD leaves it unnamed.
        #[test]
        fn a_repository_with_no_remote_names_its_main_worktrees_branch_its_default() {
            let (_temp, repository) = repository("ref: refs/heads/trunk");
            let config = repository.common_dir.join("config");
            let plain = "[core]\n\tbare = false\n";
            fs::write(&config, plain).unwrap();
            assert_eq!(repository.own_default_branch(), None, "no commit yet");
            fs::write(repository.common_dir.join("refs/heads/trunk"), FIRST).unwrap();
            assert_eq!(repository.own_default_branch().as_deref(), Some("trunk"));

            let packed = repository.common_dir.join("packed-refs");
            fs::write(&packed, format!("{SECOND} refs/remotes/origin/trunk\n")).unwrap();
            assert_eq!(repository.own_default_branch(), None, "a packed remote ref");
            fs::write(&packed, format!("{SECOND} refs/tags/v1\n")).unwrap();
            assert_eq!(repository.own_default_branch().as_deref(), Some("trunk"));
            let remotes = repository.common_dir.join("refs/remotes/backup");
            fs::create_dir_all(&remotes).unwrap();
            fs::write(remotes.join("trunk"), SECOND).unwrap();
            assert_eq!(repository.own_default_branch(), None, "a loose remote ref");
            fs::remove_dir_all(repository.common_dir.join("refs/remotes")).unwrap();

            for (text, why) in [
                ("[remote \"backup\"]\n\turl = /srv/backup.git\n", "a remote"),
                ("[include]\n\tpath = more.config\n", "an include"),
            ] {
                fs::write(&config, format!("{plain}{text}")).unwrap();
                assert_eq!(repository.own_default_branch(), None, "{why}");
            }
            fs::write(&config, plain).unwrap();
            fs::write(repository.git_dir.join("HEAD"), format!("{FIRST}\n")).unwrap();
            assert_eq!(repository.own_default_branch(), None, "a detached HEAD");
        }

        #[test]
        fn branch_issue_reads_the_first_config_line() {
            let config = concat!(
                "[branch \"topic\"]\n",
                "\tissue = \"owner/repository#42\"\n",
                "\tdescription = purpose\n",
            );

            assert_eq!(
                parse_branch_value(config, "topic", "issue").as_deref(),
                Some("owner/repository#42")
            );
        }

        #[test]
        fn a_config_names_its_remotes_urls_and_an_include_or_open_header_leaves_them_unknown() {
            let urls = |config: &str| {
                let (remotes, complete) = parse_remotes(config);
                complete.then(|| {
                    remotes
                        .into_iter()
                        .map(|remote| remote.url)
                        .collect::<Vec<_>>()
                })
            };
            let config = concat!(
                "[core]\n\tbare = false\n",
                "[remote \"origin\"]\n\turl = git@github.com:acme/app.git\n",
                "\tfetch = +refs/heads/*:refs/remotes/origin/*\n",
                "[Remote.upstream]\n\tURL = \"https://github.com/acme/upstream\" # kept\n",
                "[branch \"main\"]\n\tremote = origin\n",
                "[remote \"a]b\"] url = https://github.com/acme/inline\n",
            );
            assert_eq!(
                urls(config),
                Some(vec![
                    "git@github.com:acme/app.git".to_owned(),
                    "https://github.com/acme/upstream".to_owned(),
                    "https://github.com/acme/inline".to_owned(),
                ])
            );
            assert_eq!(urls("[core]\n\tbare = false\n"), Some(Vec::new()));
            for include in [
                "[include]\n\tpath = more",
                "[includeIf \"gitdir:~/\"]\n\tpath = more",
                "[remote \"origin\"\n\turl = https://github.com/acme/app",
            ] {
                assert_eq!(urls(include), None, "{include}");
            }
        }

        fn origin(config: &str) -> Option<String> {
            let (remotes, remotes_complete) = parse_remotes(config);
            Config {
                remotes,
                remotes_complete,
                ..Config::default()
            }
            .origin_url()
            .map(str::to_owned)
        }

        /// The origin is found as Git spells it, by its exact name, and an
        /// `include` cannot hide an origin the file itself names.
        #[test]
        fn the_origin_url_is_the_origin_remotes_own_url() {
            let config = "[core]\n\turl = not-this\n[remote \"upstream\"]\n\turl = git@github.com:other/app.git\n[remote \"origin\"]\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n\turl = git@github.com:acme/app.git\n";
            assert_eq!(
                origin(config).as_deref(),
                Some("git@github.com:acme/app.git")
            );
            assert_eq!(origin("[remote \"upstream\"]\n\turl = x\n"), None);
            assert_eq!(
                origin("[Remote  \"origin\"]\n# a note\n\tURL = git@github.com:acme/app.git\n")
                    .as_deref(),
                Some("git@github.com:acme/app.git")
            );
            assert_eq!(origin("[remote \"Origin\"]\n\turl = x\n"), None);
            assert_eq!(origin("[remote.origin]\n\turl = y\n").as_deref(), Some("y"));
            let included = "[include]\n\tpath = more\n[remote \"origin\"]\n\turl = z\n";
            assert_eq!(origin(included).as_deref(), Some("z"));
            assert!(!parse_remotes(included).1, "the list is still unknown");
        }

        /// One pass over the config reads every branch as the one-branch
        /// readers do: the first value of each key wins, an empty first
        /// value is no value, and another section's keys are not a branch's.
        #[test]
        fn branch_notes_read_every_branch_as_the_one_branch_readers_do() {
            let config = concat!(
                "[core]\n\tdescription = not a branch\n",
                "[branch \"topic\"]\n\tissue = \"owner/repository#42\"\n\tdescription = purpose\n",
                "[branch \"empty\"]\n\tdescription =\n\tdescription = later\n",
                "[branch \"topic\"]\n\tdescription = second\n",
                "[branch \"plain\"]\n\tmerge = refs/heads/plain\n",
            );
            let notes = parse_branch_notes(config);
            for branch in ["topic", "empty", "plain"] {
                let note = notes.get(branch).cloned().unwrap_or_default();
                assert_eq!(
                    note.description,
                    parse_branch_description(config, branch),
                    "{branch}"
                );
                assert_eq!(
                    note.issue,
                    parse_branch_value(config, branch, "issue"),
                    "{branch}"
                );
            }
            assert_eq!(notes.keys().collect::<Vec<_>>(), ["topic"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn linked_worktrees_share_one_project_identity() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let linked = temp.path().join("linked");
        fs::create_dir(&main).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(&main)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args(["config", "user.email", "test@example.com"])
                .current_dir(&main)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args(["config", "user.name", "Test"])
                .current_dir(&main)
                .status()
                .unwrap()
                .success()
        );
        fs::write(main.join("README.md"), "test\n").unwrap();
        assert!(
            Command::new("git")
                .args(["add", "."])
                .current_dir(&main)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args(["-c", "commit.gpgsign=false", "commit", "-qm", "init"])
                .current_dir(&main)
                .status()
                .unwrap()
                .success()
        );
        assert!(
            Command::new("git")
                .args([
                    "worktree",
                    "add",
                    "-q",
                    "-b",
                    "linked",
                    linked.to_str().unwrap()
                ])
                .current_dir(&main)
                .status()
                .unwrap()
                .success()
        );
        let main_id = resolve(&main, "local").unwrap();
        let linked_id = resolve(&linked, "local").unwrap();
        assert_eq!(main_id.id, linked_id.id);
        assert_eq!(main_id.root, linked_id.root);
        assert_eq!(main_id.checkout_root, fs::canonicalize(&main).unwrap());
        assert_eq!(linked_id.checkout_root, fs::canonicalize(&linked).unwrap());
        assert_eq!(linked_id.kind, ProjectKind::Git);
    }

    #[test]
    fn device_is_part_of_identity_and_missing_paths_never_fallback() {
        let temp = tempfile::tempdir().unwrap();
        assert_ne!(
            resolve(temp.path(), "local").unwrap().id,
            resolve(temp.path(), "mini").unwrap().id
        );
        assert!(matches!(
            resolve(&temp.path().join("missing"), "local"),
            Err(ResolveError::MissingPath(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn dotgit_symlink_is_rejected_instead_of_aliasing_another_project() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let victim = temp.path().join("victim");
        let attacker = temp.path().join("attacker");
        fs::create_dir(&victim).unwrap();
        fs::create_dir(&attacker).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(&victim)
                .status()
                .unwrap()
                .success()
        );
        symlink(victim.join(".git"), attacker.join(".git")).unwrap();

        assert!(matches!(
            resolve(&attacker, "local"),
            Err(ResolveError::InvalidGitLink { reason, .. }) if reason == "dotgit_symlink"
        ));
    }

    #[test]
    fn forged_git_pointer_without_reciprocal_worktree_registration_is_rejected() {
        let temp = tempfile::tempdir().unwrap();
        let victim = temp.path().join("victim");
        let attacker = temp.path().join("attacker");
        fs::create_dir(&victim).unwrap();
        fs::create_dir(&attacker).unwrap();
        assert!(
            Command::new("git")
                .args(["init", "-q"])
                .current_dir(&victim)
                .status()
                .unwrap()
                .success()
        );
        fs::write(
            attacker.join(".git"),
            format!("gitdir: {}\n", victim.join(".git").display()),
        )
        .unwrap();

        assert!(matches!(
            resolve(&attacker, "local"),
            Err(ResolveError::InvalidGitLink { reason, .. })
                if reason.contains("commondir_metadata")
        ));
    }
}
