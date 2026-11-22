//! The active checkout's header reads, driven by the existing Changes pump.
//! Two single-flight workers keep Git independent of a slow GitHub request.
use std::time::Duration;

use hide_node_link::protocol::Call;
use hide_node_link::worktrees::CheckoutGit;
use serde::{Deserialize, Serialize};

use crate::changes::ChannelRef;
use crate::files::DocumentRoot;
use crate::node_access::call_as;
use crate::observed::Observation;
use crate::reader::BackgroundRead;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct GitRequest {
    pub root: DocumentRoot,
    pub channel: Result<ChannelRef, String>,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CiRequest {
    pub git: GitRequest,
    /// The host that reads the checks, and the repository the commits are in.
    pub host: crate::code_host::Host,
    pub repository: crate::code_host::RepositoryRef,
    pub sha: String,
    pub commits: Vec<String>,
    pub channel: ChannelRef,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Header {
    pub state: GitState,
    pub facts: Option<CheckoutGit>,
    /// The Git read behind `facts`: when it last succeeded and whether the
    /// last read did (PRD git-freshness D-09); absent before the first.
    pub observed: Option<Observation>,
    pub reason: Option<String>,
    pub ci: Option<Ci>,
    pub recent_ci: Vec<Ci>,
    /// What the background fetch last saw of the remote, filled when the
    /// header is projected (`Runtime::remote_check`).
    pub remote: Option<crate::runtime::RemoteCheck>,
    /// The GitHub page of each recent or incoming commit a remote holds,
    /// made by the core from the validated repository (PRD git-freshness
    /// D-05); the renderer never receives a remote URL.
    pub commit_links: Vec<CommitLink>,
    /// The checkout's origin is a GitHub repository, so a commit with no
    /// page in `commit_links` is one no remote holds (`푸시 안 됨`).
    pub commit_pages: bool,
    /// The operator's `받기` on this checkout while it runs or after it
    /// was refused, filled when the header is projected; absent otherwise.
    pub catch_up: Option<CatchUpView>,
}

/// What the screen shows of a catch-up: running until the Git read shows
/// the new `HEAD`, refused with the count of the operator's files in the way,
/// refused while a merge, rebase, cherry-pick or revert is in progress, or
/// failed for a reason the log keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum CatchUpView {
    Running,
    Refused { files: u32 },
    Busy,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct CommitLink {
    pub sha: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GitState {
    Loading,
    Fresh,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Ci {
    pub sha: String,
    /// The commit's result as GitHub last answered it; a failed or aged read
    /// keeps it and says so in `observed` (D-09, B22).
    pub state: CiState,
    pub observed: Option<Observation>,
    pub url: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CiState {
    Loading,
    Passing,
    Pending,
    Failed,
    None,
    Unknown,
    /// GitHub never answered for this commit.
    Unavailable,
}

pub(crate) type GitAnswer = (GitRequest, Result<CheckoutGit, String>);
/// The commits' results, or why GitHub could not be read.
pub(crate) type CiAnswer = (CiRequest, Result<Vec<Ci>, String>);

pub(crate) fn git_reader() -> BackgroundRead<Option<GitRequest>, Option<GitAnswer>> {
    BackgroundRead::new(
        Duration::from_secs(2),
        Duration::ZERO,
        |request: &Option<GitRequest>| {
            request.as_ref().map(|request| {
                let facts = request
                    .channel
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|channel| {
                        call_as(
                            &*channel.0,
                            Call::CheckoutGit {
                                path: request.root.path.clone(),
                            },
                            Duration::from_secs(4),
                        )
                        .map_err(|error| error.to_string())
                    });
                (request.clone(), facts)
            })
        },
    )
}

pub(crate) fn ci_reader() -> BackgroundRead<Option<CiRequest>, Option<CiAnswer>> {
    BackgroundRead::new(
        Duration::from_secs(30),
        Duration::ZERO,
        |request: &Option<CiRequest>| {
            request
                .as_ref()
                .map(|request| (request.clone(), read_ci(request)))
        },
    )
}

fn read_ci(request: &CiRequest) -> Result<Vec<Ci>, String> {
    request
        .host
        .commit_checks()
        .ok_or_else(|| "The code host reads no commit checks".to_owned())
        .and_then(|checks| {
            checks.read(&*request.channel.0, &request.repository, &request.commits)
        })
        .inspect_err(|reason| {
            crate::diagnostic!(
                serde_json::json!({"component":"checkout_git", "kind":"ci.read_failed", "sha":request.sha, "reason":reason})
            );
        })
}
