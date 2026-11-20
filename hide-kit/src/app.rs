//! Whether a state folder's core is the desktop app's to update (PRD
//! device-connect-iroh B23).
//!
//! A Hide started on a machine over a login runs with no app, and the
//! machine that reaches it keeps it at its own build. From the day the app
//! is opened on that account the app updates the core, and a core a paired
//! machine raised past the app's build would refuse the app's own
//! connection. So the app says it is there: its packaged `hide connect`
//! notes the package it runs from in the state folder it opens
//! ([`note`]), and a raise reads the note ([`present`]).
//!
//! The note is the app's own word and nothing else's. The app's kit pass
//! cannot give it: on a machine whose core a login item runs, the app's
//! `hide connect` updates that core and no daemon ever runs from the
//! package, so no kit is applied as the app's. Nor can the account's `hide`
//! command: a device helper that holds the name keeps it while its own
//! command is there. It is kept by state folder, so an app opened on a
//! folder of its own says nothing of the account's other cores.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What [`note`] keeps: the resources folder of the package whose `hide`
/// last opened the state folder.
#[derive(Debug, Deserialize, Serialize)]
struct Note {
    package: PathBuf,
}

/// Notes that the app whose package resources are `package` opened
/// `state_dir`'s core; answers whether the note changed. Nothing is written
/// when the note already names the package.
pub fn note(state_dir: &Path, package: &Path) -> Result<bool, String> {
    let path = crate::layout::app_note(state_dir);
    if read(&path)?.is_some_and(|noted| noted.package == package) {
        return Ok(false);
    }
    let note = Note {
        package: package.to_path_buf(),
    };
    let mut bytes = serde_json::to_vec(&note).map_err(|error| error.to_string())?;
    bytes.push(b'\n');
    crate::write_atomically(&path, &bytes, hide_platform::fs::Access::Private)?;
    Ok(true)
}

/// Whether an app opened `state_dir`'s core and its package is still
/// there: the folder the note names is one a desktop package's programs run
/// from ([`crate::bundled_kit_dir`]). A package since removed says nothing,
/// so the core is a paired machine's to update again.
pub fn present(state_dir: &Path) -> Result<bool, String> {
    Ok(
        read(&crate::layout::app_note(state_dir))?.is_some_and(|noted| {
            noted.package.is_dir() && crate::bundled_kit_dir(&noted.package.join("hided")).is_some()
        }),
    )
}

/// The note at `path`; none when no app opened the folder. A note that
/// cannot be read is an error, never read as no app.
fn read(path: &Path) -> Result<Option<Note>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{} could not be read: {error}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| format!("{} is not an app's note: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A desktop package's resources folder as this system lays one out
    /// (`crate::bundled_kit_dir`), under `root`.
    fn package(root: &Path) -> PathBuf {
        if cfg!(target_os = "macos") {
            let resources = root.join("Hide.app").join("Contents").join("Resources");
            std::fs::create_dir_all(&resources).unwrap();
            resources
        } else {
            let resources = root.join("resources");
            std::fs::create_dir_all(&resources).unwrap();
            std::fs::write(resources.join("app.asar"), "").unwrap();
            let shell = format!("hide{}", std::env::consts::EXE_SUFFIX);
            std::fs::write(root.join(shell), "").unwrap();
            resources
        }
    }

    // B23: a state folder is the app's once the app noted its package
    // there, for as long as the package stands; a folder that is no
    // package's, such as a helper root's build, is never an app.
    #[test]
    fn a_folder_is_the_apps_while_the_package_it_noted_is_there() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        assert_eq!(present(&state), Ok(false));

        let helper = dir.path().join("host-helper").join("0123456789abcdef");
        std::fs::create_dir_all(&helper).unwrap();
        assert_eq!(note(&state, &helper), Ok(true));
        assert_eq!(present(&state), Ok(false));

        let package = package(&dir.path().join("app"));
        assert_eq!(note(&state, &package), Ok(true));
        assert_eq!(note(&state, &package), Ok(false), "the same package again");
        assert_eq!(present(&state), Ok(true));
        // Another folder's core is not this app's.
        let other = dir.path().join("other-state");
        std::fs::create_dir_all(&other).unwrap();
        assert_eq!(present(&other), Ok(false));

        std::fs::remove_dir_all(dir.path().join("app")).unwrap();
        assert_eq!(present(&state), Ok(false));
    }

    // A note that cannot be read is not read as "no app": the raise that
    // asked is refused rather than let through.
    #[test]
    fn a_note_that_cannot_be_read_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(crate::layout::app_note(dir.path()), "not json").unwrap();
        assert!(present(dir.path()).is_err());
    }
}
