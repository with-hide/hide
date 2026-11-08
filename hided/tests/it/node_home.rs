//! Hide's Home folder (`hide_host::home::sync`, `Call::HomeSync`): one link per
//! registered project, converging on reruns, and never removing anything Hide
//! did not make. Every test works in a temporary directory passed as the
//! account's home; nothing reads the real one.
//!
//! Windows' Home holds no link (`hide_host::home::LINKS`), so these run on
//! Unix; Windows' listing form is `hide_host::home`'s own tests, which the
//! Windows lane runs.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use hide_host::ErrorCode;
use hide_host::home::{HomeLink, HomeSynced, MARKER_FILE, sync};
use hide_host::protocol::{Call, Request};

/// A temporary account home and a folder per project name under `projects/`.
struct Account {
    _dir: tempfile::TempDir,
    home: PathBuf,
}

impl Account {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        Self { _dir: dir, home }
    }

    /// A project folder at `relative` under the account home, with a file.
    fn project(&self, relative: &str) -> String {
        let path = self.home.join("projects").join(relative);
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("keep.txt"), "mine").unwrap();
        path.to_string_lossy().into_owned()
    }

    fn hide(&self) -> PathBuf {
        self.home.join("hide")
    }

    fn sync(&self, projects: &[&str]) -> HomeSynced {
        let projects: Vec<String> = projects.iter().map(|path| (*path).to_owned()).collect();
        sync(&self.home, &projects).unwrap()
    }
}

fn names(synced: &HomeSynced) -> Vec<(&str, &str)> {
    synced
        .links
        .iter()
        .map(|link| (link.name.as_str(), link.target.as_str()))
        .collect()
}

#[test]
fn the_first_sync_makes_home_its_marker_links_and_guide_files() {
    let account = Account::new();
    let alpha = account.project("alpha");
    let beta = account.project("beta");

    let synced = account.sync(&[&beta, &alpha]);

    assert!(synced.created);
    assert_eq!(synced.home, account.hide().to_string_lossy());
    assert_eq!(
        synced.links,
        vec![
            HomeLink {
                name: "alpha".into(),
                target: alpha.clone()
            },
            HomeLink {
                name: "beta".into(),
                target: beta.clone()
            },
        ]
    );
    assert!(synced.dropped.is_empty() && synced.skipped.is_empty());
    assert_eq!(
        std::fs::read_link(account.hide().join("alpha")).unwrap(),
        Path::new(&alpha)
    );
    assert_eq!(
        std::fs::read_to_string(account.hide().join("alpha/keep.txt")).unwrap(),
        "mine"
    );
    assert!(account.hide().join(MARKER_FILE).is_file());
    let guide = std::fs::read_to_string(account.hide().join("AGENTS.md")).unwrap();
    assert!(guide.contains(&format!("- alpha -> {alpha}")), "{guide}");
    assert!(guide.contains("edits here are not kept"), "{guide}");
    assert_eq!(
        std::fs::read_link(account.hide().join("CLAUDE.md")).unwrap(),
        Path::new("AGENTS.md")
    );
    assert_eq!(
        std::fs::read_to_string(account.hide().join("CLAUDE.md")).unwrap(),
        guide
    );
}

#[test]
fn a_rerun_with_the_same_projects_changes_nothing() {
    let account = Account::new();
    let alpha = account.project("alpha");
    let first = account.sync(&[&alpha]);
    let marker = std::fs::read(account.hide().join(MARKER_FILE)).unwrap();

    let second = account.sync(&[&alpha, &alpha]);

    assert!(!second.created);
    assert_eq!(second.links, first.links);
    assert!(second.dropped.is_empty() && second.skipped.is_empty());
    assert_eq!(
        std::fs::read(account.hide().join(MARKER_FILE)).unwrap(),
        marker
    );
}

#[test]
fn syncs_racing_on_one_home_all_succeed_and_the_marker_lists_every_link() {
    let account = Account::new();
    let projects: Vec<String> = (0..8).map(|n| account.project(&format!("p{n}"))).collect();
    let home = account.home.clone();
    let workers: Vec<_> = (0..6)
        .map(|_| {
            let home = home.clone();
            let projects = projects.clone();
            std::thread::spawn(move || sync(&home, &projects))
        })
        .collect();
    for worker in workers {
        let synced = worker.join().unwrap().expect("every racing sync converges");
        assert_eq!(synced.links.len(), projects.len());
    }
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(account.hide().join(MARKER_FILE)).unwrap()).unwrap();
    assert_eq!(marker["links"].as_object().unwrap().len(), projects.len());
    let again = account.sync(&projects.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(!again.created);
    assert!(again.skipped.is_empty(), "{:?}", again.skipped);
}

