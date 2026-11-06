//! The last GitHub answer per project, kept across a restart.
//!
//! The runtime holds the authoritative copy; this file only lets a daemon that
//! has just started draw the sidebar's pull request state before its first
//! `gh` pass finishes. A restored answer is always stale until a read replaces
//! it, so a file that is days old is shown as old rather than as current.
//!
//! The write runs on its own thread, never under `Mutex<Runtime>`: the runtime
//! hands over an owned snapshot and the thread serializes and replaces the
//! file. One thread and one pending slot per store, so a burst of answers
//! writes the newest and skips the rest.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::latest_file::{LatestFile, read_capped};
use crate::model::{GithubProjectSnapshot, GithubSnapshot};

const SCHEMA_VERSION: u32 = 2;

/// The most a file may hold. The projects (64), their pull requests (200) and
/// issues (200) are capped, so a file past this is not one this store wrote.
const READ_CAP: u64 = 32 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
struct Stored {
    schema_version: u32,
    projects: Vec<StoredProject>,
}

/// A project as the wire carries it, plus what the wire leaves out of each pull
/// request (`skip_serializing`): its times and the commit and repository that
/// decide which checkout it belongs to. They sit beside it, one entry per pull
/// request, so a field the wire does carry is stored without a change here; a
/// new field the wire omits must be added to `PullRequestExtras`, which the
/// round-trip test in this file catches by building a pull request literally.
#[derive(Deserialize, Serialize)]
struct StoredProject {
    #[serde(flatten)]
    project: GithubProjectSnapshot,
    pull_request_extras: Vec<PullRequestExtras>,
}

#[derive(Deserialize, Serialize)]
struct PullRequestExtras {
    created_at_unix_ms: Option<u64>,
    closed_at_unix_ms: Option<u64>,
    head_oid: Option<String>,
    cross_repository: bool,
}

fn encode(github: &GithubSnapshot) -> Result<Vec<u8>, String> {
    let stored = Stored {
        schema_version: SCHEMA_VERSION,
        // Only a project that has been read successfully has an answer worth
        // restoring; the others would draw an empty list as if it were one.
        projects: github
            .projects
            .iter()
            .filter(|project| project.status.last_success_at_unix_ms.is_some())
            .map(|project| StoredProject {
                pull_request_extras: project
                    .pull_requests
                    .iter()
                    .map(|pull_request| PullRequestExtras {
                        created_at_unix_ms: pull_request.created_at_unix_ms,
                        closed_at_unix_ms: pull_request.closed_at_unix_ms,
                        head_oid: pull_request.head_oid.clone(),
                        cross_repository: pull_request.cross_repository,
                    })
                    .collect(),
                project: project.clone(),
            })
            .collect(),
    };
    serde_json::to_vec(&stored).map_err(|error| format!("could not be encoded: {error}"))
}

/// What a restored project says about itself: read once, not current.
fn decode(bytes: &[u8]) -> Result<GithubSnapshot, String> {
    let stored: Stored =
        serde_json::from_slice(bytes).map_err(|error| format!("could not be decoded: {error}"))?;
    if stored.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "has schema version {}, expected {SCHEMA_VERSION}",
            stored.schema_version
        ));
    }
    let mut projects = Vec::with_capacity(stored.projects.len());
    for stored in stored.projects {
        let StoredProject {
            mut project,
            pull_request_extras,
        } = stored;
        if pull_request_extras.len() != project.pull_requests.len() {
            return Err(format!(
                "lists {} pull request extras for {} pull requests in {}",
                pull_request_extras.len(),
                project.pull_requests.len(),
                project.root_path
            ));
        }
        for (pull_request, extras) in project.pull_requests.iter_mut().zip(pull_request_extras) {
            pull_request.created_at_unix_ms = extras.created_at_unix_ms;
            pull_request.closed_at_unix_ms = extras.closed_at_unix_ms;
            pull_request.head_oid = extras.head_oid;
            pull_request.cross_repository = extras.cross_repository;
        }
        if project.status.last_success_at_unix_ms.is_none() {
            return Err(format!(
                "holds {} without a successful read",
                project.root_path
            ));
        }
        project.pull_requests_read = true;
        project.issues_read = true;
        project.status.stale = true;
        project.status.loading = false;
        projects.push(project);
    }
    Ok(GithubSnapshot { projects })
}

pub(crate) struct GithubStore {
    path: PathBuf,
    file: LatestFile<GithubSnapshot>,
}

