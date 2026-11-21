//! A repository clone as the core asks a node for it: which URLs are
//! accepted and the folder name a URL gives, what the node reports while Git
//! runs, and how the clone ended. A URL can carry credentials, so it is never
//! printed: [`CloneSource`]'s `Debug` shows its host only, and no message
//! here names it.

use std::path::PathBuf;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A URL Git can clone from, and the folder name it gives.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CloneSource {
    url: String,
    host: String,
    name: String,
}

impl std::fmt::Debug for CloneSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The URL can carry a token; only its host is printed.
        f.debug_struct("CloneSource")
            .field("host", &self.host)
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl CloneSource {
    /// Accepts `https://host/path`, `ssh://[user@]host[:port]/path`, the scp
    /// form `[user@]host:path`, and `file:///path`, and names the folder the
    /// way `git clone` does: the last path segment without a trailing `.git`.
    /// Anything else is refused with the reason in plain words.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let url = raw.trim();
        if url.is_empty() {
            return Err("Enter a Git URL.".to_owned());
        }
        // `::` is Git's transport-helper form (`ext::`, `fd::`), which no
        // accepted URL spells; `GIT_ALLOW_PROTOCOL` refuses it too, but it
        // is not offered as a clone in the first place.
        if url.starts_with('-')
            || url.contains("::")
            || url.chars().any(|c| c.is_whitespace() || c.is_control())
        {
            return Err("This is not a Git URL.".to_owned());
        }
        let (host, path) = if let Some(rest) = url.strip_prefix("https://") {
            split_authority(rest)?
        } else if let Some(rest) = url.strip_prefix("ssh://") {
            split_authority(rest)?
        } else if let Some(rest) = url.strip_prefix("file://") {
            if !rest.starts_with('/') {
                return Err(
                    "A file URL names an absolute path: file:///path/to/repo.git".to_owned(),
                );
            }
            ("localhost".to_owned(), rest.to_owned())
        } else if url.contains("://") {
            return Err("Use an https, ssh or git@host:path URL.".to_owned());
        } else {
            split_scp(url)?
        };
        let name = folder_name(&path)
            .ok_or_else(|| "This URL names no repository to clone.".to_owned())?;
        Ok(Self {
            url: url.to_owned(),
            host,
            name,
        })
    }

    /// The URL as given, credentials included; for Git only, never a log.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The host the URL names, without user or credentials: what a log line
    /// may carry.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The folder the clone lands in under its parent.
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// `[user[:secret]@]host[:port]/path` after the scheme.
fn split_authority(rest: &str) -> Result<(String, String), String> {
    let (authority, path) = rest
        .split_once('/')
        .ok_or_else(|| "This URL names no repository to clone.".to_owned())?;
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = host_port
        .rsplit_once(':')
        .filter(|(_, port)| port.chars().all(|c| c.is_ascii_digit()))
        .map_or(host_port, |(host, _)| host);
    if host.is_empty() || !valid_host(host) {
        return Err("This URL names no host.".to_owned());
    }
    Ok((host.to_ascii_lowercase(), path.to_owned()))
}

/// The scp form `[user@]host:path`, which Git reads as ssh.
fn split_scp(url: &str) -> Result<(String, String), String> {
    let (authority, path) = url
        .split_once(':')
        .ok_or_else(|| "Use an https, ssh or git@host:path URL.".to_owned())?;
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    // A slash before the colon is a local path, which Git would read as one.
    if host.is_empty() || authority.contains('/') || !valid_host(host) {
        return Err("Use an https, ssh or git@host:path URL.".to_owned());
    }
    Ok((host.to_ascii_lowercase(), path.to_owned()))
}

fn valid_host(host: &str) -> bool {
    host.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '[' | ']' | ':'))
}

/// Git's own naming: the last segment, trailing slashes and `.git` dropped.
fn folder_name(path: &str) -> Option<String> {
    let path = path.trim_end_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    let name = path.rsplit(['/', ':']).next()?;
    (!name.is_empty() && name != "." && name != ".." && !name.contains('\\'))
        .then(|| name.to_owned())
}

/// Where Git says it is, as the progress line the operator reads.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloneProgress {
    /// Git's own stage name: `Receiving objects`, `Resolving deltas`, ...
    pub stage: String,
    pub percent: Option<u8>,
}

/// Why a clone ended without a repository. Each reads as one sentence the
/// operator can act on; none carries the URL.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloneFailure {
    Cancelled,
    /// Git printed nothing for [`STALL_LIMIT`].
    Stalled(Duration),
    /// The host wanted credentials Git could not find without asking.
    Authentication,
    /// `ssh` does not know the host's key yet and would not ask.
    UnknownHostKey,
    NotFound,
    UnreachableHost,
    /// Something is already at the target's name.
    TargetExists(PathBuf),
    /// Git finished, but by then the target's path no longer named the folder
    /// it cloned into: it, or a folder above it, was moved or replaced. The
    /// clone stays where it went and nothing is registered.
    Moved(PathBuf),
    /// Git failed otherwise; its last `fatal:` line, credentials removed.
    Git(String),
    /// The run itself could not start or finish its own work.
    Io(String),
}

