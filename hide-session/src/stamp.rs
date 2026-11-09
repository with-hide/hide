//! A session file's change token, compared by readers to know whether a
//! file moved since they last read it. An observation, never read
//! authority: a reader still confirms what it reads.

use std::fs;
use std::path::Path;

/// The current stamp of the file at `path`, `None` when it is gone or
/// unreadable.
pub fn stamp_at(path: &Path) -> Option<String> {
    if path.file_name().is_some_and(|name| name == "store.db") {
        crate::cursor::source_stamp(path)
    } else {
        metadata_stamp(&fs::metadata(path).ok()?)
    }
}

/// What a look at a file found ([`seen_at`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Seen {
    /// The file is there, with this stamp.
    At(String),
    /// Nothing is there.
    Absent,
    /// The look itself failed (a link that leads nowhere, a folder that
    /// cannot be read), so whether the file is there is not known.
    Unseen(std::io::ErrorKind),
}

/// The file at `path` as a look at it finds it, telling a file that is not
/// there from one that could not be looked at, which [`stamp_at`] does not.
pub fn seen_at(path: &Path) -> Seen {
    match fs::metadata(path) {
        Ok(metadata) => metadata_stamp(&metadata)
            .map_or(Seen::Unseen(std::io::ErrorKind::Unsupported), Seen::At),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Seen::Absent,
        Err(error) => Seen::Unseen(error.kind()),
    }
}

fn metadata_stamp(m: &fs::Metadata) -> Option<String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(format!(
            "{}:{}:{}:{}:{}",
            m.dev(),
            m.ino(),
            m.len(),
            m.mtime(),
            m.mtime_nsec()
        ))
    }
    #[cfg(not(unix))]
    {
        Some(format!("{}:{:?}", m.len(), m.modified().ok()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_tells_a_file_that_is_not_there_from_one_that_cannot_be_looked_at() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("file");
        assert_eq!(seen_at(&file), Seen::Absent);
        std::fs::write(&file, "x").unwrap();
        assert!(matches!(seen_at(&file), Seen::At(_)));
        #[cfg(unix)]
        {
            let looped = dir.path().join("looped");
            std::os::unix::fs::symlink(&looped, &looped).unwrap();
            assert!(matches!(seen_at(&looped), Seen::Unseen(_)));
        }
    }
}
