//! Reading the operator's agent definition folder
//! ([`layout::agent_definitions`](crate::layout::agent_definitions)) into
//! the catalog `hide_agent_adapter::definition` checks (PRD
//! engine-bundle-3-core-owns D-17).
//!
//! The folder is read whole each time it is asked for, on the caller's
//! thread, which is never the core's lock: no watch, no cache. Only `.toml`
//! files directly in it count, in name order; the first
//! [`MAX_USER_FILES`] are read when each is at most [`MAX_FILE_BYTES`], and
//! every other is named as left out without being opened, so a full folder
//! costs at most 64 reads of 256 KiB. A file whose name is not UTF-8 is
//! named as unreadable, with the bytes it cannot spell replaced, rather than
//! passed over in silence.

use std::path::Path;

use hide_agent_adapter::definition::{Catalog, MAX_FILE_BYTES, MAX_USER_FILES, UserFile};

/// The built-in agents and the operator's files in `home`'s folder. A
/// missing folder holds no files; a folder that cannot be listed is the
/// error, which leaves the caller with the built-in agents and the reason.
pub fn read(home: &Path) -> Result<Catalog, String> {
    Ok(Catalog::read(files(&crate::layout::agent_definitions(
        home,
    ))?))
}

fn files(folder: &Path) -> Result<Vec<UserFile>, String> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {error}", folder.display())),
    };
    // Each `.toml` file by the name it is shown with, and whether that name
    // is the file's own (UTF-8) or a lossy spelling of it.
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("{}: {error}", folder.display()))?;
        let file_name = entry.file_name();
        let (name, spelled) = match file_name.to_str() {
            Some(name) => (name.to_owned(), true),
            None => (file_name.to_string_lossy().into_owned(), false),
        };
        if name.ends_with(".toml") && entry.path().is_file() {
            names.push((name, spelled));
        }
    }
    names.sort();
    Ok(names
        .into_iter()
        .enumerate()
        .map(|(index, (name, spelled))| {
            if index >= MAX_USER_FILES {
                return UserFile::BeyondLimit { name };
            }
            if !spelled {
                return UserFile::Unreadable {
                    name,
                    error: "its name is not UTF-8".to_owned(),
                };
            }
            let path = folder.join(&name);
            match std::fs::metadata(&path) {
                Ok(metadata) if metadata.len() > MAX_FILE_BYTES => UserFile::TooLarge {
                    name,
                    bytes: metadata.len(),
                },
                Ok(_) => match std::fs::read_to_string(&path) {
                    Ok(text) => UserFile::Text { name, text },
                    Err(error) => UserFile::Unreadable {
                        name,
                        error: error.to_string(),
                    },
                },
                Err(error) => UserFile::Unreadable {
                    name,
                    error: error.to_string(),
                },
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_is_read_in_name_order_and_only_its_toml_files_count() {
        let home = tempfile::tempdir().unwrap();
        let folder = crate::layout::agent_definitions(home.path());
        assert_eq!(
            read(home.path()).unwrap().definitions.len(),
            hide_agent_adapter::definition::builtins().len()
        );
        std::fs::create_dir_all(folder.join("sub.toml")).unwrap();
        std::fs::write(folder.join("notes.txt"), "not a definition").unwrap();
        for id in ["b", "a"] {
            std::fs::write(
                folder.join(format!("{id}.toml")),
                format!("schema = 1\nid = \"{id}\"\nname = \"{id}\"\nexecutables = [\"{id}\"]\nhooks = \"none\"\n"),
            )
            .unwrap();
        }
        std::fs::write(
            folder.join("large.toml"),
            vec![b'#'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        let catalog = read(home.path()).unwrap();
        let mine: Vec<&str> = catalog
            .definitions
            .iter()
            .rev()
            .take(2)
            .map(|definition| definition.id.as_str())
            .collect();
        assert_eq!(mine, ["b", "a"]);
        assert_eq!(catalog.rejected.len(), 1);
        assert_eq!(catalog.rejected[0].file, "large.toml");
    }

    #[test]
    fn files_past_the_limit_are_named_without_being_read() {
        let folder = tempfile::tempdir().unwrap();
        for index in 0..=MAX_USER_FILES {
            std::fs::write(
                folder.path().join(format!("{index:03}.toml")),
                "unreadable on purpose",
            )
            .unwrap();
        }
        let files = files(folder.path()).unwrap();
        assert_eq!(files.len(), MAX_USER_FILES + 1);
        assert!(
            matches!(&files[MAX_USER_FILES], UserFile::BeyondLimit { name } if name == "064.toml")
        );
        assert!(matches!(&files[0], UserFile::Text { .. }));
    }

    // macOS and Windows file systems refuse a name that is not Unicode.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_file_whose_name_is_not_utf8_is_named_as_unreadable() {
        use std::os::unix::ffi::OsStrExt;
        let folder = tempfile::tempdir().unwrap();
        let name = std::ffi::OsStr::from_bytes(b"bad\xff.toml");
        std::fs::write(folder.path().join(name), "schema = 1").unwrap();
        let files = files(folder.path()).unwrap();
        assert!(matches!(
            files.as_slice(),
            [UserFile::Unreadable { name, error }]
                if name == "bad\u{fffd}.toml" && error == "its name is not UTF-8"
        ));
    }
}
