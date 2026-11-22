//! How a guest's build reaches a host that does not have it (PRD D-77
//! (1)): a `file` stream whose first message names the build and every
//! file of it, followed by each file's bytes in that order, and answered
//! once.
//!
//! A build is named by the first 16 hex characters of a digest over its
//! files' paths, modes and digests, so a host that has those files under
//! that name has that build, whoever placed it. A host checks every file
//! against the message before the build's folder counts as placed.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::frame::{self, FrameError};

/// How many characters of the digest name a build's folder.
pub const ID_CHARS: usize = 16;
/// The most files a build has.
pub const MAX_FILES: usize = 64;
/// The most bytes a build's files have together: the size limit of a
/// release's device kit (`hide-node/src/ssh/kit/manifest.rs`, `MAX_KIT`).
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;
/// The most bytes of a file's path.
pub const MAX_PATH: usize = 255;

/// A message of a `file` stream. The first one says what the stream
/// carries; a host drops a stream whose first message it does not know.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum FileMessage {
    /// Guest to host, first: the build whose files' bytes follow, each
    /// file whole, in this order.
    Build { id: String, files: Vec<BuildFile> },
    /// Host to guest, after the last byte: the build is placed and every
    /// file of it matched.
    BuildPlaced { id: String },
    /// Host to guest: the stream's content was not taken, and why. Nothing
    /// of it counts as placed.
    Refused { reason: String },
}

impl FileMessage {
    pub const TYPES: [&'static str; 3] = ["build", "build_placed", "refused"];

    /// `None` for a type this build does not know.
    pub fn parse(body: &[u8]) -> Result<Option<Self>, FrameError> {
        match frame::message_type(body)? {
            Some(name) if Self::TYPES.contains(&name.as_str()) => frame::decode(body).map(Some),
            _ => Ok(None),
        }
    }
}

/// One file of a build, as it goes into the build's folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildFile {
    /// Its path under the build's folder, `/` between names.
    pub path: String,
    pub size: u64,
    /// The SHA-256 of its bytes, lowercase hex.
    pub sha256: String,
    pub executable: bool,
}

/// The name of the build these files make: any change to any file's path,
/// mode or bytes is another name.
pub fn id_of(files: &[BuildFile]) -> String {
    let mut entries: Vec<String> = files
        .iter()
        .map(|file| {
            format!(
                "{}\0{}\0{}\n",
                file.path,
                if file.executable { "x" } else { "-" },
                file.sha256
            )
        })
        .collect();
    entries.sort();
    let digest = Sha256::digest(entries.concat().as_bytes());
    hex::encode(digest)[..ID_CHARS].to_owned()
}

/// Whether `id` is spelled as a build's folder is: 16 lowercase hex
/// characters.
pub fn is_id(id: &str) -> bool {
    id.len() == ID_CHARS
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn path_stays_below(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= MAX_PATH
        && !path.contains(['\\', ':', '\0'])
        && path
            .split('/')
            .all(|name| !name.is_empty() && name != "." && name != "..")
}

/// Why a host does not take the build a message names; `Ok` when it may
/// read the bytes.
pub fn check(id: &str, files: &[BuildFile]) -> Result<(), &'static str> {
    if !is_id(id) {
        return Err("the build's name is not 16 hex characters");
    }
    if files.is_empty() || files.len() > MAX_FILES {
        return Err("the build has no file, or more than a build may");
    }
    let mut total = 0u64;
    let mut seen = std::collections::BTreeSet::new();
    for file in files {
        if !path_stays_below(&file.path) {
            return Err("a file's path does not stay below the build's folder");
        }
        if !seen.insert(file.path.as_str()) {
            return Err("a file is named twice");
        }
        if file.sha256.len() != 64 || !file.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err("a file's digest is not a SHA-256");
        }
        total = total.saturating_add(file.size);
    }
    if total > MAX_BYTES {
        return Err("the build is larger than a build may be");
    }
    if id_of(files) != id {
        return Err("the build's name is not the digest of its files");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, executable: bool) -> BuildFile {
        BuildFile {
            path: path.to_owned(),
            size: 3,
            sha256: hex::encode(Sha256::digest(path.as_bytes())),
            executable,
        }
    }

    #[test]
    fn a_build_is_named_by_its_files_whatever_their_order() {
        let files = [file("hided", true), file("holder-conpty/conpty.dll", false)];
        let reversed = [files[1].clone(), files[0].clone()];
        let id = id_of(&files);
        assert!(is_id(&id), "{id}");
        assert_eq!(id, id_of(&reversed));
        assert_ne!(id, id_of(&[file("hided", false), files[1].clone()]));
        assert_eq!(check(&id, &files), Ok(()));
    }

    #[test]
    fn a_host_refuses_a_build_it_could_not_place_safely() {
        let good = [file("hided", true)];
        let id = id_of(&good);
        assert!(check("0123", &good).is_err());
        assert!(check(&id, &[]).is_err());
        assert!(
            check(&id_of(&[file("hide", true)]), &good).is_err(),
            "a name of other files"
        );
        for path in [
            "../hided",
            "/hided",
            "a//b",
            "a/./b",
            "c:\\hided",
            "a\\b",
            "",
        ] {
            let files = [file(path, true)];
            assert!(check(&id_of(&files), &files).is_err(), "{path:?}");
        }
        let twice = [file("hided", true), file("hided", true)];
        assert!(check(&id_of(&twice), &twice).is_err());
        let mut large = file("hided", true);
        large.size = MAX_BYTES + 1;
        assert!(check(&id_of(std::slice::from_ref(&large)), &[large]).is_err());
    }

    #[test]
    fn a_file_message_reads_back_and_an_unknown_type_is_skipped() {
        let message = FileMessage::Build {
            id: "0123456789abcdef".to_owned(),
            files: vec![file("hided", true)],
        };
        let body = serde_json::to_vec(&message).unwrap();
        assert_eq!(FileMessage::parse(&body).unwrap(), Some(message));
        assert_eq!(FileMessage::parse(br#"{"t":"later"}"#).unwrap(), None);
    }
}
