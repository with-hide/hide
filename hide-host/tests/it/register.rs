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
