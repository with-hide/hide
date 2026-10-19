//! The active checkout's header reads, driven by the existing Changes pump.
//! Two single-flight workers keep Git independent of a slow GitHub request.
use std::time::Duration;

use hide_node_link::gh::{ChecksList, GhAnswer, checks_summary_arguments};
use hide_node_link::protocol::Call;
use hide_node_link::worktrees::CheckoutGit;
use serde::{Deserialize, Serialize};

use crate::changes::ChannelRef;
use crate::files::DocumentRoot;
use crate::node_access::call_as;
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
    pub repository: String,
    pub sha: String,
    pub channel: ChannelRef,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Header {
    pub state: GitState,
    pub facts: Option<CheckoutGit>,
    pub reason: Option<String>,
    pub ci: Option<Ci>,
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
    pub state: CiState,
    pub checked_at_unix_ms: Option<u64>,
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
    Unavailable,
    Stale,
}

impl CiState {
    fn combine(a: Self, b: Self) -> Self {
        [Self::Failed, Self::Pending, Self::Unknown, Self::Passing]
            .into_iter()
            .find(|state| *state == a || *state == b)
            .unwrap_or(Self::None)
    }
}

pub(crate) type GitAnswer = (GitRequest, Result<CheckoutGit, String>);
pub(crate) type CiAnswer = (CiRequest, Ci);

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

#[derive(Deserialize)]
struct CheckSummary {
    state: CiState,
    url: Option<String>,
}

fn read_ci(request: &CiRequest) -> Ci {
    let read = |list| -> Result<CheckSummary, String> {
        let args = checks_summary_arguments(&request.repository, &request.sha, list)
            .ok_or("Invalid exact-SHA check target")?;
        let answer: GhAnswer = call_as(
            &*request.channel.0,
            Call::Gh {
                cwd: None,
                args,
                repository: Some(request.repository.clone()),
            },
            Duration::from_secs(20),
        )
        .map_err(|error| error.to_string())?;
        match answer {
            GhAnswer::Output { stdout } => {
                serde_json::from_str(&stdout).map_err(|error| error.to_string())
            }
            GhAnswer::Failed { category, .. } => Err(category.english().to_owned()),
        }
    };
    let result = read(ChecksList::CheckRuns)
        .and_then(|runs| read(ChecksList::Status).map(|statuses| (runs, statuses)));
    let (state, url) = match result {
        Ok((runs, statuses)) => (
            CiState::combine(runs.state, statuses.state),
            runs.url.or(statuses.url),
        ),
        Err(reason) => {
            crate::diagnostic!(
                serde_json::json!({"component":"checkout_git", "kind":"ci.read_failed", "sha":request.sha, "reason":reason})
            );
            (CiState::Unavailable, None)
        }
    };
    Ci {
        sha: request.sha.clone(),
        state,
        checked_at_unix_ms: unix_ms(),
        // Only actual HTTP(S) provider URLs can reach Browser View.
        url: url.filter(|url| url.starts_with("https://") || url.starts_with("http://")),
    }
}

fn unix_ms() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
}

#[cfg(test)]
mod tests {
    use super::CiState::{self, *};
    #[test]
    fn a_commit_status_failure_or_pending_blocks_a_passing_check_run() {
        assert_eq!(CiState::combine(Passing, Failed), Failed);
        assert_eq!(CiState::combine(Passing, Pending), Pending);
        assert_eq!(CiState::combine(Passing, Unknown), Unknown);
        assert_eq!(CiState::combine(None, None), None);
        assert_eq!(CiState::combine(None, Passing), Passing);
    }
}
