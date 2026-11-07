//! The projects worth adding on a node (`Call::ImportableProjects`, PRD
//! device-add-project D-09 to D-14): agents' folders, Paseo's and Herdr's
//! projects, merged into the projects a registration would add.

use std::collections::BTreeSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use hide_host::importable::{IMPORTABLE_CAP, ImportableProjects, find};

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

/// `unix_ms` as an RFC 3339 time in UTC, the way Paseo writes `updatedAt`.
fn rfc3339(unix_ms: u64) -> String {
    let seconds = unix_ms / 1000;
    let (days, rest) = ((seconds / 86_400) as i64, seconds % 86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

struct Home {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

impl Home {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("home");
        std::fs::create_dir(&path).unwrap();
        Self { _dir: dir, path }
    }

    fn variables(&self) -> impl Fn(&str) -> Option<OsString> + '_ {
        move |name| match name {
            "HOME" => Some(self.path.clone().into_os_string()),
            "XDG_CONFIG_HOME" => Some(self.path.join(".config").into_os_string()),
            _ => None,
        }
    }

    fn find(&self, exclude: &[String]) -> ImportableProjects {
        find(&self.path, &self.variables(), now_ms(), exclude)
    }

    fn folder(&self, relative: &str) -> PathBuf {
        let path = self.path.join(relative);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.path.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn claude(&self, name: &str, cwd: &Path) -> PathBuf {
        self.write(
            &format!(".claude/projects/p/{name}.jsonl"),
            &format!(
                "{{\"type\":\"user\",\"cwd\":{},\"timestamp\":\"2026-11-07T01:00:00Z\",\"message\":{{\"role\":\"user\",\"content\":\"hi\"}}}}\n",
                serde_json::to_string(cwd).unwrap()
            ),
        )
    }

    fn codex(&self, name: &str, cwd: &Path) -> PathBuf {
        self.write(
            &format!(".codex/sessions/2026/11/07/{name}.jsonl"),
            &format!(
                "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{name}\",\"cwd\":{}}}}}\n",
                serde_json::to_string(cwd).unwrap()
            ),
        )
    }

    fn paseo(&self, records: &[(&Path, u64, bool)]) -> PathBuf {
        let records = records
            .iter()
            .enumerate()
            .map(|(index, (root, updated, archived))| {
                serde_json::json!({
                    "projectId": format!("prj_{index}"), "rootPath": root, "kind": "git",
                    "displayName": "x", "projectKey": null, "customName": null,
                    "createdAt": rfc3339(*updated), "updatedAt": rfc3339(*updated),
                    "archivedAt": archived.then(|| rfc3339(*updated)),
                })
            })
            .collect::<Vec<_>>();
        self.write(
            ".paseo/projects/projects.json",
            &serde_json::to_string_pretty(&records).unwrap(),
        )
    }

    fn herdr(&self, text: &str) -> PathBuf {
        self.write(".config/herdr/session.json", text)
    }
}

/// A repository at `main` with a linked worktree at `linked`, as Git writes
/// the two (`hide_project` reads them without a git process).
fn repository_with_worktree(main: &Path, linked: &Path) {
    let admin = main.join(".git/worktrees/linked");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(linked).unwrap();
    std::fs::write(linked.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
    std::fs::write(admin.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    std::fs::write(
        admin.join("gitdir"),
        format!("{}\n", linked.join(".git").display()),
    )
    .unwrap();
}

fn wire(path: &Path) -> String {
    hide_platform::path::to_wire_lossy(path)
}

fn rows(found: &ImportableProjects) -> Vec<(String, BTreeSet<String>)> {
    found
        .projects
        .iter()
        .map(|project| {
            let sources = project
                .sources
                .iter()
                .map(|source| {
                    serde_json::to_value(source)
                        .unwrap()
                        .as_str()
                        .unwrap()
                        .to_owned()
                })
                .collect();
            (project.label.clone(), sources)
        })
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

/// B20, B21, B23, B26: a repository worked in from a linked worktree and a
/// subfolder is one row carrying each agent; a folder Paseo and Herdr both
/// keep is one row carrying both; home, a folder outside home, a folder
/// that is gone, an archived Paseo project, a project older than thirty days
/// and a registered one are not rows; and every file read is the same byte
/// for byte afterwards.
#[test]
fn every_source_merges_into_the_projects_a_registration_would_add() {
    let home = Home::new();
    let outside = tempfile::tempdir().unwrap();
    let app = home.path.join("work/app");
    let linked = home.path.join("work/app-feature");
    repository_with_worktree(&app, &linked);
    let site = home.folder("work/site");
    let registered = home.folder("work/registered");
    let archived = home.folder("work/archived");
    let old = home.folder("work/old");
    let now = now_ms();
    let read = [
        home.claude("in-worktree", &linked),
        home.claude("in-home", &home.path),
        home.claude("registered", &registered),
        home.codex("in-subfolder", &home.folder("work/app/src")),
        home.paseo(&[
            (&site, now - DAY_MS, false),
            (&home.path.join("work/gone"), now, false),
            (&archived, now, true),
            (&old, now - 90 * DAY_MS, false),
            (outside.path(), now, false),
        ]),
        home.herdr(
            &serde_json::json!({"version": 3, "workspaces": [
                {"id": "w1", "identity_cwd": site, "tabs": [],
                 "worktree_space": {"key": "k", "label": "site", "repo_root": site,
                                    "checkout_path": site, "is_linked_worktree": false}},
                {"id": "w2", "identity_cwd": home.path, "tabs": []},
            ]})
            .to_string(),
        ),
    ];
    // Herdr keeps no time per workspace: its save is as recent as its file.
    std::fs::File::options()
        .write(true)
        .open(&read[5])
        .unwrap()
        .set_modified(SystemTime::now() - std::time::Duration::from_secs(2 * 24 * 60 * 60))
        .unwrap();
    let before = read
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect::<Vec<_>>();

    let found = home.find(&[wire(&registered)]);

    assert_eq!(
        rows(&found),
        vec![
            ("~/work/app".to_owned(), set(&["claude", "codex"])),
            ("~/work/site".to_owned(), set(&["paseo", "herdr"])),
        ]
    );
    assert_eq!(found.projects[0].path, wire(&app));
    assert_eq!(found.projects[0].name, "app");
    assert!(found.dropped.is_empty(), "{:?}", found.dropped);
    let after = read
        .iter()
        .map(|path| std::fs::read(path).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(before, after);
}

/// B27: a Paseo registry and a Herdr session that cannot be read drop out
/// with why, and the agents' projects are still listed.
#[test]
fn a_source_that_cannot_be_read_drops_out_alone() {
    let home = Home::new();
    let app = home.folder("work/app");
    home.claude("one", &app);
    home.write(".paseo/projects/projects.json", "{\"not\": \"an array\"}");
    home.herdr("{\"version\": 9, \"workspaces\": []}");

    let found = home.find(&[]);

    assert_eq!(
        rows(&found),
        vec![("~/work/app".to_owned(), set(&["claude"]))]
    );
    let dropped = found
        .dropped
        .iter()
        .map(|source| source.source.as_str())
        .collect::<Vec<_>>();
    assert_eq!(dropped, vec!["paseo", "herdr"]);
    assert!(found.dropped.iter().all(|source| !source.reason.is_empty()));
}

/// B22: at most twenty rows, the newest first.
#[test]
fn the_answer_holds_the_twenty_newest_projects() {
    let home = Home::new();
    let now = now_ms();
    let folders = (0..IMPORTABLE_CAP + 5)
        .map(|index| home.folder(&format!("work/p{index:02}")))
        .collect::<Vec<_>>();
    let records = folders
        .iter()
        .enumerate()
        .map(|(index, folder)| (folder.as_path(), now - (index as u64 + 1) * 60_000, false))
        .collect::<Vec<_>>();
    home.paseo(&records);

    let found = home.find(&[]);

    let labels = found
        .projects
        .iter()
        .map(|project| project.label.clone())
        .collect::<Vec<_>>();
    let expected = (0..IMPORTABLE_CAP)
        .map(|index| format!("~/work/p{index:02}"))
        .collect::<Vec<_>>();
    assert_eq!(labels, expected);
}
