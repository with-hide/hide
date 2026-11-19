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
//!
//! A build is up to [`build::MAX_BYTES`], and its link may be slow: the
//! host takes each file a piece at a time into a file beside its name, so
//! no file is held in memory and no file has a time it must arrive in.

use std::io;
use std::path::{Path, PathBuf};

use hide_peer_protocol::build::{self, BuildFile};
use sha2::{Digest, Sha256};

use super::{EstablishError, HELPER_NAME, HelperPackages, Payload, device_program};

/// The most bytes of a file asked of its source, and held, at once.
const PIECE: usize = 64 * 1024;

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The file in a build's folder that says every file of the build is there,
/// holding the build's name.
pub const MARKER: &str = ".build";

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
        place(root, &self.id, &self.files, &mut self.source())
    }

    /// Every file's bytes one after the other, in the order of
    /// [`GuestBuild::files`]: what a guest sends and [`place`] takes.
    pub fn source(&self) -> impl io::Read + '_ {
        Source {
            files: self.payload.files.iter(),
            on: &[],
        }
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

/// [`GuestBuild::source`]: the build's files as one run of bytes.
struct Source<'a> {
    files: std::slice::Iter<'a, super::Package>,
    on: &'a [u8],
}

impl io::Read for Source<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        while self.on.is_empty() {
            match self.files.next() {
                Some(file) => self.on = &file.bytes,
                None => return Ok(0),
            }
        }
        let taken = self.on.len().min(buffer.len());
        buffer[..taken].copy_from_slice(&self.on[..taken]);
        self.on = &self.on[taken..];
        Ok(taken)
    }
}

/// Places the build `id` under `root` from `source`: every file's bytes one
/// after the other, in the order of `files`. Each file is read a piece at a
/// time into `<name>.part` beside its name, digested as it goes, and takes
/// its name only when its size and digest are the ones the build names; a
/// file that fails leaves nothing behind. A file already there with the
/// build's bytes is kept, and its bytes in `source` are read past and
/// checked like any other's, so what follows stays in step. The marker is
/// written last: a folder a failure left is finished by the next call or
/// removed with the root. Answers where the build's node program is.
///
/// `source` sets how long a piece may take; nothing here bounds a file.
pub fn place(
    root: &Path,
    id: &str,
    files: &[BuildFile],
    source: &mut dyn io::Read,
) -> Result<PathBuf, String> {
    use hide_platform::fs::atomic::{self, DurableAccess, DurableWriteError};
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
        let path = file
            .path
            .split('/')
            .fold(folder.clone(), |path, name| path.join(name));
        if there(&path, file) {
            take(source, file, &mut io::sink()).map_err(|error| error.to_string())?;
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
        atomic::write_file_durable_from(&path, access, |part| take(source, file, part)).map_err(
            |error| match error {
                // The file's own refusal, which names it.
                DurableWriteError::BeforeReplace { source, .. } => source.to_string(),
                error => format!("{} could not be written: {error}", path.display()),
            },
        )?;
    }
    let marker = folder.join(MARKER);
    atomic::write_file(&marker, id.as_bytes(), Access::Private)
        .map_err(|error| format!("{} could not be written: {error}", marker.display()))?;
    Ok(folder.join(device_program(HELPER_NAME, std::env::consts::OS)))
}

/// Whether the file at `path` already holds the bytes the build names for
/// `file`, read through a piece at a time.
fn there(path: &Path, file: &BuildFile) -> bool {
    let Ok(mut found) = std::fs::File::open(path) else {
        return false;
    };
    if !found
        .metadata()
        .is_ok_and(|found| found.is_file() && found.len() == file.size)
    {
        return false;
    }
    let mut hasher = Sha256::new();
    io::copy(&mut found, &mut hasher).is_ok() && hex(hasher.finalize()) == file.sha256
}