#[test]
fn names_a_case_insensitive_disk_would_fold_together_are_told_apart() {
    let account = Account::new();
    let upper = account.project("work/App");
    let lower = account.project("play/app");
    let synced = account.sync(&[&upper, &lower]);
    assert_eq!(
        names(&synced),
        [("App-work", upper.as_str()), ("app-play", lower.as_str())]
    );
    assert!(synced.skipped.is_empty(), "{:?}", synced.skipped);
}

#[test]
fn a_project_named_like_homes_own_files_takes_another_name() {
    let account = Account::new();
    let agents = account.project("agents.md");
    let synced = account.sync(&[&agents]);
    assert_eq!(names(&synced), [("agents.md-2", agents.as_str())]);
    assert!(
        account
            .hide()
            .join("AGENTS.md")
            .symlink_metadata()
            .unwrap()
            .is_file()
    );
}

#[test]
fn a_project_whose_path_has_a_control_character_is_skipped_and_kept_out_of_the_guide() {
    let account = Account::new();
    let plain = account.project("plain");
    let odd = account.project("line\nbreak");
    let synced = account.sync(&[&plain, &odd]);
    assert_eq!(names(&synced), [("plain", plain.as_str())]);
    assert_eq!(synced.skipped.len(), 1);
    assert_eq!(synced.skipped[0].target, odd);
    assert_eq!(synced.skipped[0].reason, "control_character");
    let guide = std::fs::read_to_string(account.hide().join("AGENTS.md")).unwrap();
    assert!(!guide.contains("break"), "{guide}");
}

#[test]
fn a_parent_suffix_too_long_for_the_disk_falls_back_to_the_folder_name() {
    let account = Account::new();
    // `app-` and 252 more bytes is one past the 255 a file name may take.
    let long = account.project(&format!("{}/app", "p".repeat(252)));
    let short = account.project("short/app");
    let synced = account.sync(&[&long, &short]);
    assert_eq!(
        names(&synced),
        [("app", long.as_str()), ("app-short", short.as_str())]
    );
    assert!(synced.skipped.is_empty(), "{:?}", synced.skipped);
}

#[cfg(unix)]
#[test]
fn home_and_its_files_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let account = Account::new();
    let alpha = account.project("alpha");
    account.sync(&[&alpha]);
    let mode = |path: PathBuf| path.metadata().unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(account.hide()), 0o700);
    assert_eq!(mode(account.hide().join(MARKER_FILE)), 0o600);
    assert_eq!(mode(account.hide().join("AGENTS.md")), 0o600);
}

#[test]
fn removing_a_project_drops_only_its_link() {
    let account = Account::new();
    let alpha = account.project("alpha");
    let beta = account.project("beta");
    account.sync(&[&alpha, &beta]);

    let synced = account.sync(&[&alpha]);

    assert_eq!(names(&synced), vec![("alpha", alpha.as_str())]);
    assert_eq!(synced.dropped.len(), 1);
    assert_eq!(synced.dropped[0].name, "beta");
    assert_eq!(synced.dropped[0].target, beta);
    assert_eq!(synced.dropped[0].reason, "unregistered");
    assert!(
        std::fs::symlink_metadata(account.hide().join("beta")).is_err(),
        "the link stays"
    );
    assert_eq!(
        std::fs::read_to_string(Path::new(&beta).join("keep.txt")).unwrap(),
        "mine"
    );
}

#[test]
fn projects_that_share_a_folder_name_are_told_apart_by_their_parent() {
    let account = Account::new();
    let first = account.project("x/app");
    let second = account.project("y/app");

    let synced = account.sync(&[&second, &first]);

    assert_eq!(
        names(&synced),
        vec![("app-x", first.as_str()), ("app-y", second.as_str())]
    );
}

#[test]
fn a_name_that_still_repeats_takes_a_number_in_path_order() {
    let account = Account::new();
    let first = account.project("a/x/app");
    let second = account.project("b/x/app");

    let synced = account.sync(&[&second, &first]);

    assert_eq!(
        names(&synced),
        vec![("app-x", first.as_str()), ("app-x-2", second.as_str())]
    );
}

#[test]
fn a_project_folder_that_moved_away_loses_its_link_as_dangling() {
    let account = Account::new();
    let alpha = account.project("alpha");
    let beta = account.project("beta");
    account.sync(&[&alpha, &beta]);
    std::fs::rename(&beta, account.home.join("moved")).unwrap();

    let synced = account.sync(&[&alpha, &beta]);

    assert_eq!(names(&synced), vec![("alpha", alpha.as_str())]);
    assert_eq!(synced.dropped.len(), 1);
    assert_eq!(synced.dropped[0].name, "beta");
    assert_eq!(synced.dropped[0].reason, "dangling");
    assert_eq!(synced.skipped.len(), 1);
    assert_eq!(synced.skipped[0].target, beta);
    assert_eq!(synced.skipped[0].reason, "missing");
    assert!(std::fs::symlink_metadata(account.hide().join("beta")).is_err());
    assert!(account.home.join("moved/keep.txt").is_file());
}

