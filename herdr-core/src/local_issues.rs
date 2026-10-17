//! Local issues: the issue source for a project that keeps its issues on this
//! Mac rather than on GitHub (the Tasks board's `Local` adapter, `tasks.rs`).
//!
//! Every project's local issues live in one file in hided's state directory,
//! `local-issues.json` beside `core-state.json`, keyed by the project's path:
//! the core's own machine's projects under `projects`, and a node's that
//! dials the core under `devices` by that node's id, which is the slot a core
//! move writes the machine giving up the core into (`node_migration::reown`),
//! so its issues are the same ones after the move. The runtime reads it once when it is created, before any
//! lock exists, holds the store in memory, applies a change there at once,
//! and hands the whole store to a save thread, so no file I/O happens under
//! the runtime mutex (the UI state's own save pattern, `write_ui_state`).
//!
//! An issue is numbered per project from 1 and shown as `L-<n>`; a number is
//! never reused, so a link written into a branch keeps naming the same issue.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file format version this code writes and reads.
const VERSION: u32 = 1;
/// Issues one project may hold. Creating past it is refused, not truncated.
pub const ISSUES_PER_PROJECT: usize = 2_000;
/// Projects the store may hold, every machine's together.
pub const PROJECTS: usize = 500;
/// A title is one line of at most this many characters.
pub const TITLE_LIMIT: usize = 256;
/// A body is at most this many bytes.
pub const BODY_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalIssue {
    pub number: u32,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub open: bool,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
    /// Actual closure, unchanged by edits; absent in older stores.
    #[serde(default)]
    pub closed_at_unix_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalIssueProject {
    /// The number the next issue takes; never lowered, so no number repeats.
    pub next_number: u32,
    pub issues: Vec<LocalIssue>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalIssueStore {
    #[serde(default)]
    pub version: u32,
    /// By project path.
    #[serde(default)]
    pub projects: BTreeMap<String, LocalIssueProject>,
    /// Another machine's projects, by the id this folder names that machine
    /// by and then by project path: a core move keeps the machine giving up
    /// the core here (`node_migration::reown`), and the move back makes them
    /// `projects` again. A node that dials the core reads and changes its
    /// own entry through it; every save keeps every entry.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub devices: BTreeMap<String, BTreeMap<String, LocalIssueProject>>,
}

/// `L-3`, the id a local issue shows.
pub fn display_id(number: u32) -> String {
    format!("L-{number}")
}

/// The number in an id an operator or a branch wrote for a local issue:
/// `L-3`, `l-3`, `#3` or `3`.
pub fn parse_id(value: &str) -> Option<u32> {
    let value = value.trim();
    let digits = value
        .strip_prefix("L-")
        .or_else(|| value.strip_prefix("l-"))
        .or_else(|| value.strip_prefix('#'))
        .unwrap_or(value);
    digits.parse::<u32>().ok().filter(|number| *number > 0)
}

/// A title as the store keeps it: one line, trimmed, within the limit.
pub fn validated_title(title: &str) -> Result<String, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("이슈 제목을 입력하세요.".into());
    }
    if title.contains(['\n', '\r']) {
        return Err("이슈 제목은 한 줄이어야 합니다.".into());
    }
    if title.chars().count() > TITLE_LIMIT {
        return Err(format!("이슈 제목은 {TITLE_LIMIT}자 이하여야 합니다."));
    }
    Ok(title.to_owned())
}

pub fn validated_body(body: &str) -> Result<String, String> {
    if body.len() > BODY_LIMIT {
        return Err(format!(
            "이슈 내용은 {} KB 이하여야 합니다.",
            BODY_LIMIT / 1024
        ));
    }
    Ok(body.trim_end().to_owned())
}

impl LocalIssueStore {
    /// One machine's projects: the core's own for `None`, a node's by its id.
    fn machine(&self, machine: Option<&str>) -> Option<&BTreeMap<String, LocalIssueProject>> {
        match machine {
            None => Some(&self.projects),
            Some(device) => self.devices.get(device),
        }
    }

    fn project_mut(&mut self, machine: Option<&str>, path: &str) -> Option<&mut LocalIssueProject> {
        match machine {
            None => self.projects.get_mut(path),
            Some(device) => self.devices.get_mut(device)?.get_mut(path),
        }
    }

