//! Files to and from another machine over a [`super::session::Session`]'s
//! start script, for the core move's copies (`hided`'s `core_move`).
//!
//! A file goes whole or not at all on either side: the bytes land beside
//! the final name, are checked against the SHA-256 the sending side stated,
//! and only then take the name. A file is read here twice when it is sent,
//! once for its digest and once for its bytes, so a copy never holds a whole
//! file in memory.

use std::io::Read;
use std::path::Path;

use sha2::{Digest, Sha256};

use super::host::bootstrap::{self, Pipe, StepError};

/// One file of a copy: where it is on this machine and on the other.
#[derive(Clone, Debug)]
pub struct FileCopy {
    pub local: std::path::PathBuf,
    /// An absolute path on the other machine, `/`-separated.
    pub remote: String,
}

fn local(path: &Path, error: impl std::fmt::Display) -> StepError {
    StepError::Local(format!("{}: {error}", path.display()))
}

fn hex(digest: impl AsRef<[u8]>) -> String {
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The SHA-256 and size of a local file, read through once.
fn digest_of(path: &Path) -> Result<(String, u64), StepError> {
    let mut file = std::fs::File::open(path).map_err(|error| local(path, error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut size = 0_u64;
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                hasher.update(&buffer[..read]);
                size += read as u64;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(local(path, error)),
        }
    }
    Ok((hex(hasher.finalize()), size))
}

/// Sends `file` to its absolute path on the other machine, where each
/// missing folder on the way is made private. `sent` hears each chunk's
/// size as it goes.
pub(super) fn upload(
    pipe: &mut dyn Pipe,
    file: &FileCopy,
    sent: &(dyn Fn(u64) + Sync),
) -> Result<(), StepError> {
    let (digest, size) = digest_of(&file.local)?;
    let mut bytes = std::fs::File::open(&file.local).map_err(|error| local(&file.local, error))?;
    bootstrap::put_file(pipe, &file.remote, &digest, size, &mut bytes, sent)
}

/// Copies `file.remote` into `<local>.part` beside its final name and
/// renames it into place once it holds the whole file the other machine
/// stated, by size and digest. Only a file below `into` is written
/// ([`folders_below`]). `received` hears each chunk's size as it is
/// written.
pub(super) fn download(
    pipe: &mut dyn Pipe,
    into: &Path,
    file: &FileCopy,
    received: &(dyn Fn(u64) + Sync),
) -> Result<(), StepError> {
    folders_below(into, &file.local)?;
    let part = file.local.with_extension(match file.local.extension() {
        Some(extension) => format!("{}.part", extension.to_string_lossy()),
        None => "part".to_owned(),
    });
    match std::fs::remove_file(&part) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(local(&part, error));
        }
        _ => {}
    }
    let mut written = hide_platform::fs::private::create_new_file(&part)
        .map_err(|error| local(&part, error))?;
    let copied = (|| {
        let mut hashing = Hashing {
            file: &mut written,
            hasher: Sha256::new(),
            size: 0,
        };
        let (size, stated) = bootstrap::get_file(pipe, &file.remote, &mut hashing, received)?;
        let (hasher, got) = (hashing.hasher, hashing.size);
        if got != size || hex(hasher.finalize()) != stated {
            return Err(StepError::Refused(format!(
                "{} changed while it was read",
                file.remote
            )));
        }
        written.sync_all().map_err(|error| local(&part, error))
    })();
    drop(written);
    if let Err(error) = copied {
        let _ = std::fs::remove_file(&part);
        return Err(error);
    }
    std::fs::rename(&part, &file.local).map_err(|error| local(&file.local, error))
}

/// A file being written, with the digest and size of what went into it.
struct Hashing<'a> {
    file: &'a mut std::fs::File,
    hasher: Sha256,
    size: u64,
}

impl std::io::Write for Hashing<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let written = self.file.write(bytes)?;
        self.hasher.update(&bytes[..written]);
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.file.flush()
    }
}

/// Makes each missing folder between `into` and the file `local`, private.
/// A `local` that is not below `into` by plain names, or a folder on the
/// way that is a link or a file, is refused: what the other machine names
/// is written below the folder it was given and nowhere else.
fn folders_below(into: &Path, local_file: &Path) -> Result<(), StepError> {
    use std::path::Component;
    let outside = || {
        StepError::Local(format!(
            "{} is not below {}",
            local_file.display(),
            into.display()
        ))
    };
    let mut names = Vec::new();
    for component in local_file
        .strip_prefix(into)
        .map_err(|_| outside())?
        .components()
    {
        match component {
            Component::Normal(name) => names.push(name),
            _ => return Err(outside()),
        }
    }
    if names.pop().is_none() {
        return Err(outside());
    }
    hide_platform::fs::private::create_dir_all(into).map_err(|error| local(into, error))?;
    let mut folder = into.to_path_buf();
    for name in names {
        folder.push(name);
        match std::fs::symlink_metadata(&folder) {
            Ok(found) if found.is_dir() => {}
            Ok(_) => {
                return Err(StepError::Local(format!(
                    "{} is not a folder",
                    folder.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hide_platform::fs::private::create_dir(&folder)
                    .map_err(|error| local(&folder, error))?;
            }
            Err(error) => return Err(local(&folder, error)),
        }
    }
    Ok(())
}

/// The size of a local file, for the copy's progress total.
pub fn local_size(path: &Path) -> Result<u64, String> {
    std::fs::metadata(path)
        .map(|metadata| metadata.len())
        .map_err(|error| format!("{}: {error}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_writes_only_below_the_folder_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("move-staging/i1");
        folders_below(&into, &into.join("factory-files/a/b.prd")).unwrap();
        assert!(into.join("factory-files/a").is_dir());
        for local in [
            dir.path().join("outside"),
            into.join("../outside"),
            into.clone(),
        ] {
            assert!(folders_below(&into, &local).is_err(), "{}", local.display());
        }
        // A link on the way would lead the file outside.
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        hide_platform::fs::link::create_link(&elsewhere, &into.join("linked")).unwrap();
        assert!(folders_below(&into, &into.join("linked/x")).is_err());
        assert!(!elsewhere.join("x").exists());
    }
}