#[test]
fn something_the_operator_keeps_at_a_link_name_is_left_alone() {
    let account = Account::new();
    let alpha = account.project("alpha");
    account.sync(&[]);
    std::fs::create_dir(account.hide().join("alpha")).unwrap();
    std::fs::write(account.hide().join("alpha/theirs.txt"), "x").unwrap();
    std::fs::write(account.hide().join("notes.txt"), "notes").unwrap();

    for _ in 0..2 {
        let synced = account.sync(&[&alpha]);
        assert!(synced.links.is_empty());
        assert_eq!(synced.skipped.len(), 1);
        assert_eq!(synced.skipped[0].target, alpha);
        assert_eq!(synced.skipped[0].reason, "name_taken");
        assert!(account.hide().join("alpha/theirs.txt").is_file());
        assert_eq!(
            std::fs::read_to_string(account.hide().join("notes.txt")).unwrap(),
            "notes"
        );
    }
}

#[test]
fn a_link_the_operator_replaced_with_a_folder_is_forgotten_not_deleted() {
    let account = Account::new();
    let alpha = account.project("alpha");
    account.sync(&[&alpha]);
    std::fs::remove_file(account.hide().join("alpha")).unwrap();
    std::fs::create_dir(account.hide().join("alpha")).unwrap();
    std::fs::write(account.hide().join("alpha/theirs.txt"), "x").unwrap();

    let synced = account.sync(&[]);

    assert!(synced.links.is_empty() && synced.dropped.is_empty());
    assert!(account.hide().join("alpha/theirs.txt").is_file());
    let marker = std::fs::read_to_string(account.hide().join(MARKER_FILE)).unwrap();
    assert!(!marker.contains("alpha"), "{marker}");
}

#[test]
fn a_link_the_operator_pointed_elsewhere_is_not_removed() {
    let account = Account::new();
    let alpha = account.project("alpha");
    let other = account.project("other");
    account.sync(&[&alpha]);
    let entry = account.hide().join("alpha");
    std::fs::remove_file(&entry).unwrap();
    hide_platform::fs::link::create_link(Path::new(&other), &entry).unwrap();

    let synced = account.sync(&[]);

    assert!(synced.dropped.is_empty());
    assert!(hide_platform::fs::link::is_link_to(
        &entry,
        Path::new(&other)
    ));
}

#[test]
fn a_second_namesake_renames_the_first_projects_link() {
    let account = Account::new();
    let first = account.project("a/app");
    let second = account.project("b/app");
    account.sync(&[&first]);

    let synced = account.sync(&[&first, &second]);

    // `app` became `app-a`; the old name is Hide's to remove.
    assert_eq!(
        names(&synced),
        vec![("app-a", first.as_str()), ("app-b", second.as_str())]
    );
    assert_eq!(synced.dropped.len(), 1);
    assert_eq!(synced.dropped[0].name, "app");
    assert!(std::fs::symlink_metadata(account.hide().join("app")).is_err());
}

#[test]
fn a_link_whose_name_now_belongs_to_another_folder_is_replaced() {
    let account = Account::new();
    let first = account.project("a/app");
    let second = account.project("b/app");
    account.sync(&[&first]);

    let synced = account.sync(&[&second]);

    assert_eq!(names(&synced), vec![("app", second.as_str())]);
    assert!(synced.dropped.is_empty());
    assert_eq!(
        std::fs::read_link(account.hide().join("app")).unwrap(),
        Path::new(&second)
    );
    assert!(Path::new(&first).join("keep.txt").is_file());
}

#[test]
fn a_folder_named_hide_that_is_not_homes_is_refused_and_untouched() {
    let empty = Account::new();
    std::fs::create_dir(empty.hide()).unwrap();
    let alpha = empty.project("alpha");
    let error = sync(&empty.home, std::slice::from_ref(&alpha)).unwrap_err();
    assert_eq!(error.code, ErrorCode::HomeConflict);
    assert_eq!(
        error.message,
        "~/hide already exists and is not Hide's Home. Rename or move it, then open Home again."
    );
    assert_eq!(std::fs::read_dir(empty.hide()).unwrap().count(), 0);

    let file = Account::new();
    std::fs::write(file.hide(), "mine").unwrap();
    let error = sync(&file.home, &[]).unwrap_err();
    assert_eq!(error.code, ErrorCode::HomeConflict);
    assert_eq!(std::fs::read_to_string(file.hide()).unwrap(), "mine");

    let link = Account::new();
    let target = link.project("alpha");
    hide_platform::fs::link::create_link(Path::new(&target), &link.hide()).unwrap();
    assert_eq!(
        sync(&link.home, &[]).unwrap_err().code,
        ErrorCode::HomeConflict
    );
    assert!(std::fs::read_dir(&target).unwrap().count() == 1);
}

