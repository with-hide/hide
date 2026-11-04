//! The helper answers project facts, never contents, and only for an
//! absolute path; a missing folder is `NotFound`, not a guess.

use hide_host::ErrorCode;
use hide_host::protocol::Call;
use hide_host::serve::handle;

/// A path as the answer spells it, on the wire: the same text on macOS and
/// Linux, `C:/...` on Windows.
fn wire(path: &std::path::Path) -> std::path::PathBuf {
    hide_platform::path::to_wire_lossy(path).into()
}

#[test]
fn a_folder_answers_its_facts_and_a_missing_or_relative_path_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().canonicalize().unwrap().join("plain");
    std::fs::create_dir_all(folder.join("inner")).unwrap();
    let facts: hide_project::ProjectFacts = serde_json::from_value(
        handle(Call::Project {
            path: folder.join("inner").to_string_lossy().into_owned(),
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(facts.kind, hide_project::ProjectKind::Folder);
    assert_eq!(facts.root, wire(&folder.join("inner")));

    let missing = handle(Call::Project {
        path: folder.join("gone").to_string_lossy().into_owned(),
    })
    .unwrap_err();
    assert_eq!(missing.code, ErrorCode::NotFound);
    let relative = handle(Call::Project {
        path: "plain".to_owned(),
    })
    .unwrap_err();
    assert_eq!(relative.code, ErrorCode::InvalidPath);
}

#[test]
fn a_linked_worktree_names_its_repository_as_the_project() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let main = root.join("main");
    let linked = root.join("linked");
    std::fs::create_dir_all(main.join(".git/worktrees/linked")).unwrap();
    std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(&linked).unwrap();
    std::fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", main.join(".git/worktrees/linked").display()),
    )
    .unwrap();
    std::fs::write(
        main.join(".git/worktrees/linked/HEAD"),
        "ref: refs/heads/feature\n",
    )
    .unwrap();
    std::fs::write(main.join(".git/worktrees/linked/commondir"), "../..\n").unwrap();
    std::fs::write(
        main.join(".git/worktrees/linked/gitdir"),
        format!("{}\n", linked.join(".git").display()),
    )
    .unwrap();
    let facts: hide_project::ProjectFacts = serde_json::from_value(
        handle(Call::Project {
            path: linked.to_string_lossy().into_owned(),
        })
        .unwrap(),
    )
    .unwrap();
    assert_eq!(facts.kind, hide_project::ProjectKind::Git);
    assert_eq!(facts.root, wire(&main));
    assert_eq!(facts.checkout_root, wire(&linked));
    assert!(facts.linked_worktree);
    assert_eq!(facts.branch.as_deref(), Some("feature"));
}
