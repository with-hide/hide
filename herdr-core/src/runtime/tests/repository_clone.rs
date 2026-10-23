//! Add a project's Clone from URL through the core: the worker clones a local
//! bare repository and hands the folder to the ordinary registration, the same
//! clone sent twice runs once, and a cancel leaves no folder.

use super::*;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

fn git(dir: &Path, args: &[&str]) {
    // `git commit` starts a detached `git maintenance run --auto`, which can
    // pack and prune the fixture's loose objects while the next `git clone
    // --bare` is still copying them ("failed to copy file to ...").
    // `maintenance.auto=false` stops that run; `gc.auto=0` does the same for a
    // Git older than 2.29, whose commit ran `gc --auto` itself. A signing
    // setting in the account's own Git config never reaches the fixture.
    let status = Command::new("git")
        .args([
            "-c",
            "maintenance.auto=false",
            "-c",
            "gc.auto=0",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A bare repository at `outer/origin.git` and an empty `outer/projects`.
fn fixture() -> (tempfile::TempDir, String, PathBuf) {
    let outer = tempfile::tempdir().unwrap();
    let work = outer.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    git(&work, &["init", "-q", "-b", "main"]);
    for index in 0..100 {
        std::fs::write(
            work.join(format!("f{index}.txt")),
            format!("{index}\n").repeat(40),
        )
        .unwrap();
    }
    git(&work, &["add", "."]);
    git(&work, &["commit", "-q", "-m", "fixture"]);
    let bare = outer.path().join("origin.git");
    git(
        outer.path(),
        &[
            "clone",
            "-q",
            "--bare",
            work.to_str().unwrap(),
            bare.to_str().unwrap(),
        ],
    );
    let parent = outer.path().join("projects");
    std::fs::create_dir_all(&parent).unwrap();
    let url = format!("file://{}", bare.display());
    (outer, url, parent)
}

/// A runtime whose live context points back at itself, as the daemon's does,
/// with a Herdr socket that does not exist.
fn shared_runtime() -> SharedRuntime {
    let shared = SharedRuntime::new(runtime());
    let socket_path = std::env::temp_dir()
        .join(format!(
            "herdr-core-clone-{}-{}.sock",
            std::process::id(),
            NEXT_RUNTIME_STATE_ID.fetch_add(1, Ordering::Relaxed)
        ))
        .to_string_lossy()
        .into_owned();
    shared.lock().unwrap().live = Some(live::LiveContext {
        socket_path: socket_path.clone().into(),
        runtime: shared.weak(),
        notifier: crate::handle::ChangeNotifier::noop(),
        api_connector: Arc::new(hide_herdr_client::LocalSocketConnector::new(&socket_path)),
        node: Arc::new(hide_node::Local::of_process()),
    });
    shared
}

fn clone_event(url: &str, parent: &Path, name: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "schema_version": SCHEMA_VERSION,
        "kind": "clone_repository",
        "payload": {"url": url, "parent": parent.display().to_string(), "name": name}
    }))
    .unwrap()
}

/// Waits for the clone slot to leave `cloning` and `cancelling`.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn settled(shared: &Arc<Mutex<Runtime>>) -> crate::model::RepositoryCloneSnapshot {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let clone = shared
            .lock()
            .unwrap()
            .snapshot
            .repository_clone
            .clone()
            .unwrap();
        if !matches!(clone.phase.as_str(), "cloning" | "cancelling") {
            return clone;
        }
        assert!(
            Instant::now() < deadline,
            "the clone never settled: {clone:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn children(parent: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(parent)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_finished_clone_is_registered_through_create_workspace() {
    let (_outer, url, parent) = fixture();
    let shared = shared_runtime();
    assert!(
        shared
            .lock()
            .unwrap()
            .dispatch_json(&clone_event(&url, &parent, "origin"))
    );
    let clone = settled(&shared);
    let target = parent.join("origin");
    assert_eq!(clone.phase, "finished", "{clone:?}");
    assert_eq!(clone.path, target.display().to_string());
    assert_eq!(clone.host, "localhost");
    assert_eq!(clone.failure, None);
    assert!(target.join("f0.txt").is_file());
    assert_eq!(children(&parent), ["origin"]);
    let guard = shared.lock().unwrap();
    assert!(
        guard
            .snapshot
            .status
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == "workspace.create.requested"
                && diagnostic.message.ends_with(&target.display().to_string())),
        "the folder was not handed to create_workspace: {:?}",
        guard.snapshot.status.diagnostics
    );
}

