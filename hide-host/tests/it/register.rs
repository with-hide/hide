//! Which folders a host lets be registered as projects (`Call::Registrable`):
//! a folder inside its own home, or outside it with the operator's
//! confirmation of that path, named by the project it belongs to.

use std::path::Path;
use std::process::Command;

use hide_host::ErrorCode;
use hide_host::register::check;

#[test]
fn a_folder_inside_home_is_registered_as_its_project() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path().canonicalize().unwrap();
    let folder = home.join("notes");
    std::fs::create_dir(&folder).unwrap();
    let repository = home.join("repo");
    std::fs::create_dir_all(repository.join("src")).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(&repository)
            .status()
            .unwrap()
            .success()
    );

    let plain = check(&folder, None, &home).unwrap();
    assert_eq!(plain.root, folder.to_string_lossy());
    assert!(!plain.is_git);
    let inside = check(&repository.join("src"), None, &home).unwrap();
    assert_eq!(inside.root, repository.to_string_lossy());
    assert!(inside.is_git);
}

#[test]
fn home_itself_a_folder_outside_it_and_a_file_are_refused() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let file = home.path().join("file.txt");
    std::fs::write(&file, "x").unwrap();

    let code = |path: &Path| check(path, None, home.path()).unwrap_err().code;
    assert_eq!(code(home.path()), ErrorCode::OutsideRoot);
    assert_eq!(code(elsewhere.path()), ErrorCode::NeedsConfirmation);
    assert_eq!(code(&file), ErrorCode::NotADirectory);
    assert_eq!(code(&home.path().join("missing")), ErrorCode::NotFound);
    assert_eq!(code(Path::new("relative")), ErrorCode::InvalidPath);
    let dotdot = home.path().join("..").join("x");
    assert_eq!(code(&dotdot), ErrorCode::InvalidPath);
}

/// PRD wsl-device B15, B18, B20: a folder outside the host's home is
/// registered only with the operator's confirmation of that same path, which
/// nothing remembers; it is not read before then.
#[test]
fn a_folder_outside_home_needs_its_own_confirmation() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let app = elsewhere.path().join("app");
    let other = elsewhere.path().join("other");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::create_dir(&other).unwrap();
    let code = |path: &Path, confirmed: Option<&Path>| {
        check(path, confirmed, home.path()).unwrap_err().code
    };

    let added = check(&app, Some(&app), home.path()).unwrap();
    assert_eq!(
        added.root,
        hide_platform::fs::identity::canonical(&app)
            .unwrap()
            .to_string_lossy()
    );
    assert!(!added.is_git);
    for (path, confirmed) in [
        (&other, Some(app.as_path())),
        (&app.join("src"), Some(app.as_path())),
        (&app, Some(app.join("src").as_path())),
        (&app, Some(Path::new("relative"))),
        (&app, None),
    ] {
        assert_eq!(
            code(path, confirmed),
            ErrorCode::NeedsConfirmation,
            "{path:?} confirmed as {confirmed:?}"
        );
    }
    // Not read before it is confirmed: a missing folder asks the same way.
    let missing = elsewhere.path().join("missing");
    assert_eq!(code(&missing, None), ErrorCode::NeedsConfirmation);
    assert_eq!(code(&missing, Some(&missing)), ErrorCode::NotFound);
}

/// PRD wsl-device B18: a confirmation changes nothing about a symlink under
/// home that leads out of it, or about home and the folders holding it.
#[cfg(unix)]
#[test]
fn a_confirmation_does_not_reach_past_home_s_own_refusals() {
    let outer = tempfile::tempdir().unwrap();
    let home = outer.path().join("home");
    let elsewhere = outer.path().join("elsewhere");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let escape = home.join("escape");
    std::os::unix::fs::symlink(&elsewhere, &escape).unwrap();
    let code = |path: &Path| check(path, Some(path), &home).unwrap_err().code;
    assert_eq!(code(&escape), ErrorCode::OutsideRoot);
    for holder in [home.as_path(), outer.path(), Path::new("/")] {
        assert_eq!(code(holder), ErrorCode::OutsideRoot, "{holder:?}");
    }
    let up = elsewhere.join("up");
    std::os::unix::fs::symlink(outer.path(), &up).unwrap();
    assert_eq!(code(&up), ErrorCode::OutsideRoot);
}

