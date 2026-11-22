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
use crate::model::{GithubProjectSnapshot, GithubSnapshot, RepositoryIdentity};

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
                project: GithubProjectSnapshot {
                    // Made again from the identity on restore.
                    status: crate::model::GithubStatusSnapshot {
                        repository: None,
                        ..project.status.clone()
                    },
                    ..project.clone()
                },
            })
            .collect(),
    };
    serde_json::to_vec(&stored).map_err(|error| format!("could not be encoded: {error}"))
}

/// What a file from before the identity was saved said about a repository's
/// visibility: under its issues (`issues.public`).
#[derive(Deserialize)]
struct LegacyStored {
    #[serde(default)]
    projects: Vec<LegacyProject>,
}

#[derive(Deserialize)]
struct LegacyProject {
    root_path: String,
    #[serde(default)]
    issues: LegacyIssues,
}

#[derive(Default, Deserialize)]
struct LegacyIssues {
    #[serde(default)]
    public: Option<bool>,
}

/// What a restored project says about itself: read once, not current.
fn decode(bytes: &[u8]) -> Result<GithubSnapshot, String> {
    // A saved declaration is not trusted: each status's declaration is the
    // registered unit's own, found by the saved key (a key no unit registers
    // restores as none), as each reference's pages are made again below.
    let mut value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("could not be decoded: {error}"))?;
    let mut hosts = std::collections::HashMap::new();
    for project in value["projects"].as_array_mut().into_iter().flatten() {
        let root = project["root_path"].as_str().unwrap_or_default().to_owned();
        let saved = project["status"]
            .as_object_mut()
            .and_then(|status| status.remove("host"));
        let declaration = saved
            .as_ref()
            .and_then(|host| host["key"].as_str())
            .and_then(|key| crate::code_host::CodeHosts::STANDARD.host(key))
            .map(|host| crate::code_host::declaration(host.unit()));
        hosts.insert(root, declaration);
    }
    let stored: Stored =
        serde_json::from_value(value).map_err(|error| format!("could not be decoded: {error}"))?;
    let legacy_public: std::collections::HashMap<String, Option<bool>> =
        serde_json::from_slice::<LegacyStored>(bytes)
            .map(|legacy| {
                legacy
                    .projects
                    .into_iter()
                    .map(|project| (project.root_path, project.issues.public))
                    .collect()
            })
            .unwrap_or_default();
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
        project.status.host = hosts.remove(&project.root_path).flatten();
        // The issues part comes back with the outcome it was saved with; a
        // file from before it was kept reads `Read` (the field's default).
        // A saved reference names its repository and number as its host's
        // unit made them; the unit makes the rest again (its label, pages and
        // key), so a file saved before it made one it makes now reads like a
        // fresh answer, and a host no unit is registered for keeps what it
        // saved. A file from before references were saved is GitHub's: each
        // of its pull requests is named by GitHub's unit from its own
        // address, server and port kept, as a fresh read names it.
        // One saved before keys were made whose host has no unit names
        // nothing it can be compared by, and is dropped.
        for pull_request in &mut project.pull_requests {
            let named = match &pull_request.reference {
                Some(saved) => crate::code_host::CodeHosts::STANDARD
                    .of(&saved.repository)
                    .map(|host| (host, saved.repository.clone(), saved.number)),
                None => crate::code_host::CodeHosts::STANDARD.change_request_at(&pull_request.url),
            };
            if let Some((host, repository, number)) = named {
                pull_request.reference = Some(crate::code_host::change_request(
                    host.unit(),
                    repository,
                    number,
                ));
            } else if pull_request
                .reference
                .as_ref()
                .is_some_and(|saved| saved.key.is_empty())
            {
                pull_request.reference = None;
            }
        }
        // A file from before the identity was saved: its pull requests'
        // addresses name the repository with its server. The issues' bare
        // `OWNER/NAME` does not (an Enterprise server's reads as github.com),
        // so a file without pull requests waits for the next read.
        if project.repository.is_none() {
            project.repository = project
                .pull_requests
                .iter()
                .find_map(|pull_request| pull_request.reference.as_ref())
                .and_then(|reference| {
                    let host = crate::code_host::CodeHosts::STANDARD.of(&reference.repository)?;
                    Some(RepositoryIdentity {
                        public: legacy_public.get(&project.root_path).copied().flatten(),
                        ..RepositoryIdentity::named(host.unit(), reference.repository.clone())
                    })
                });
        }
        // The repository's page is its unit's to make, again on restore.
        if let Some(identity) = &mut project.repository {
            identity.url = identity.repository.as_ref().and_then(|repository| {
                let host = crate::code_host::CodeHosts::STANDARD.of(repository)?;
                Some(host.unit().repository_url(repository))
            });
        }
        project.show_repository();
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
                reference: None,
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
            issues_read: crate::model::PartRead::Read,
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

    /// A file as main saved it: no identity, no outcome of the issues part
    /// and no reference on its pull requests, the visibility under its
    /// issues beside their bare `OWNER/NAME`.
    fn saved_before_hosts(url: Option<&str>) -> GithubSnapshot {
        let mut project = answer("/repo", Some(100));
        match url {
            Some(url) => project.pull_requests[0].url = url.to_owned(),
            None => project.pull_requests.clear(),
        }
        let mut stored: serde_json::Value = serde_json::from_slice(
            &encode(&GithubSnapshot {
                projects: vec![project],
            })
            .unwrap(),
        )
        .unwrap();
        let fields = stored["projects"][0].as_object_mut().unwrap();
        fields.remove("repository");
        fields.remove("issues_read");
        fields.insert(
            "issues".to_owned(),
            serde_json::json!({"repository": "acme/app", "issues": [], "overflow": false, "public": true}),
        );
        for pull_request in fields["pull_requests"].as_array_mut().unwrap() {
            pull_request.as_object_mut().unwrap().remove("reference");
        }
        decode(&serde_json::to_vec(&stored).unwrap()).expect("a file main saved restores")
    }

    /// Each pull request of a file main saved is named again from its own
    /// address, server and port kept; the repository's identity comes from
    /// that address, never from the issues' bare `OWNER/NAME`, which would
    /// read an Enterprise server's repository as github.com's.
    #[test]
    fn a_file_saved_before_hosts_is_named_from_its_pull_requests_addresses() {
        for (url, authority, files) in [
            (
                "https://github.com/acme/app/pull/12",
                "github.com",
                "https://github.com/acme/app/pull/12/files",
            ),
            (
                "https://ghe.example.com/acme/app/pull/12",
                "ghe.example.com",
                "https://ghe.example.com/acme/app/pull/12/files",
            ),
            (
                "https://ghe.example.com:8443/acme/app/pull/12",
                "ghe.example.com:8443",
                "https://ghe.example.com:8443/acme/app/pull/12/files",
            ),
        ] {
            let restored = saved_before_hosts(Some(url));
            let project = &restored.projects[0];
            let reference = project.pull_requests[0]
                .reference
                .as_ref()
                .expect("named again");
            assert_eq!(
                (
                    reference.repository.authority.as_str(),
                    reference.files_url.as_str()
                ),
                (authority, files)
            );
            let identity = project.repository.as_ref().expect("named by the address");
            assert_eq!(identity.repository.as_ref(), Some(&reference.repository));
            assert_eq!(identity.public, Some(true), "the visibility main saved");
            assert_eq!(project.issues_read, crate::model::PartRead::Read);
        }
        assert_eq!(
            saved_before_hosts(None).projects[0].repository,
            None,
            "without a pull request the server is unknown until the next read"
        );
    }

    /// The repository the screen names and opens is made from the saved
    /// identity on restore by its host's unit, the server's port and a
    /// gitlab.com subgroup kept, and never written to the file itself.
    #[test]
    fn the_repository_on_the_screen_is_made_again_on_restore_and_not_saved() {
        for (host, authority, path, name, url) in [
            ("github", "github.com", "acme/app", "acme/app", "https://github.com/acme/app"),
            (
                "github",
                "ghe.example.com:8443",
                "acme/app",
                "ghe.example.com:8443/acme/app",
                "https://ghe.example.com:8443/acme/app",
            ),
            (
                "gitlab",
                "gitlab.com",
                "acme/shop/api",
                "gitlab.com/acme/shop/api",
                "https://gitlab.com/acme/shop/api",
            ),
        ] {
            let repository = crate::code_host::RepositoryRef {
                host: host.to_owned(),
                authority: authority.to_owned(),
                path: path.to_owned(),
            };
            let unit = crate::code_host::CodeHosts::STANDARD
                .of(&repository)
                .expect("a registered host")
                .unit();
            let mut sent = answer("/repo", Some(100));
            sent.repository = Some(RepositoryIdentity::named(unit, repository));
            sent.show_repository();
            let shown = crate::model::RepositoryOnScreen {
                name: name.to_owned(),
                url: url.to_owned(),
            };
            assert_eq!(sent.status.repository.as_ref(), Some(&shown));
            let stored: serde_json::Value = serde_json::from_slice(
                &encode(&GithubSnapshot {
                    projects: vec![sent.clone()],
                })
                .unwrap(),
            )
            .unwrap();
            assert!(
                stored["projects"][0]["status"].get("repository").is_none(),
                "the file keeps the identity only"
            );
            let restored = saved_then_restored(GithubSnapshot {
                projects: vec![sent],
            })
            .expect("the file is restored");
            assert_eq!(restored.projects[0].status.repository, Some(shown));
        }
    }

    /// Each of the four outcomes of the issues part comes back as it was
    /// saved; only a file from before the part was kept reads `Read`.
    #[test]
    fn the_issues_and_identity_outcomes_come_back_as_saved() {
        use crate::model::PartRead;
        for outcome in [
            PartRead::Read,
            PartRead::Failed,
            PartRead::NotRead,
            PartRead::Unsupported,
        ] {
            let mut sent = answer("/repo", Some(100));
            sent.issues_read = outcome;
            sent.identity_read = outcome;
            let restored = saved_then_restored(GithubSnapshot {
                projects: vec![sent],
            })
            .expect("the file is restored");
            assert_eq!(restored.projects[0].issues_read, outcome);
            assert_eq!(restored.projects[0].identity_read, outcome);
        }
        let mut stored: serde_json::Value = serde_json::from_slice(
            &encode(&GithubSnapshot {
                projects: vec![answer("/repo", Some(100))],
            })
            .unwrap(),
        )
        .unwrap();
        let project = stored["projects"][0].as_object_mut().unwrap();
        assert!(project.remove("issues_read").is_some());
        assert!(project.remove("identity_read").is_some());
        let restored = decode(&serde_json::to_vec(&stored).unwrap()).unwrap();
        assert_eq!(restored.projects[0].issues_read, PartRead::Read);
        assert_eq!(restored.projects[0].identity_read, PartRead::Read);
    }

    /// A reference another host's unit made comes back as saved, even where
    /// its address would read as a GitHub Enterprise one: the file never
    /// moves a pull request to another host.
    #[test]
    fn a_saved_reference_of_another_host_comes_back_as_its_unit_made_it() {
        let repository = crate::code_host::RepositoryRef {
            host: "plain".to_owned(),
            authority: "code.example".to_owned(),
            path: "acme/app".to_owned(),
        };
        let mut sent = answer("/repo", Some(100));
        let pull_request = &mut sent.pull_requests[0];
        pull_request.url = format!("https://code.example/acme/app/pull/{}", pull_request.number);
        let made = crate::code_host::change_request(
            &crate::code_host::tests::PLAIN,
            repository,
            pull_request.number,
        );
        pull_request.reference = Some(made.clone());
        let restored = saved_then_restored(GithubSnapshot {
            projects: vec![sent],
        })
        .expect("the file is restored");
        assert_eq!(restored.projects[0].pull_requests[0].reference, Some(made));
    }

    #[test]
    fn a_reference_saved_in_an_older_shape_is_named_again_rather_than_refusing_the_file() {
        let mut sent = answer("/repo", Some(100));
        let number = sent.pull_requests[0].number;
        sent.repository = crate::code_host::CodeHosts::STANDARD
            .repository("acme/app")
            .map(|(host, repository)| RepositoryIdentity::named(host.unit(), repository));
        let mut stored: serde_json::Value = serde_json::from_slice(
            &encode(&GithubSnapshot {
                projects: vec![sent],
            })
            .unwrap(),
        )
        .unwrap();
        stored["projects"][0]["pull_requests"][0]["reference"] = serde_json::json!({
            "repository": {"host": "github", "authority": "github.com", "path": "acme/app"},
            "number": number,
        });
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("github-snapshot.json");
        std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let restored = GithubStore::new(path.clone())
            .restore()
            .expect("the file is restored");
        let pull_request = &restored.projects[0].pull_requests[0];
        let reference = pull_request.reference.as_ref().expect("named again");
        assert_eq!(
            reference.key,
            format!("github.com/acme/app#{}", pull_request.number)
        );

        // One whose host no unit is registered for names nothing it can be
        // compared by, and is dropped rather than kept with an empty key.
        stored["projects"][0]["pull_requests"][0]["reference"] = serde_json::json!({
            "repository": {"host": "plain", "authority": "code.example", "path": "acme/app"},
            "number": number,
        });
        std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let restored = GithubStore::new(path)
            .restore()
            .expect("the file is restored");
        assert_eq!(restored.projects[0].pull_requests[0].reference, None);
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
        assert!(project.pull_requests_read);
        assert_eq!(project.issues_read, crate::model::PartRead::Read);
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

    /// A checkout whose remotes were not known has no read to save: a
    /// restore brings back neither an empty read list nor no remote for it,
    /// so the reader makes it again from the node's facts.
    #[test]
    fn remotes_not_known_is_not_saved_and_restores_as_nothing() {
        let mut unknown = crate::code_host::read::no_remote("/unknown".to_owned());
        unknown.remotes_not_known = true;
        let restored = saved_then_restored(GithubSnapshot {
            projects: vec![unknown, answer("/repo", Some(100))],
        })
        .unwrap();
        assert_eq!(restored.project("/unknown"), None);
        assert!(restored.project("/repo").is_some());
    }

    /// The store does not trust a saved declaration: a status comes back
    /// with its registered unit's declaration as it is now, found by the
    /// saved key, whatever the file held beside the key; a key no unit
    /// registers comes back with none.
    #[test]
    fn a_saved_declaration_comes_back_as_the_registered_units_by_its_key() {
        let gitlab = crate::code_host::declaration(&crate::gitlab::host::GITLAB);
        let mut projects = Vec::new();
        for root in ["/unnamed", "/operations", "/unknown"] {
            let mut project = answer(root, Some(100));
            project.status.host = Some(gitlab.clone());
            projects.push(project);
        }
        let mut stored: serde_json::Value =
            serde_json::from_slice(&encode(&GithubSnapshot { projects }).unwrap()).unwrap();
        let host = |stored: &mut serde_json::Value, index: usize| {
            stored["projects"][index]["status"]["host"]
                .as_object_mut()
                .unwrap()
                .clone()
        };
        let mut unnamed = host(&mut stored, 0);
        unnamed.remove("name");
        stored["projects"][0]["status"]["host"] = unnamed.into();
        stored["projects"][1]["status"]["host"]["operations"] =
            serde_json::json!(["merge", "watch"]);
        stored["projects"][2]["status"]["host"]["key"] = "elsewhere".into();
        let restored = decode(&serde_json::to_vec(&stored).unwrap()).unwrap();
        let declared = |root: &str| restored.project(root).unwrap().status.host.clone();
        assert_eq!(declared("/unnamed"), Some(gitlab.clone()));
        assert_eq!(declared("/operations"), Some(gitlab));
        assert_eq!(declared("/unknown"), None);
    }
}