impl GithubStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            file: LatestFile::new(
                path.clone(),
                "hide-github-save",
                "github",
                "snapshot",
                encode,
            ),
            path,
        }
    }

    /// What the last run saved. A missing file is the first run and says
    /// nothing; any other reason a file is not used is stated, then the file is
    /// ignored and the next save replaces it.
    pub(crate) fn restore(&self) -> Option<GithubSnapshot> {
        let discard = |reason: String| {
            crate::diagnostic!(serde_json::json!({
                "component": "github",
                "kind": "snapshot.discarded",
                "path": self.path.to_string_lossy(),
                "message": reason,
            }));
            None
        };
        let bytes = match read_capped(&self.path, READ_CAP) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return None,
            Err(reason) => return discard(reason),
        };
        match decode(&bytes) {
            Ok(github) => {
                crate::diagnostic!(serde_json::json!({
                    "component": "github",
                    "kind": "snapshot.restored",
                    "projects": github.projects.len(),
                }));
                Some(github)
            }
            Err(reason) => discard(reason),
        }
    }

    /// Queues `github` to replace the file. A snapshot still waiting when a
    /// newer one arrives is dropped: only the latest answer is worth writing.
    pub(crate) fn save(&self, github: GithubSnapshot) {
        self.file.save(github);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        GithubStatusSnapshot, PullRequestBadge, PullRequestChecks, PullRequestSnapshot,
    };

    fn answer(root: &str, last_success: Option<u64>) -> GithubProjectSnapshot {
        GithubProjectSnapshot {
            root_path: root.to_owned(),
            status: GithubStatusSnapshot {
                available: true,
                last_success_at_unix_ms: last_success,
                ..Default::default()
            },
            pull_requests: vec![PullRequestSnapshot {
                closing_issues: Vec::new(),
                title: "Fix".to_owned(),
                checks: PullRequestChecks::Failed,
                number: 12,
                head_branch: "fix".to_owned(),
                base_branch: "main".to_owned(),
                url: "https://example.invalid/pull/12".to_owned(),
                badge: PullRequestBadge::Closed,
                review: None,
                is_draft: false,
                merged_at_unix_ms: None,
                updated_at_unix_ms: Some(9),
                created_at_unix_ms: Some(4),
                closed_at_unix_ms: Some(8),
                head_oid: Some("0123456789abcdef".to_owned()),
                cross_repository: true,
                work: Default::default(),
            }],
            pull_requests_read: true,
            issues_read: true,
            ..Default::default()
        }
    }

    fn saved_then_restored(github: GithubSnapshot) -> Option<GithubSnapshot> {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("github-snapshot.json");
        let store = GithubStore::new(path.clone());
        store.save(github);
        drop(store);
        GithubStore::new(path).restore()
    }

    #[test]
    fn a_saved_answer_comes_back_stale_with_every_pull_request_field() {
        let sent = GithubSnapshot {
            projects: vec![answer("/repo", Some(100))],
        };
        let restored = saved_then_restored(sent.clone()).expect("the file is restored");
        let project = &restored.projects[0];
        assert!(project.status.stale && !project.status.loading);
        assert_eq!(project.status.last_success_at_unix_ms, Some(100));
        assert_eq!(project.pull_requests, sent.projects[0].pull_requests);
        assert!(project.pull_requests_read && project.issues_read);
    }

    #[test]
    fn an_issues_labels_are_restored_and_a_file_saved_before_labels_still_restores() {
        let mut sent = answer("/repo", Some(100));
        sent.issues = crate::issues::ProjectIssuesSnapshot {
            repository: Some("acme/app".into()),
            issues: crate::issues::parse_issues(r#"[{"number":7,"title":"Fix","url":"https://github.com/acme/app/issues/7","state":"OPEN","labels":[{"name":"bug","color":"d73a4a"}]}]"#).unwrap(),
            ..Default::default()
        };
        let github = GithubSnapshot {
            projects: vec![sent],
        };
        let restored = saved_then_restored(github.clone()).expect("the file is restored");
        assert_eq!(
            restored.projects[0].issues.issues[0].labels,
            github.projects[0].issues.issues[0].labels
        );
        let mut older: serde_json::Value =
            serde_json::from_slice(&encode(&github).unwrap()).unwrap();
        older["projects"][0]["issues"]["issues"][0]
            .as_object_mut()
            .unwrap()
            .remove("labels")
            .expect("the saved issue carried its labels");
        let restored =
            decode(&serde_json::to_vec(&older).unwrap()).expect("an older file restores");
        assert!(restored.projects[0].issues.issues[0].labels.is_empty());
    }

    #[test]
    fn a_project_never_read_successfully_is_not_saved() {
        let restored = saved_then_restored(GithubSnapshot {
            projects: vec![answer("/never", None), answer("/repo", Some(1))],
        })
        .unwrap();
        assert_eq!(
            restored
                .projects
                .iter()
                .map(|project| project.root_path.as_str())
                .collect::<Vec<_>>(),
            vec!["/repo"]
        );
    }

    #[test]
    fn the_newest_of_several_saves_is_the_one_on_disk() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("github-snapshot.json");
        let store = GithubStore::new(path.clone());
        for at in 1..=20 {
            store.save(GithubSnapshot {
                projects: vec![answer("/repo", Some(at))],
            });
        }
        drop(store);
        let restored = GithubStore::new(path).restore().unwrap();
        assert_eq!(
            restored.projects[0].status.last_success_at_unix_ms,
            Some(20)
        );
    }

    #[test]
    fn a_missing_file_restores_nothing() {
        let folder = tempfile::tempdir().unwrap();
        assert!(
            GithubStore::new(folder.path().join("github-snapshot.json"))
                .restore()
                .is_none()
        );
    }

    #[test]
    fn a_damaged_or_other_version_file_is_not_used() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("github-snapshot.json");
        let store = GithubStore::new(path.clone());
        let good = encode(&GithubSnapshot {
            projects: vec![answer("/repo", Some(1))],
        })
        .unwrap();
        let other_version = String::from_utf8(good.clone()).unwrap().replacen(
            "\"schema_version\":2",
            "\"schema_version\":1",
            1,
        );
        let torn = &good[..good.len() / 2];
        let mismatched_times = String::from_utf8(good.clone()).unwrap().replacen(
            "\"pull_request_extras\":[{",
            "\"pull_request_extras\":[{},{",
            1,
        );
        for bytes in [
            b"not json".as_slice(),
            other_version.as_bytes(),
            torn,
            mismatched_times.as_bytes(),
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(store.restore().is_none());
        }
    }
}