/// A repository at `main` with a linked worktree at `linked`, as Git writes
/// the two (`hide_project` reads them without a git process).
fn repository_with_worktree(main: &Path, linked: &Path) {
    let admin = main.join(".git/worktrees/linked");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(main.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::create_dir_all(linked).unwrap();
    std::fs::write(
        linked.join(".git"),
        format!("gitdir: {}\n", admin.display()),
    )
    .unwrap();
    std::fs::write(admin.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
    std::fs::write(admin.join("commondir"), "../..\n").unwrap();
    std::fs::write(
        admin.join("gitdir"),
        format!("{}\n", linked.join(".git").display()),
    )
    .unwrap();
}

/// PRD wsl-device B15: a registration adds the project a folder belongs to,
/// so a folder inside a repository outside home, or a linked worktree of one
/// wherever it is, is asked about once, by that repository's root
/// (`confirm`), and the confirmation of that root registers it; the
/// confirmation of the folder alone does not.
#[test]
fn a_folder_in_a_project_outside_home_asks_about_the_project_it_adds() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let repo = elsewhere.path().join("repo");
    let linked = elsewhere.path().join("away/linked");
    repository_with_worktree(&repo, &linked);
    let public = repo.join("public");
    std::fs::create_dir(&public).unwrap();
    let root = hide_platform::fs::identity::canonical(&repo)
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let asked = |path: &Path, confirmed: Option<&Path>| {
        let error = check(path, confirmed, home.path()).unwrap_err();
        (error.code, error.confirm)
    };

    for folder in [&public, &linked] {
        for confirmed in [None, Some(folder.as_path())] {
            assert_eq!(
                asked(folder, confirmed),
                (ErrorCode::NeedsConfirmation, Some(root.clone())),
                "{folder:?} confirmed as {confirmed:?}"
            );
        }
        let added = check(folder, Some(Path::new(&root)), home.path()).unwrap();
        assert_eq!(added.root, root, "{folder:?}");
        assert!(added.is_git);
    }
    assert_eq!(check(&repo, Some(&repo), home.path()).unwrap().root, root);

    // A worktree under home whose repository is outside it asks too.
    let other = elsewhere.path().join("other");
    let inward = home.path().join("inward");
    repository_with_worktree(&other, &inward);
    assert_eq!(
        asked(&inward, None),
        (
            ErrorCode::NeedsConfirmation,
            Some(
                hide_platform::fs::identity::canonical(&other)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            )
        )
    );
}

/// A folder on a drive that is a link to a network share is judged by where
/// it leads: no confirmation makes the share a project.
#[cfg(windows)]
#[test]
fn a_confirmed_link_to_a_network_share_is_refused_like_the_share() {
    let home = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let shared = elsewhere.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    // The same folder through the drive's administrative share.
    let local = hide_platform::fs::identity::canonical(&shared).unwrap();
    let local = local.to_string_lossy();
    let share = format!(r"\\localhost\{}${}", &local[..1], &local[2..]);
    let link = elsewhere.path().join("share-link");
    std::os::windows::fs::symlink_dir(&share, &link).unwrap();
    let refused = check(&link, Some(&link), home.path()).unwrap_err();
    assert_eq!(
        refused.code,
        ErrorCode::OutsideRoot,
        "{share}: {}",
        refused.message
    );
}

/// Where a path stands against home is judged by what it is, not by how it
/// is spelled: macOS's `/System/Volumes/Data` spelling of home is home, and
/// the folder holding it all holds home.
#[cfg(target_os = "macos")]
#[test]
fn another_spelling_of_home_is_home() {
    let outer = tempfile::tempdir().unwrap();
    let home = hide_platform::fs::identity::canonical(outer.path())
        .unwrap()
        .join("home");
    std::fs::create_dir_all(home.join("app")).unwrap();
    let data = Path::new("/System/Volumes/Data");
    let spelled = data.join(home.strip_prefix("/").unwrap());
    assert!(
        spelled.join("app").is_dir(),
        "{spelled:?} is the same folder"
    );
    for holder in [spelled.as_path(), data] {
        assert_eq!(
            check(holder, Some(holder), &home).unwrap_err().code,
            ErrorCode::OutsideRoot,
            "{holder:?}"
        );
    }
    // Registered by the spelling the system gives the folder.
    let added = check(&spelled.join("app"), None, &home).unwrap();
    assert_eq!(added.root, home.join("app").to_string_lossy());
}

/// On a volume that ignores letter case, home written in another case is
/// still home: a folder under it needs no confirmation, and a link under it
/// that leads out stays refused.
#[test]
fn home_in_another_letter_case_is_home_where_case_is_ignored() {
    let outer = tempfile::tempdir().unwrap();
    let home = outer.path().join("Home");
    std::fs::create_dir_all(home.join("app")).unwrap();
    if hide_platform::fs::identity::case_sensitive(outer.path()).unwrap() {
        return;
    }
    let shouted = outer.path().join("HOME");
    assert!(check(&shouted.join("app"), None, &home).is_ok());
    assert_eq!(
        check(&shouted, Some(&shouted), &home).unwrap_err().code,
        ErrorCode::OutsideRoot
    );
}