    fn issue_mut(
        &mut self,
        machine: Option<&str>,
        path: &str,
        number: u32,
    ) -> Option<&mut LocalIssue> {
        self.project_mut(machine, path)?
            .issues
            .iter_mut()
            .find(|issue| issue.number == number)
    }

    /// A project on `machine`: the core's own for `None`, a node's by its id.
    pub fn project(&self, machine: Option<&str>, path: &str) -> Option<&LocalIssueProject> {
        self.machine(machine)?.get(path)
    }

    pub fn issue(&self, machine: Option<&str>, path: &str, number: u32) -> Option<&LocalIssue> {
        self.project(machine, path)?
            .issues
            .iter()
            .find(|issue| issue.number == number)
    }

    /// Adds an open issue to a project and returns its number.
    pub fn create(
        &mut self,
        machine: Option<&str>,
        path: &str,
        title: &str,
        body: &str,
        now_unix_ms: u64,
    ) -> Result<u32, String> {
        let title = validated_title(title)?;
        let body = validated_body(body)?;
        let held = self.projects.len() + self.devices.values().map(BTreeMap::len).sum::<usize>();
        if self.project(machine, path).is_none() && held >= PROJECTS {
            return Err(format!(
                "로컬 이슈는 {PROJECTS}개 프로젝트까지만 둘 수 있습니다."
            ));
        }
        let projects = match machine {
            None => &mut self.projects,
            Some(device) => self.devices.entry(device.to_owned()).or_default(),
        };
        let project = projects.entry(path.to_owned()).or_default();
        if project.issues.len() >= ISSUES_PER_PROJECT {
            return Err(format!(
                "한 프로젝트의 로컬 이슈는 {ISSUES_PER_PROJECT}개까지입니다."
            ));
        }
        let number = project.next_number.max(1);
        project.next_number = number + 1;
        project.issues.push(LocalIssue {
            number,
            title,
            body,
            open: true,
            created_at_unix_ms: now_unix_ms,
            updated_at_unix_ms: now_unix_ms,
            closed_at_unix_ms: None,
        });
        self.version = VERSION;
        Ok(number)
    }

    /// Rewrites an issue's title and body (PRD overview-lenses-issues D-41);
    /// `Ok(false)` when they are already these.
    pub fn update(
        &mut self,
        machine: Option<&str>,
        path: &str,
        number: u32,
        title: &str,
        body: &str,
        now_unix_ms: u64,
    ) -> Result<bool, String> {
        let title = validated_title(title)?;
        let body = validated_body(body)?;
        let issue = self
            .issue_mut(machine, path, number)
            .ok_or_else(|| format!("로컬 이슈 {}가 없습니다.", display_id(number)))?;
        if issue.title == title && issue.body == body {
            return Ok(false);
        }
        issue.title = title;
        issue.body = body;
        issue.updated_at_unix_ms = now_unix_ms;
        Ok(true)
    }

    /// Removes a memo the backlog just made (`되돌리기`, PRD
    /// overview-device-tasks D-21). Its number is not given out again.
    pub fn delete(&mut self, machine: Option<&str>, path: &str, number: u32) -> Result<(), String> {
        let project = self
            .project_mut(machine, path)
            .ok_or_else(|| format!("로컬 이슈 {}가 없습니다.", display_id(number)))?;
        let before = project.issues.len();
        project.issues.retain(|issue| issue.number != number);
        if project.issues.len() == before {
            return Err(format!("로컬 이슈 {}가 없습니다.", display_id(number)));
        }
        Ok(())
    }

    /// Opens or closes an issue; `false` when there is no such issue or it
    /// already is in that state.
    pub fn set_open(
        &mut self,
        machine: Option<&str>,
        path: &str,
        number: u32,
        open: bool,
        now_unix_ms: u64,
    ) -> bool {
        let Some(issue) = self.issue_mut(machine, path, number) else {
            return false;
        };
        if issue.open == open {
            return false;
        }
        issue.open = open;
        issue.updated_at_unix_ms = now_unix_ms;
        issue.closed_at_unix_ms = (!open).then_some(now_unix_ms);
        true
    }
}

