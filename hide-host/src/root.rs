use std::path::{Path, PathBuf};

use cap_std::fs::Dir;
use hide_platform::path::{PathError, RelPath};

use crate::error::{ErrorCode, HostError, HostResult};

pub use hide_node_link::RootIdentity;

/// An opened checkout root: the handle every operation is confined to.
#[derive(Debug)]
pub struct Root {
    path: PathBuf,
    /// The canonical spelling of `path` when it was opened; an absolute link
    /// target under it is a place inside the checkout.
    real_path: PathBuf,
    dir: Dir,
    identity: RootIdentity,
}

impl Root {
    /// Opens `path` and records which directory it names.
    pub fn open(path: &Path) -> HostResult<Self> {
        if !path.is_absolute() {
            return Err(HostError::new(
                ErrorCode::InvalidPath,
                "A checkout root must be an absolute path",
            ));
        }
        let dir = Dir::open_ambient_dir(path, cap_std::ambient_authority())
            .map_err(|error| HostError::io(&error, "The checkout folder could not be opened"))?;
        let identity = identity_of(&dir)
            .map_err(|error| HostError::io(&error, "The checkout folder could not be inspected"))?;
        let real_path = hide_platform::fs::identity::canonical(path)
            .map_err(|error| HostError::io(&error, "The checkout folder could not be resolved"))?;
        Ok(Self {
            path: path.to_path_buf(),
            real_path,
            dir,
            identity,
        })
    }

    /// Opens `path` and refuses it unless it is still the directory `expected`
    /// names.
    pub fn open_pinned(path: &Path, expected: RootIdentity) -> HostResult<Self> {
        let root = Self::open(path)?;
        if root.identity != expected {
            return Err(HostError::new(
                ErrorCode::RootReplaced,
                "The checkout folder was replaced since it was opened; nothing was read or changed",
            ));
        }
        Ok(root)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn dir(&self) -> &Dir {
        &self.dir
    }

    pub fn real_path(&self) -> &Path {
        &self.real_path
    }

    pub fn identity(&self) -> RootIdentity {
        self.identity
    }
}

pub fn identity_of(dir: &Dir) -> std::io::Result<RootIdentity> {
    let id = hide_platform::fs::identity::file_id_of(dir)?;
    // The wire carries a 64-bit inode. NTFS and every Unix filesystem fit; a
    // 128-bit file id (ReFS) is refused rather than cut short.
    let inode = u64::try_from(id.index()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "this volume's directory identity does not fit the wire",
        )
    })?;
    Ok(RootIdentity {
        device: id.volume(),
        inode,
    })
}

/// An absolute path the core names on this machine: its wire spelling
/// (`hide_platform::path::from_wire`), or a native absolute one, which a
/// core on this same machine may still hand its own node. `None` for any
/// other text. On macOS and Linux both are the same `/...` path.
pub fn native_absolute(path: &str) -> Option<PathBuf> {
    hide_platform::path::from_wire(path)
        .ok()
        .or_else(|| Some(PathBuf::from(path)).filter(|path| path.is_absolute()))
}

/// An absolute path of this machine as an answer carries it: the wire
/// spelling, so a core on another system reads it as absolute and relates
/// it to the others by names. On macOS and Linux it is the path's own text.
pub fn wire(path: &Path) -> String {
    hide_platform::path::to_wire_lossy(path)
}

/// [`wire`] for a path a program printed, such as Git's `C:/repo`.
pub fn wire_text(path: &str) -> String {
    wire(Path::new(path))
}

/// A path inside a root as the wire spells it (`hide_platform::path::RelPath`:
/// `/`-separated names, no leading `/`, no `.` or `..`), as this machine
/// names it below the root's handle. The empty string is the root itself.
/// The handle is what actually confines the work; this refuses the shapes
/// that could never name a child, and a name this system cannot hold as
/// written, before anything is opened.
pub(crate) fn relative_path(raw: &str) -> HostResult<PathBuf> {
    RelPath::parse(raw)
        .and_then(|relative| relative.to_native())
        .map_err(|error| match error {
            // Inside the checkout, but no file here can have that name.
            PathError::Unrepresentable(_) => HostError::new(
                ErrorCode::InvalidPath,
                format!("The path is invalid: {error}"),
            ),
            _ => HostError::new(
                ErrorCode::InvalidPath,
                "The path is not inside the checkout",
            ),
        })
}

/// The parent directory handle of `relative` and the final name, opened
/// through `dir`. The root itself has no parent inside the checkout.
pub fn open_parent(dir: &Dir, relative: &Path) -> HostResult<(Dir, std::ffi::OsString)> {
    let name = relative.file_name().ok_or_else(|| {
        HostError::new(ErrorCode::InvalidPath, "The checkout root is not an item")
    })?;
    let parent = relative
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let opened = dir
        .open_dir(parent)
        .map_err(|error| HostError::io(&error, "The folder could not be opened"))?;
    Ok((opened, name.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_refuse_every_shape_that_leaves_the_root() {
        for raw in ["/etc/passwd", "../x", "a/../b", "./a", "a\0b"] {
            assert_eq!(
                relative_path(raw).unwrap_err().code,
                ErrorCode::InvalidPath,
                "{raw}"
            );
        }
        assert_eq!(relative_path("").unwrap(), PathBuf::new());
        assert_eq!(relative_path("a/b.txt").unwrap(), PathBuf::from("a/b.txt"));
    }

    #[cfg(unix)]
    #[test]
    fn a_root_replaced_at_its_path_is_refused() {
        let outer = tempfile::tempdir().unwrap();
        let path = outer.path().join("checkout");
        std::fs::create_dir(&path).unwrap();
        let identity = Root::open(&path).unwrap().identity();
        std::fs::rename(&path, outer.path().join("moved")).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert_eq!(
            Root::open_pinned(&path, identity).unwrap_err().code,
            ErrorCode::RootReplaced
        );
    }
}
