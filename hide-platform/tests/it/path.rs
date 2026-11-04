//! The contract of `hide_platform::path`, stated as what a caller observes.
//! The same file runs on macOS, Linux and Windows: a file below a checkout
//! has one relative spelling everywhere, and only names below the root
//! convert.

use std::fs;
use std::path::{Path, PathBuf};

use hide_platform::fs::identity;
use hide_platform::path::{self, PathError, RelPath};

/// A checkout folder with a cased name and `src/main.rs` in it, spelled
/// without a `\\?\` prefix (the spelling a shell and Herdr report).
fn checkout() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = identity::canonical(dir.path()).unwrap().join("Checkout");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src").join("main.rs"), "fn main() {}").unwrap();
    (dir, root)
}

#[test]
fn a_file_below_a_checkout_has_the_same_relative_spelling_on_every_system() {
    let (_dir, root) = checkout();
    let file = root.join("src").join("main.rs");
    let relative = path::relative(&root, &file).unwrap();
    assert_eq!(relative.as_str(), "src/main.rs");
    assert_eq!(root.join(relative.to_native().unwrap()), file);
    assert!(path::relative(&root, &root).unwrap().is_root());
}

#[test]
fn a_root_spelled_with_the_long_path_prefix_relates_to_its_short_spelling() {
    let (_dir, root) = checkout();
    // `std::fs::canonicalize` answers `\\?\C:\...` on Windows and the plain
    // path elsewhere; either spelling of the root holds the same file.
    let long = fs::canonicalize(&root).unwrap();
    let file = root.join("src").join("main.rs");
    assert_eq!(
        path::relative(&long, &file).unwrap().as_str(),
        "src/main.rs"
    );
    assert_eq!(
        path::relative(&root, &long.join("src").join("main.rs"))
            .unwrap()
            .as_str(),
        "src/main.rs"
    );
}

#[test]
fn a_path_that_climbs_out_is_absolute_where_it_should_not_be_or_is_elsewhere_is_refused() {
    let (_dir, root) = checkout();
    assert_eq!(
        path::relative(&root, &root.join("..").join("Checkout").join("src")),
        Err(PathError::NotNormal)
    );
    assert_eq!(
        path::relative(&root, Path::new("src")),
        Err(PathError::NotAbsolute)
    );
    assert_eq!(RelPath::parse("/etc/passwd"), Err(PathError::Absolute));
    assert_eq!(RelPath::parse("../etc"), Err(PathError::NotNormal));
    assert_eq!(
        RelPath::from_native(&root.join("src")),
        Err(PathError::Absolute)
    );
    let sibling = root.with_file_name("Checkout-other").join("a");
    assert_eq!(path::relative(&root, &sibling), Err(PathError::Outside));
    if cfg!(windows) {
        let drive = root.to_str().unwrap().chars().next().unwrap();
        let other = if drive.eq_ignore_ascii_case(&'Z') {
            'Y'
        } else {
            'Z'
        };
        let elsewhere = PathBuf::from(format!(r"{other}:\Checkout\src"));
        assert_eq!(
            path::relative(&root, &elsewhere),
            Err(PathError::OtherVolume)
        );
    }
}

#[test]
fn a_differently_cased_spelling_is_inside_only_on_a_volume_that_ignores_case() {
    let (_dir, root) = checkout();
    let swapped = root.with_file_name("cHECKOUT").join("src").join("main.rs");
    let answer = path::relative(&root, &swapped);
    if identity::case_sensitive(&root).unwrap() {
        assert_eq!(answer, Err(PathError::Outside));
    } else {
        assert_eq!(answer.unwrap().as_str(), "src/main.rs");
    }
}

#[test]
fn an_absolute_path_reads_back_from_its_wire_spelling() {
    let (_dir, root) = checkout();
    let file = root.join("src").join("main.rs");
    let wire = path::to_wire(&file).unwrap();
    assert!(!wire.contains('\\'), "{wire}");
    assert_eq!(path::from_wire(&wire).unwrap(), file);
    if !cfg!(windows) {
        // macOS and Linux keep the spelling they always sent.
        assert_eq!(wire, file.to_str().unwrap());
    }
    let wire_root = path::to_wire(&root).unwrap();
    assert_eq!(
        path::wire_relative(&wire_root, &wire).unwrap(),
        path::relative(&root, &file).unwrap()
    );
    assert_eq!(
        path::wire_join(&wire_root, &RelPath::parse("src/main.rs").unwrap()),
        wire
    );
    assert_eq!(path::to_wire(Path::new("src")), Err(PathError::NotAbsolute));
}

#[test]
fn a_relative_spelling_converts_only_to_names_this_system_can_hold() {
    let walked = Path::new("src").join("main.rs");
    let relative = RelPath::from_native(&walked).unwrap();
    assert_eq!(relative.as_str(), "src/main.rs");
    assert_eq!(relative.to_native().unwrap(), walked);
    for name in [r"a\b", "a:b", "NUL", "name."] {
        let held = RelPath::parse(name).unwrap().to_native();
        if cfg!(windows) {
            assert!(matches!(held, Err(PathError::Unrepresentable(_))), "{name}");
        } else {
            assert_eq!(held.unwrap(), Path::new(name));
        }
    }
}

#[test]
fn a_shell_word_on_a_drive_mount_names_that_drive() {
    for (word, native) in [
        ("/c/work/x/.env", Some(r"C:\work\x\.env")),
        ("/d", Some(r"D:\")),
        ("/c/", Some(r"C:\")),
        ("/cygdrive/e/repo", Some(r"E:\repo")),
        ("/C/x", Some(r"C:\x")),
        ("/cygdrive/c", Some(r"C:\")),
        ("/cygdrive/", None),
        ("//server/share", None),
        ("/cygdrive", None),
        ("/usr/bin", None),
        ("/tmp/x", None),
        ("c/x", None),
        ("C:/x", None),
        ("/1/x", None),
    ] {
        assert_eq!(path::shell_drive_path(word).as_deref(), native, "{word}");
    }
}

/// A folder's parent is read from its spelling on every system: a root keeps
/// its `/`, a drive's `C:/` too on a machine whose paths have no drives, and
/// a root has none.
#[test]
fn a_wire_spelling_s_parent_keeps_its_root_whatever_system_reads_it() {
    for (wire, parent) in [
        ("/work/repo", Some("/work")),
        ("/work/repo/", Some("/work")),
        ("/work", Some("/")),
        ("/", None),
        ("C:/work/repo", Some("C:/work")),
        ("C:/work", Some("C:/")),
        ("C:/", None),
        ("//server/share/work", Some("//server/share")),
        ("//server/share", None),
        ("src/main.rs", Some("src")),
        ("src", None),
    ] {
        assert_eq!(path::wire_parent(wire), parent, "{wire}");
    }
}
