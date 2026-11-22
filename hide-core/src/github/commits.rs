//! The checks of exact commits, read in one fixed GraphQL query of at most
//! five commits, with each answer's commit id and address checked against
//! the commit asked about.

use std::time::Duration;

use hide_node_link::gh::{GhAnswer, checkout_checks_query};
use hide_node_link::protocol::Call;
use serde::Deserialize;

use crate::checkout_git::{Ci, CiState};
use crate::node_access::{NodeLink, call_as};
use crate::observed::{Observation, now_unix_ms};

/// What each of `commits` has for checks in `repository` (`OWNER/NAME`).
pub(super) fn read(
    node: &dyn NodeLink,
    repository: &str,
    commits: &[String],
) -> Result<Vec<Ci>, String> {
    let query =
        checkout_checks_query(repository, commits).ok_or("Invalid exact-SHA check targets")?;
    let answer: GhAnswer = call_as(
        node,
        Call::Gh {
            cwd: None,
            args: vec!["api".into(), "graphql".into(), "-f".into(), query],
            repository: Some(repository.to_owned()),
        },
        Duration::from_secs(20),
    )
    .map_err(|error| error.to_string())?;
    match answer {
        GhAnswer::Output { stdout } => parse_checks(&stdout, repository, commits),
        GhAnswer::Failed { category, .. } => Err(category.english().to_owned()),
    }
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
