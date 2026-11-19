//! The operator's MCP keys (PRD skill-mcp-manager B10, B12, B13, B18, D-11,
//! D-39): the values of a server's variables and headers, kept in one
//! owner-only file, [`FILE`], in the store's folder of the item they belong
//! to, and handed to `hide_agent_hooks::mcp` only when an agent's file is
//! written.
//!
//! The file lives and goes with its item's folder: a delete moves the folder
//! to the undo list and a restore moves it back, so the keys come back with
//! the item, and a permanent delete removes the folder with them (D-39). Two
//! items of one name, one in the store and one in the undo list, therefore
//! never share a key.
//!
//! Nothing here formats a value: a read hands back [`Secret`]s, which print
//! as `••••`, and a failure says what went wrong with the file without
//! quoting it, since a parser's message can quote the value it choked on. A
//! file that is not the account's own private regular file is refused, never
//! read: a key read from a file someone else could write would be a key
//! someone else chose. Every read and write runs in a store pass, under the
//! kit's account lock, so the file takes no lock of its own.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

use hide_agent_hooks::mcp::{Keys, Secret};
use serde::{Deserialize, Serialize};

/// The key file's name in an item's folder.
pub const FILE: &str = "keys.json";
/// The largest key file Hide reads or writes.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// The longest value of one key.
pub const MAX_VALUE_BYTES: usize = 16 * 1024;
/// The longest name of a variable or header.
pub const MAX_NAME_BYTES: usize = 256;
/// The most keys one item holds.
pub const MAX_KEYS_PER_ITEM: usize = 64;
const SCHEMA: u32 = 1;

/// Why the key file could not be used. It never carries a key or a line of
/// the file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KeyFileError {
    /// The file is not the account's own private regular file.
    NotPrivate { path: PathBuf },
    /// The file is not a key file this Hide reads, or is larger than
    /// [`MAX_FILE_BYTES`].
    Unreadable { path: PathBuf },
    /// A key is longer, or an item holds more keys, than the limits allow.
    TooLarge { what: &'static str },
    /// Reading or writing the file failed.
    Io { path: PathBuf, kind: ErrorKind },
}

impl fmt::Display for KeyFileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPrivate { path } => write!(
                formatter,
                "{} is not this account's own private file, so Hide does not read keys from it",
                path.display()
            ),
            Self::Unreadable { path } => {
                write!(formatter, "{} is not a key file Hide reads", path.display())
            }
            Self::TooLarge { what } => write!(formatter, "{what} is over Hide's limit for keys"),
            Self::Io { path, kind } => {
                write!(formatter, "{} could not be used: {kind}", path.display())
            }
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema: u32,
    /// The values by variable or header name.
    keys: BTreeMap<String, String>,
}

/// The keys kept in the item folder `folder`; none when it holds no key
/// file.
pub fn read(folder: &Path) -> Result<Keys, KeyFileError> {
    Ok(read_file(&folder.join(FILE))?
        .keys
        .into_iter()
        .map(|(name, value)| (name, Secret::new(value)))
        .collect())
}

/// Replaces the keys kept in the item folder `folder` with `keys`; an empty
/// `keys` takes the file out. The file is written only when it changes.
pub fn write(folder: &Path, keys: &Keys) -> Result<(), KeyFileError> {
    if keys.len() > MAX_KEYS_PER_ITEM {
        return Err(KeyFileError::TooLarge {
            what: "the number of keys",
        });
    }
    if keys
        .keys()
        .any(|name| name.is_empty() || name.len() > MAX_NAME_BYTES)
    {
        return Err(KeyFileError::TooLarge {
            what: "a key's name",
        });
    }
    if keys
        .values()
        .any(|value| value.expose().len() > MAX_VALUE_BYTES)
    {
        return Err(KeyFileError::TooLarge { what: "a key" });
    }
    let path = folder.join(FILE);
    let io = |kind: ErrorKind| KeyFileError::Io {
        path: path.clone(),
        kind,
    };
    let plain: BTreeMap<String, String> = keys
        .iter()
        .map(|(name, value)| (name.clone(), value.expose().to_owned()))
        .collect();
    // A file Hide cannot read as its own is refused before it is replaced.
    let current = read_file(&path)?;
    if current.keys == plain {
        return Ok(());
    }
    if plain.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io(error.kind())),
        };
    }
    let bytes = serde_json::to_vec(&File {
        schema: SCHEMA,
        keys: plain,
    })
    .map_err(|_| KeyFileError::Unreadable { path: path.clone() })?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(KeyFileError::TooLarge {
            what: "the key file",
        });
    }
    hide_platform::fs::private::create_dir_all(folder).map_err(|error| io(error.kind()))?;
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
        .map(drop)
        .map_err(|error| io(error.kind()))
}

fn read_file(path: &Path) -> Result<File, KeyFileError> {
    let file = match hide_platform::fs::private::open_own_file(path, false) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(File::default()),
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            return Err(KeyFileError::NotPrivate {
                path: path.to_path_buf(),
            });
        }
        Err(error) => {
            return Err(KeyFileError::Io {
                path: path.to_path_buf(),
                kind: error.kind(),
            });
        }
    };
    if !hide_platform::fs::private::is_private(path).unwrap_or(false) {
        return Err(KeyFileError::NotPrivate {
            path: path.to_path_buf(),
        });
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| KeyFileError::Io {
            path: path.to_path_buf(),
            kind: error.kind(),
        })?;
    let unreadable = || KeyFileError::Unreadable {
        path: path.to_path_buf(),
    };
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(unreadable());
    }
    // serde's message can quote the value it could not read, so it is
    // dropped here and never reaches a log.
    let file: File = serde_json::from_slice(&bytes).map_err(|_| unreadable())?;
    if file.schema != SCHEMA {
        return Err(unreadable());
    }
    Ok(file)
}

#[cfg(test)]
#[path = "mcp_keys/tests.rs"]
mod tests;
