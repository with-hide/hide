//! One build of Hide as another Hide's machine runs it (PRD
//! device-connect-iroh D-77 (1), (2)): the programs a guest's node, its
//! `hide`, its hooks and its pane holders are, read here by the rule every
//! device's kit follows (`kit::select`), and placed in a guest's install
//! root on the machine that hosts it.
//!
//! The same code runs on both machines. The guest reads its build for the
//! host's platform and names it ([`GuestBuild`]); the host places a build
//! of that name from its own programs when they are that build, or from
//! the bytes the guest sends ([`place`]). A build's folder counts only once
//! every file in it matched and its marker is written.

use std::io;
use std::path::{Path, PathBuf};

use hide_peer_protocol::build::{self, BuildFile};

use super::{EstablishError, HELPER_NAME, HelperPackages, Payload, device_program, hex_digest};

/// The file in a build's folder that says every file of the build is there,
/// holding the build's name.
const MARKER: &str = ".build";

/// A build read whole for one platform.
pub struct GuestBuild {
    payload: Payload,
    files: Vec<BuildFile>,
    id: String,
}

impl HelperPackages {
    /// This build for a machine of `os`/`arch`: the node, the kit's parts,
    /// the pane holder and its runtime, all of them or a refusal that names
    /// what this build lacks for that platform.
    pub fn guest_build(&self, os: &str, arch: &str) -> Result<GuestBuild, EstablishError> {
        let payload = self.payload_for(os, arch)?;
        if !payload.missing.is_empty() {
            return Err(EstablishError::Unsupported(payload.missing.join("; ")));
        }
        let files: Vec<BuildFile> = payload
            .files
            .iter()
            .map(|file| BuildFile {
                path: file.relative.clone(),
                size: file.bytes.len() as u64,
                sha256: file.digest.clone(),
                executable: file.executable,
            })
            .collect();
        let id = build::id_of(&files);
        Ok(GuestBuild { payload, files, id })
    }
}

impl GuestBuild {
    /// The build's name: its folder in a guest's install root.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Every file of the build, in the order their bytes are sent.
    pub fn files(&self) -> &[BuildFile] {
        &self.files
    }

    /// The bytes of the file at `index` of [`GuestBuild::files`].
    pub fn bytes(&self, index: usize) -> Option<&[u8]> {
        self.payload
            .files
            .get(index)
            .map(|file| file.bytes.as_slice())
    }

    /// Places this build under `root`, for a host whose own programs are
    /// the build a guest named.
    pub fn place(&self, root: &Path) -> Result<PathBuf, String> {
        let mut next = 0;
        place(root, &self.id, &self.files, |_| {
            let bytes = self.bytes(next).unwrap_or_default().to_vec();
            next += 1;
            Ok(bytes)
        })
    }
}

/// Where the node program of the build `id` is under `root`, when that
/// build is placed there whole.
pub fn placed(root: &Path, id: &str, os: &str) -> Option<PathBuf> {
    let folder = root.join(id);
    let whole = build::is_id(id)
        && std::fs::read_to_string(folder.join(MARKER)).is_ok_and(|named| named == id);
    whole.then(|| folder.join(device_program(HELPER_NAME, os)))
}