/// Reads the bytes of `file` off `source` into `into`, a piece at a time,
/// and refuses them unless they are the ones the build names. The error
/// names the file and what failed.
fn take(source: &mut dyn io::Read, file: &BuildFile, into: &mut dyn io::Write) -> io::Result<()> {
    let failed = |what: &str, error: io::Error| {
        io::Error::new(
            error.kind(),
            format!("{} could not be {what}: {error}", file.path),
        )
    };
    let mut hasher = Sha256::new();
    let mut piece = vec![0u8; PIECE];
    let mut left = file.size;
    while left > 0 {
        let wanted = left.min(PIECE as u64) as usize;
        let read = match source.read(&mut piece[..wanted]) {
            Ok(0) => return Err(failed("read", io::ErrorKind::UnexpectedEof.into())),
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(failed("read", error)),
        };
        hasher.update(&piece[..read]);
        into.write_all(&piece[..read])
            .map_err(|error| failed("written", error))?;
        left -= read as u64;
    }
    if hex(hasher.finalize()) != file.sha256 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is not the file the build names", file.path),
        ));
    }
    Ok(())
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
            sha256: hex(Sha256::digest(bytes)),
            executable,
        }
    }

    /// What is under a build's folder, by name.
    fn names(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A source that counts what was asked of it and hands out at most
    /// `most` bytes at a time, as a slow link does.
    struct Trickle<'a> {
        bytes: &'a [u8],
        most: usize,
        asked: Vec<usize>,
    }

    impl io::Read for Trickle<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.asked.push(buffer.len());
            let taken = self.bytes.len().min(buffer.len()).min(self.most);
            buffer[..taken].copy_from_slice(&self.bytes[..taken]);
            self.bytes = &self.bytes[taken..];
            Ok(taken)
        }
    }

    #[test]
    fn a_build_counts_as_placed_only_once_every_file_matched() {
        let root = tempfile::tempdir().unwrap();
        let program = device_program(HELPER_NAME, std::env::consts::OS);
        let files = [
            file(&program, b"node", true),
            file("holder-conpty/conpty.dll", b"runtime", false),
        ];
        let id = build::id_of(&files);
        let folder = root.path().join(&id);

        // A file that is not the one named stops the build before its
        // marker: nothing is placed, and nothing of the file is left.
        let refused = place(root.path(), &id, &files, &mut &b"nodeRUNTIME"[..]);
        assert_eq!(
            refused.unwrap_err(),
            "holder-conpty/conpty.dll is not the file the build names"
        );
        assert_eq!(placed(root.path(), &id, std::env::consts::OS), None);
        assert_eq!(names(&folder), [program.as_str(), "holder-conpty"]);
        assert!(names(&folder.join("holder-conpty")).is_empty());

        // Bytes that end early are no file either.
        let short = place(root.path(), &id, &files, &mut &b"noderun"[..]);
        assert!(
            short
                .unwrap_err()
                .starts_with("holder-conpty/conpty.dll could not be read"),
        );
        assert!(names(&folder.join("holder-conpty")).is_empty());

        // The whole build again: the first file is there already, and its
        // bytes are read past so the second's are the right ones.
        let placed_at = place(root.path(), &id, &files, &mut &b"noderuntime"[..]).unwrap();
        assert_eq!(std::fs::read(&placed_at).unwrap(), b"node");
        assert_eq!(
            std::fs::read(folder.join("holder-conpty").join("conpty.dll")).unwrap(),
            b"runtime"
        );
        assert_eq!(
            placed(root.path(), &id, std::env::consts::OS),
            Some(placed_at.clone())
        );
        assert_eq!(names(&folder.join("holder-conpty")), ["conpty.dll"]);
        // Asked again, it is the same build in the same place, and bytes
        // that are not a kept file's are still refused.
        assert_eq!(
            place(root.path(), &id, &files, &mut &b"noderuntime"[..]).unwrap(),
            placed_at
        );
        assert!(place(root.path(), &id, &files, &mut &b"NODEruntime"[..]).is_err());
        assert_eq!(std::fs::read(&placed_at).unwrap(), b"node");
    }

    /// A file larger than a piece is taken a piece at a time, however few
    /// bytes each read brings: no read asks for more than a piece, and what
    /// a writer that was killed left beside the file's name is replaced.
    #[test]
    fn a_file_is_taken_a_piece_at_a_time_and_a_part_left_behind_is_replaced() {
        let root = tempfile::tempdir().unwrap();
        let program = device_program(HELPER_NAME, std::env::consts::OS);
        let bytes: Vec<u8> = (0..PIECE * 3 + 17).map(|at| (at % 251) as u8).collect();
        let files = [file(&program, &bytes, true)];
        let id = build::id_of(&files);
        let folder = root.path().join(&id);
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(format!("{program}.part")), b"half").unwrap();

        let mut source = Trickle {
            bytes: &bytes,
            most: 1000,
            asked: Vec::new(),
        };
        let placed_at = place(root.path(), &id, &files, &mut source).unwrap();

        assert_eq!(std::fs::read(&placed_at).unwrap(), bytes);
        assert!(source.asked.iter().all(|asked| *asked <= PIECE));
        assert!(source.asked.len() > bytes.len() / 1000);
        assert_eq!(names(&folder), [MARKER, program.as_str()]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&placed_at).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
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