impl CloneFailure {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Stalled(_) => "stalled",
            Self::Authentication => "authentication",
            Self::UnknownHostKey => "unknown_host_key",
            Self::NotFound => "not_found",
            Self::UnreachableHost => "unreachable_host",
            Self::TargetExists(_) => "target_exists",
            Self::Moved(_) => "moved",
            Self::Git(_) => "git",
            Self::Io(_) => "io",
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Cancelled => "The clone was cancelled.".to_owned(),
            Self::Stalled(limit) => format!(
                "The transfer stalled: nothing arrived for {} seconds, so the clone was stopped.",
                limit.as_secs()
            ),
            Self::Authentication => "Authentication failed: this repository needs credentials Git could not find. Set up an SSH key or a credential helper, then try again.".to_owned(),
            Self::UnknownHostKey => "This host's SSH key is not known yet. Connect to it once from a terminal to trust it, then try again.".to_owned(),
            Self::NotFound => "The repository was not found, or it needs sign-in.".to_owned(),
            Self::UnreachableHost => "The host could not be reached.".to_owned(),
            Self::TargetExists(path) => format!("{} already exists.", path.display()),
            Self::Moved(path) => format!(
                "{} was moved or replaced while it was cloned; the clone was left where it went and nothing was added.",
                path.display()
            ),
            Self::Git(line) => line.clone(),
            Self::Io(reason) => reason.clone(),
        }
    }
}

/// A node judges the URL again when it reads one off the wire.
impl TryFrom<String> for CloneSource {
    type Error = String;

    fn try_from(url: String) -> Result<Self, String> {
        Self::parse(&url)
    }
}

impl From<CloneSource> for String {
    fn from(source: CloneSource) -> Self {
        source.url
    }
}

/// What the node reports while the clone runs, at least once a second; the
/// caller answers each report with whether the clone should go on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "report", rename_all = "snake_case")]
pub enum CloneReport {
    /// Git named a new stage or percent.
    Progress { progress: CloneProgress },
    /// Nothing new; the clone is still running.
    Running,
}

/// How the clone ended: the new folder, or why there is none.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum CloneAnswer {
    Cloned { path: PathBuf },
    Failed { failure: CloneFailure },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(url: &str) -> Result<(String, String), String> {
        CloneSource::parse(url).map(|source| (source.host().to_owned(), source.name().to_owned()))
    }

    #[test]
    fn accepted_urls_name_their_folder_the_way_git_does() {
        let ok = |url: &str, host: &str, folder: &str| {
            assert_eq!(name(url), Ok((host.to_owned(), folder.to_owned())), "{url}");
        };
        ok("https://github.com/user/repo.git", "github.com", "repo");
        ok("https://github.com/user/repo", "github.com", "repo");
        ok("https://github.com/user/repo/", "github.com", "repo");
        ok("  https://GitHub.com/user/repo.git\n", "github.com", "repo");
        ok(
            "https://user:token@github.com:8443/org/repo.git",
            "github.com",
            "repo",
        );
        ok("git@github.com:user/repo.git", "github.com", "repo");
        ok("git@github.com:repo.git", "github.com", "repo");
        ok("github.com:user/.dotfiles", "github.com", ".dotfiles");
        ok(
            "ssh://git@example.com:2222/srv/repo.git",
            "example.com",
            "repo",
        );
        ok("ssh://example.com/repo.git/", "example.com", "repo");
        ok("file:///tmp/fixtures/origin.git", "localhost", "origin");
    }

    #[test]
    fn refused_urls_say_why() {
        for url in [
            "",
            "   ",
            "http://example.com/repo.git",
            "git://example.com/repo.git",
            "ext::sh -c touch% /tmp/pwned",
            "ext::true",
            "fd::3",
            "-uhttps://example.com/x",
            "https://github.com",
            "https://github.com/",
            "https:///repo.git",
            "/home/me/repo",
            "./repo",
            "repo",
            "git@github.com:",
            "git@github.com:.git",
            "file://relative/repo",
            "https://github.com/user/repo name",
        ] {
            assert!(CloneSource::parse(url).is_err(), "{url:?} was accepted");
        }
    }

    #[test]
    fn debug_never_prints_the_url() {
        let source = CloneSource::parse("https://user:s3cret@github.com/org/repo.git").unwrap();
        let printed = format!("{source:?}");
        assert!(!printed.contains("s3cret"), "{printed}");
        assert!(printed.contains("github.com"), "{printed}");
    }

    #[test]
    fn a_source_read_off_the_wire_is_judged_again() {
        let sent =
            serde_json::to_string(&CloneSource::parse("git@github.com:org/repo.git").unwrap())
                .unwrap();
        let read: CloneSource = serde_json::from_str(&sent).unwrap();
        assert_eq!(read.name(), "repo");
        assert!(serde_json::from_str::<CloneSource>("\"ext::sh -c true\"").is_err());
    }
}