/// Places the build `id` under `root` from `read`, which is asked for each
/// file's bytes in the order of `files`. Each file is checked against its
/// digest before it is written, a file already there is kept only when its
/// bytes are the build's, and the marker is written last: a folder a
/// failure left is finished by the next call or removed with the root.
/// Answers where the build's node program is.
pub fn place(
    root: &Path,
    id: &str,
    files: &[BuildFile],
    mut read: impl FnMut(&BuildFile) -> io::Result<Vec<u8>>,
) -> Result<PathBuf, String> {
    use hide_platform::fs::atomic::{self, DurableAccess};
    use hide_platform::fs::{Access, private};
    build::check(id, files).map_err(str::to_owned)?;
    let folder = root.join(id);
    private::create_dir_all(&folder).map_err(|error| {
        format!(
            "The build folder {} could not be made: {error}",
            folder.display()
        )
    })?;
    for file in files {
        let bytes =
            read(file).map_err(|error| format!("{} could not be read: {error}", file.path))?;
        if bytes.len() as u64 != file.size || hex_digest(&bytes) != file.sha256 {
            return Err(format!("{} is not the file the build names", file.path));
        }
        let path = file
            .path
            .split('/')
            .fold(folder.clone(), |path, name| path.join(name));
        if std::fs::read(&path).is_ok_and(|there| hex_digest(&there) == file.sha256) {
            continue;
        }
        if let Some(parent) = path.parent() {
            private::create_dir_all(parent)
                .map_err(|error| format!("{} could not be made: {error}", parent.display()))?;
        }
        let access = if file.executable {
            DurableAccess::PrivateExecutable
        } else {
            DurableAccess::Private
        };
        atomic::write_file_durable(&path, &bytes, access)
            .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
    }
    let marker = folder.join(MARKER);
    atomic::write_file(&marker, id.as_bytes(), Access::Private)
        .map_err(|error| format!("{} could not be written: {error}", marker.display()))?;
    Ok(folder.join(device_program(HELPER_NAME, std::env::consts::OS)))
}

/// Removes every build under `root` but the ones named in `keep`, which a
/// host passes the build that runs and the one before it. Nothing but a
/// build's folder is touched: the guest build keeps its own files beside
/// them. Answers the builds removed.
pub fn prune(root: &Path, keep: &[&str]) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !build::is_id(&name)
            || keep.contains(&name.as_str())
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
        {
            continue;
        }
        if std::fs::remove_dir_all(entry.path()).is_ok() {
            removed.push(name);
        }
    }
    removed.sort();
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, bytes: &[u8], executable: bool) -> BuildFile {
        BuildFile {
            path: path.to_owned(),
            size: bytes.len() as u64,
            sha256: hex_digest(bytes),
            executable,
        }
    }

    #[test]
    fn a_build_counts_as_placed_only_once_every_file_matched() {
        let root = tempfile::tempdir().unwrap();
        let files = [
            file(
                &device_program(HELPER_NAME, std::env::consts::OS),
                b"node",
                true,
            ),
            file("holder-conpty/conpty.dll", b"runtime", false),
        ];
        let id = build::id_of(&files);
        let bytes = |file: &BuildFile| -> io::Result<Vec<u8>> {
            Ok(match file.executable {
                true => b"node".to_vec(),
                false => b"runtime".to_vec(),
            })
        };

        // A file that is not the one named stops the build before its
        // marker: nothing is placed.
        let refused = place(root.path(), &id, &files, |_| Ok(b"other".to_vec()));
        assert!(
            refused
                .unwrap_err()
                .contains("not the file the build names")
        );
        assert_eq!(placed(root.path(), &id, std::env::consts::OS), None);

        let program = place(root.path(), &id, &files, bytes).unwrap();
        assert_eq!(std::fs::read(&program).unwrap(), b"node");
        assert_eq!(
            std::fs::read(
                root.path()
                    .join(&id)
                    .join("holder-conpty")
                    .join("conpty.dll")
            )
            .unwrap(),
            b"runtime"
        );
        assert_eq!(
            placed(root.path(), &id, std::env::consts::OS),
            Some(program.clone())
        );
        // Asked again, it is the same build in the same place.
        assert_eq!(place(root.path(), &id, &files, bytes).unwrap(), program);
    }

    #[test]
    fn a_prune_keeps_the_named_builds_and_everything_that_is_not_a_build() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "0000000000000001",
            "0000000000000002",
            "0000000000000003",
            ".hide",
        ] {
            std::fs::create_dir_all(root.path().join(name).join("x")).unwrap();
        }
        std::fs::write(root.path().join("current"), b"").unwrap();
        assert_eq!(
            prune(root.path(), &["0000000000000003", "0000000000000002"]),
            ["0000000000000001"]
        );
        let mut left: Vec<String> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [".hide", "0000000000000002", "0000000000000003", "current"]
        );
    }
}