#[test]
fn the_same_clone_sent_twice_runs_once_and_another_waits() {
    let (outer, url, parent) = fixture();
    let shared = shared_runtime();
    {
        let mut guard = shared.lock().unwrap();
        assert!(guard.dispatch_json(&clone_event(&url, &parent, "origin")));
        let first = guard.snapshot.repository_clone.clone().unwrap();
        // A double click, or a retry of the same intent, joins the running clone.
        assert!(!guard.dispatch_json(&clone_event(&format!(" {url} "), &parent, "origin")));
        assert_eq!(
            guard.snapshot.repository_clone.as_ref().unwrap().id,
            first.id
        );
        assert_eq!(guard.snapshot.status.last_error, None);
        // A different clone is refused until this one settles.
        let other = outer.path().join("elsewhere");
        std::fs::create_dir_all(&other).unwrap();
        assert!(guard.dispatch_json(&clone_event(&url, &other, "origin")));
        assert_eq!(
            guard.snapshot.status.last_error.as_ref().unwrap().kind,
            "repository.clone_busy"
        );
        assert_eq!(
            guard.snapshot.repository_clone.as_ref().unwrap().id,
            first.id
        );
    }
    assert_eq!(settled(&shared).phase, "finished");
    assert_eq!(children(&parent), ["origin"]);
    assert!(children(&outer.path().join("elsewhere")).is_empty());
    let requested = shared
        .lock()
        .unwrap()
        .snapshot
        .status
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.kind == "workspace.create.requested")
        .count();
    assert_eq!(requested, 1, "one clone, one registration");
}

#[test]
fn a_failed_clone_names_its_failure_by_code_and_sends_no_sentence() {
    let (_outer, url, parent) = fixture();
    std::fs::create_dir_all(parent.join("origin")).unwrap();
    let shared = shared_runtime();
    assert!(
        shared
            .lock()
            .unwrap()
            .dispatch_json(&clone_event(&url, &parent, "origin"))
    );
    let clone = settled(&shared);
    assert_eq!(clone.phase, "failed", "{clone:?}");
    assert_eq!(clone.failure.as_deref(), Some("target_exists"));
    assert_eq!(clone.git_line, None);
    let wire = serde_json::to_value(&clone).unwrap();
    assert!(wire.get("message").is_none(), "{wire}");
}

#[test]
fn a_cancelled_clone_leaves_no_folder_and_registers_nothing() {
    let (_outer, url, parent) = fixture();
    let shared = shared_runtime();
    {
        let mut guard = shared.lock().unwrap();
        assert!(guard.dispatch_json(&clone_event(&url, &parent, "origin")));
        let id = guard.snapshot.repository_clone.as_ref().unwrap().id;
        let cancel = serde_json::to_vec(&serde_json::json!({
            "schema_version": SCHEMA_VERSION, "kind": "cancel_repository_clone", "payload": {"id": id}
        }))
        .unwrap();
        assert!(guard.dispatch_json(&cancel));
        assert_eq!(
            guard.snapshot.repository_clone.as_ref().unwrap().phase,
            "cancelling"
        );
    }
    let clone = settled(&shared);
    assert_eq!(clone.phase, "cancelled", "{clone:?}");
    assert!(
        children(&parent).is_empty(),
        "left behind: {:?}",
        children(&parent)
    );
    assert!(
        !shared
            .lock()
            .unwrap()
            .snapshot
            .status
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.kind == "workspace.create.requested")
    );
}

#[test]
fn a_name_the_url_does_not_give_is_refused_before_anything_runs() {
    let (_outer, url, parent) = fixture();
    let shared = shared_runtime();
    let mut guard = shared.lock().unwrap();
    assert!(guard.dispatch_json(&clone_event(&url, &parent, "elsewhere")));
    assert_eq!(
        guard.snapshot.status.last_error.as_ref().unwrap().kind,
        "repository.clone_invalid"
    );
    assert_eq!(guard.snapshot.repository_clone, None);
    assert!(guard.dispatch_json(&clone_event("ext::sh -c id", &parent, "id")));
    assert_eq!(
        guard.snapshot.status.last_error.as_ref().unwrap().kind,
        "repository.clone_invalid"
    );
    assert_eq!(guard.snapshot.repository_clone, None);
    assert!(children(&parent).is_empty());
}
