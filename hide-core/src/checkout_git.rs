//! The active checkout's header reads, driven by the existing Changes pump.
//! Two single-flight workers keep Git independent of a slow GitHub request.
use std::time::Duration;

use hide_node_link::gh::{GhAnswer, checkout_checks_query};
use hide_node_link::protocol::Call;
use hide_node_link::worktrees::CheckoutGit;
use serde::{Deserialize, Serialize};

use crate::changes::ChannelRef;
use crate::files::DocumentRoot;
use crate::node_access::call_as;
use crate::observed::{Observation, now_unix_ms};
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

#[derive(Deserialize)]
struct CommitChecks {
    oid: String,
    url: String,
    #[serde(rename = "statusCheckRollup")]
    rollup: Option<CheckRollup>,
}
#[derive(Deserialize)]
struct CheckRollup {
    state: String,
}
#[derive(Deserialize)]
struct ChecksData {
    repository: Option<std::collections::HashMap<String, Option<CommitChecks>>>,
}
#[derive(Deserialize)]
struct ChecksResponse {
    data: Option<ChecksData>,
    errors: Option<Vec<serde_json::Value>>,
}

fn parse_checks(
    stdout: &str,
    repository_name: &str,
    commits: &[String],
) -> Result<Vec<Ci>, String> {
    let response: ChecksResponse =
        serde_json::from_str(stdout).map_err(|error| error.to_string())?;
    if response.errors.is_some_and(|errors| !errors.is_empty()) {
        return Err("Provider returned check errors".into());
    }
    let mut repository = response
        .data
        .and_then(|data| data.repository)
        .ok_or("Provider returned no repository")?;
    commits
        .iter()
        .enumerate()
        .map(|(index, sha)| {
            let commit = repository
                .remove(&format!("c{index}"))
                .ok_or("Provider omitted a commit result")?;
            let (state, url) = match commit {
                None => (CiState::None, None), // A local commit may not have been pushed.
                Some(commit) => {
                    if commit.oid != *sha
                        || !commit.url.eq_ignore_ascii_case(&format!(
                            "https://github.com/{}/commit/{sha}",
                            repository_name
                        ))
                    {
                        return Err("Provider returned a different commit identity".into());
                    }
                    let state = match commit.rollup.map(|rollup| rollup.state).as_deref() {
                        None => CiState::None,
                        Some("SUCCESS") => CiState::Passing,
                        Some("PENDING" | "EXPECTED") => CiState::Pending,
                        Some("FAILURE" | "ERROR") => CiState::Failed,
                        Some(_) => return Err("Provider returned an unknown check rollup".into()),
                    };
                    (state, Some(commit.url))
                }
            };
            Ok(Ci {
                sha: sha.clone(),
                state,
                observed: Some(Observation::fresh(now_unix_ms())),
                url,
            })
        })
        .collect()
}

fn read_ci(request: &CiRequest) -> Result<Vec<Ci>, String> {
    let read = || -> Result<Vec<Ci>, String> {
        let query = checkout_checks_query(&request.repository, &request.commits)
            .ok_or("Invalid exact-SHA check targets")?;
        let answer: GhAnswer = call_as(
            &*request.channel.0,
            Call::Gh {
                cwd: None,
                args: vec!["api".into(), "graphql".into(), "-f".into(), query],
                repository: Some(request.repository.clone()),
            },
            Duration::from_secs(20),
        )
        .map_err(|error| error.to_string())?;
        match answer {
            GhAnswer::Output { stdout } => {
                parse_checks(&stdout, &request.repository, &request.commits)
            }
            GhAnswer::Failed { category, .. } => Err(category.english().to_owned()),
        }
    };
    read().inspect_err(|reason| {
        crate::diagnostic!(
            serde_json::json!({"component":"checkout_git", "kind":"ci.read_failed", "sha":request.sha, "reason":reason})
        );
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_rollups_keep_failures_pending_and_absent_ci_distinct() {
        let commits: Vec<_> = ['a', 'b', 'c', 'd', 'e']
            .into_iter()
            .map(|name| name.to_string().repeat(40))
            .collect();
        let mut repository = serde_json::Map::new();
        for (index, state) in [
            Some("FAILURE"),
            Some("PENDING"),
            Some("SUCCESS"),
            None,
            Some("ERROR"),
        ]
        .into_iter()
        .enumerate()
        {
            repository.insert(format!("c{index}"), serde_json::json!({"oid": commits[index], "url": format!("https://github.com/example/fixture/commit/{}", commits[index]), "statusCheckRollup": state.map(|state| serde_json::json!({"state": state}))}));
        }
        let response = serde_json::json!({"data":{"repository": repository}});
        let result = parse_checks(&response.to_string(), "example/fixture", &commits).unwrap();
        assert_eq!(
            result.iter().map(|ci| ci.state).collect::<Vec<_>>(),
            [
                CiState::Failed,
                CiState::Pending,
                CiState::Passing,
                CiState::None,
                CiState::Failed
            ]
        );
        let mut absent = response.clone();
        absent["data"]["repository"]["c0"] = serde_json::Value::Null;
        assert_eq!(
            parse_checks(&absent.to_string(), "example/fixture", &commits).unwrap()[0].state,
            CiState::None
        );
        for field in ["oid", "url"] {
            let mut wrong = response.clone();
            wrong["data"]["repository"]["c0"][field] = "wrong-target".into();
            assert!(parse_checks(&wrong.to_string(), "example/fixture", &commits).is_err());
        }
        let mut missing = response.clone();
        missing["data"]["repository"]
            .as_object_mut()
            .unwrap()
            .remove("c4");
        assert!(parse_checks(&missing.to_string(), "example/fixture", &commits).is_err());
        let mut partial = response;
        partial["errors"] = serde_json::json!([{"message":"provider failure"}]);
        assert!(parse_checks(&partial.to_string(), "example/fixture", &commits).is_err());
    }
}