#[test]
fn an_unreadable_marker_is_a_conflict() {
    for marker in [
        "not json",
        r#"{"version":2,"links":{}}"#,
        r#"{"version":1,"links":{"../escape":"/x"}}"#,
        r#"{"version":1,"links":{"AGENTS.md":"/x"}}"#,
    ] {
        let account = Account::new();
        std::fs::create_dir(account.hide()).unwrap();
        std::fs::write(account.hide().join(MARKER_FILE), marker).unwrap();
        let error = sync(&account.home, &[]).unwrap_err();
        assert_eq!(error.code, ErrorCode::HomeConflict, "{marker}");
        assert!(error.message.contains("unreadable"), "{}", error.message);
        assert!(!account.hide().join("AGENTS.md").exists());
    }
}

#[test]
fn a_claude_file_the_operator_made_is_left_and_the_guide_is_rewritten() {
    let account = Account::new();
    account.sync(&[]);
    std::fs::remove_file(account.hide().join("CLAUDE.md")).unwrap();
    std::fs::write(account.hide().join("CLAUDE.md"), "theirs").unwrap();
    std::fs::write(account.hide().join("AGENTS.md"), "edited").unwrap();

    account.sync(&[]);

    assert_eq!(
        std::fs::read_to_string(account.hide().join("CLAUDE.md")).unwrap(),
        "theirs"
    );
    assert!(
        std::fs::read_to_string(account.hide().join("AGENTS.md"))
            .unwrap()
            .starts_with("# Hide Home")
    );
}

#[test]
fn paths_that_cannot_be_linked_are_skipped_with_their_reason() {
    let account = Account::new();
    let file = account.home.join("file.txt");
    std::fs::write(&file, "x").unwrap();
    let file = file.to_string_lossy().into_owned();

    let synced = account.sync(&["relative/dir", &file, "/"]);

    let reasons: Vec<(&str, &str)> = synced
        .skipped
        .iter()
        .map(|skip| (skip.target.as_str(), skip.reason.as_str()))
        .collect();
    assert_eq!(
        reasons,
        vec![
            ("relative/dir", "not_absolute"),
            ("/", "no_name"),
            (file.as_str(), "missing"),
        ]
    );
    assert!(synced.links.is_empty());
}

#[test]
fn more_projects_than_the_cap_write_nothing() {
    let account = Account::new();
    let projects: Vec<String> = (0..=hide_host::home::MAX_LINKS)
        .map(|number| format!("/nowhere/{number}"))
        .collect();

    let error = sync(&account.home, &projects).unwrap_err();

    assert_eq!(error.code, ErrorCode::TooLarge);
    assert!(error.message.contains("256"), "{}", error.message);
    assert!(!account.hide().exists());
}

#[test]
fn home_sync_round_trips_through_the_helper() {
    let request = Request {
        id: 7,
        call: Call::HomeSync {
            projects: vec!["/a".into()],
        },
    };
    let line = serde_json::to_string(&request).unwrap();
    assert_eq!(line, r#"{"id":7,"op":"home_sync","projects":["/a"]}"#);
    assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), request);

    // The helper reads HOME itself; run it as a process so the test never
    // edits this one's environment.
    let account = Account::new();
    let alpha = account.project("alpha");
    let answer = ask_helper(Some(&account.home), std::slice::from_ref(&alpha));
    let synced: HomeSynced = serde_json::from_value(answer["ok"].clone()).unwrap();
    assert!(synced.created);
    assert_eq!(names(&synced), vec![("alpha", alpha.as_str())]);
    assert!(account.hide().join("alpha").exists());

    let refused = ask_helper(None, &[]);
    assert_eq!(refused["error"]["code"], "unsupported");
    // A relative HOME would put Home under whatever folder the helper runs in.
    let relative = ask_helper(Some(Path::new("relative-home")), &[]);
    assert_eq!(relative["error"]["code"], "unsupported");
    assert!(!Path::new("relative-home").exists());
}

/// One `home_sync` request answered by `hided node serve`, whose HOME is
/// `home` or absent.
fn ask_helper(home: Option<&Path>, projects: &[String]) -> serde_json::Value {
    let mut command = Command::new(env!("CARGO_BIN_EXE_hided"));
    command
        .args(["node", "serve"])
        .env_remove("HOME")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());
    if let Some(home) = home {
        command.env("HOME", home);
    }
    let mut child = command.spawn().unwrap();
    let request = serde_json::json!({"id": 1, "op": "home_sync", "projects": projects});
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{request}").unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    drop(stdin);
    child.wait().unwrap();
    serde_json::from_str(&line).unwrap()
}
