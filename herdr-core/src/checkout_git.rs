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
    pub reason: Option<String>,
    pub ci: Option<Ci>,
    pub recent_ci: Vec<Ci>,
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

pub(crate) type GitAnswer = (GitRequest, Result<CheckoutGit, String>);
pub(crate) type CiAnswer = (CiRequest, Vec<Ci>);

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
                checked_at_unix_ms: unix_ms(),
                url,
            })
        })
        .collect()
}

fn read_ci(request: &CiRequest) -> Vec<Ci> {
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
    match read() {
        Ok(checks) => checks,
        Err(reason) => {
            crate::diagnostic!(
                serde_json::json!({"component":"checkout_git", "kind":"ci.read_failed", "sha":request.sha, "reason":reason})
            );
            request
                .commits
                .iter()
                .map(|sha| Ci {
                    sha: sha.clone(),
                    state: CiState::Unavailable,
                    checked_at_unix_ms: unix_ms(),
                    url: None,
                })
                .collect()
        }
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