/// Reads the store. A file that is not there is an empty store; one that
/// cannot be read or parsed is an error, so a damaged file is never
/// overwritten by an empty one.
pub fn load(path: &Path) -> Result<LocalIssueStore, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(LocalIssueStore {
                version: VERSION,
                ..LocalIssueStore::default()
            });
        }
        Err(error) => return Err(format!("{}: {error}", path.display())),
    };
    let store: LocalIssueStore =
        serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))?;
    if store.version > VERSION {
        return Err(format!(
            "{} was written by a newer Hide (version {})",
            path.display(),
            store.version
        ));
    }
    Ok(store)
}

/// Writes the whole store through a temporary file and a rename, so a crash
/// mid-write leaves the previous file intact.
pub fn save(path: &Path, store: &LocalIssueStore) -> Result<(), String> {
    let directory = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    hide_platform::fs::private::create_dir_all(directory)
        .map_err(|error| format!("{}: {error}", directory.display()))?;
    let bytes = serde_json::to_vec_pretty(store).map_err(|error| error.to_string())?;
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)
        .map_err(|error| format!("{}: {error}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        format!("{}: {error}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_never_repeat_and_ids_read_back() {
        let mut store = LocalIssueStore::default();
        assert_eq!(store.create(None, "/p", "첫 이슈", "", 1).unwrap(), 1);
        assert_eq!(store.create(None, "/p", "둘째", "본문", 2).unwrap(), 2);
        assert_eq!(store.create(None, "/q", "다른 프로젝트", "", 3).unwrap(), 1);
        store.projects.get_mut("/p").unwrap().issues.remove(1);
        assert_eq!(store.create(None, "/p", "셋째", "", 4).unwrap(), 3);
        assert_eq!(display_id(3), "L-3");
        for written in ["L-3", "l-3", "#3", "3", " L-3 "] {
            assert_eq!(parse_id(written), Some(3), "{written}");
        }
        for invalid in ["L-0", "L-", "x", "L-3a", ""] {
            assert_eq!(parse_id(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn titles_and_bodies_are_bounded() {
        let mut store = LocalIssueStore::default();
        assert!(store.create(None, "/p", "  ", "", 1).is_err());
        assert!(store.create(None, "/p", "두\n줄", "", 1).is_err());
        assert!(
            store
                .create(None, "/p", &"가".repeat(TITLE_LIMIT + 1), "", 1)
                .is_err()
        );
        assert!(
            store
                .create(None, "/p", "ok", &"a".repeat(BODY_LIMIT + 1), 1)
                .is_err()
        );
        assert!(store.projects.is_empty());
    }

    #[test]
    fn a_project_past_its_cap_is_refused() {
        let mut store = LocalIssueStore::default();
        store.projects.insert(
            "/p".into(),
            LocalIssueProject {
                next_number: ISSUES_PER_PROJECT as u32 + 1,
                issues: (1..=ISSUES_PER_PROJECT as u32)
                    .map(|number| LocalIssue {
                        number,
                        title: "t".into(),
                        open: true,
                        ..LocalIssue::default()
                    })
                    .collect(),
            },
        );
        assert!(store.create(None, "/p", "one more", "", 1).is_err());
    }

    #[test]
    fn closure_time_survives_edits_and_tracks_reopen_and_close() {
        let mut store = LocalIssueStore::default();
        store.create(None, "/p", "First work", "", 1).unwrap();
        assert!(store.set_open(None, "/p", 1, false, 2));
        assert!(
            store
                .update(None, "/p", 1, "Edited after closure", "", 3)
                .unwrap()
        );
        let issue = &store.projects["/p"].issues[0];
        assert_eq!(issue.updated_at_unix_ms, 3);
        assert_eq!(issue.closed_at_unix_ms, Some(2));
        assert!(store.set_open(None, "/p", 1, true, 4));
        assert_eq!(store.projects["/p"].issues[0].closed_at_unix_ms, None);
        assert!(store.set_open(None, "/p", 1, false, 5));
        assert_eq!(store.projects["/p"].issues[0].closed_at_unix_ms, Some(5));
        let old: LocalIssue = serde_json::from_str(
            r#"{"number":1,"title":"Older closed work","open":false,"created_at_unix_ms":1,"updated_at_unix_ms":3}"#,
        ).unwrap();
        assert_eq!(old.closed_at_unix_ms, None);
    }

    #[test]
    fn the_store_survives_a_round_trip_and_a_missing_file_is_empty() {
        let folder = tempfile::tempdir().unwrap();
        let directory = folder.path().to_path_buf();
        let path = directory.join("local-issues.json");
        assert!(load(&path).unwrap().projects.is_empty());
        let mut store = LocalIssueStore::default();
        store.create(None, "/p", "한국어 제목", "본문", 7).unwrap();
        assert!(store.set_open(None, "/p", 1, false, 8));
        assert!(!store.set_open(None, "/p", 1, false, 9));
        save(&path, &store).unwrap();
        assert_eq!(load(&path).unwrap(), store);
        std::fs::write(&path, b"{not json").unwrap();
        assert!(load(&path).is_err());
    }

    /// A node's projects are its own: the same path on the core's machine
    /// and on a node are two projects, each numbering from its own last
    /// issue, and a project counts against the cap whichever machine holds it.
    #[test]
    fn a_node_s_projects_are_kept_apart_and_keep_their_numbers() {
        let mut store = LocalIssueStore::default();
        assert_eq!(store.create(None, "/p", "core", "", 1).unwrap(), 1);
        assert_eq!(
            store.create(Some("macbook"), "/p", "node", "", 2).unwrap(),
            1
        );
        assert_eq!(
            store
                .create(Some("macbook"), "/p", "node 2", "", 3)
                .unwrap(),
            2
        );
        assert_eq!(store.issue(None, "/p", 2), None);
        assert_eq!(
            store.issue(Some("macbook"), "/p", 2).unwrap().title,
            "node 2"
        );
        assert_eq!(store.project(Some("mini"), "/p"), None);
        assert!(store.set_open(Some("macbook"), "/p", 1, false, 4));
        assert!(store.issue(None, "/p", 1).unwrap().open);
        assert!(
            store
                .update(Some("macbook"), "/p", 2, "renamed", "", 5)
                .unwrap()
        );
        assert!(store.update(None, "/p", 2, "renamed", "", 5).is_err());
        assert_eq!(store.projects.len(), 1);
        assert_eq!(store.devices["macbook"].len(), 1);

        let mut full = LocalIssueStore::default();
        for index in 0..PROJECTS - 1 {
            full.create(None, &format!("/own/{index}"), "t", "", 1)
                .unwrap();
        }
        full.create(Some("macbook"), "/node", "t", "", 1).unwrap();
        assert!(full.create(Some("macbook"), "/node/2", "t", "", 1).is_err());
        assert!(full.create(None, "/own/x", "t", "", 1).is_err());
        assert_eq!(full.create(Some("macbook"), "/node", "t", "", 1), Ok(2));
    }

    /// A store a move left over the cap, the giving machine's projects added
    /// to the new core's, still loads and reads, and its projects keep taking
    /// issues; only a project new to it is refused.
    #[test]
    fn a_store_over_the_cap_loads_and_refuses_only_new_projects() {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("local-issues.json");
        let mut store = LocalIssueStore::default();
        for index in 0..PROJECTS {
            store
                .create(None, &format!("/own/{index}"), "t", "", 1)
                .unwrap();
        }
        store.devices.entry("macbook".into()).or_default().insert(
            "/node".into(),
            LocalIssueProject {
                next_number: 8,
                issues: vec![LocalIssue {
                    number: 7,
                    title: "moved".into(),
                    open: true,
                    ..LocalIssue::default()
                }],
            },
        );
        save(&path, &store).unwrap();
        let mut loaded = load(&path).unwrap();
        assert_eq!(
            loaded.issue(Some("macbook"), "/node", 7).unwrap().title,
            "moved"
        );
        assert_eq!(
            loaded.create(Some("macbook"), "/node", "next", "", 2),
            Ok(8)
        );
        assert_eq!(loaded.create(None, "/own/0", "next", "", 2), Ok(2));
        assert!(loaded.create(None, "/own/new", "t", "", 2).is_err());
        assert!(
            loaded
                .create(Some("macbook"), "/node/new", "t", "", 2)
                .is_err()
        );
    }
}
