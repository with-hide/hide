//! The operator's MCP keys (PRD skill-mcp-manager B10, B12, B13, B18, D-11,
//! D-39): the values of a server's variables and headers, kept in one
//! owner-only file of the state folder ([`crate::layout::mcp_keys`]) by the
//! store item they belong to, and handed to `hide_agent_hooks::mcp` only when
//! an agent's file is written.
//!
//! The file is the only place Hide keeps a key. Nothing here formats a
//! value: a read hands back [`Secret`]s, which print as `••••`, and a
//! failure says what went wrong with the file without quoting it, since a
//! parser's message can quote the value it choked on. A file that is not
//! the account's own private regular file is refused, never read: a key read
//! from a file someone else could write would be a key someone else chose.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_agent_hooks::mcp::{Keys, Secret};
use serde::{Deserialize, Serialize};

/// The largest key file Hide reads or writes.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// The longest value of one key.
pub const MAX_VALUE_BYTES: usize = 16 * 1024;
/// The longest name of a variable or header.
pub const MAX_NAME_BYTES: usize = 256;
/// The most keys one item holds.
pub const MAX_KEYS_PER_ITEM: usize = 64;
/// How long a change waits for another change of the file.
const LOCK_WAIT: Duration = Duration::from_secs(5);
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
    /// Reading, locking or writing the file failed.
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
    /// Each item's keys by variable or header name.
    items: BTreeMap<String, BTreeMap<String, String>>,
}

/// The keys of the item `item`; none when it has none.
pub fn keys(state_dir: &Path, item: &str) -> Result<Keys, KeyFileError> {
    let path = crate::layout::mcp_keys(state_dir);
    Ok(read(&path)?
        .items
        .remove(item)
        .map(secrets)
        .unwrap_or_default())
}

/// The items that hold keys.
pub fn items(state_dir: &Path) -> Result<Vec<String>, KeyFileError> {
    let path = crate::layout::mcp_keys(state_dir);
    Ok(read(&path)?.items.into_keys().collect())
}

/// Replaces the keys of `item` with `keys`; an empty `keys` takes the item
/// out. The file is written only when it changes.
pub fn set(state_dir: &Path, item: &str, keys: &Keys) -> Result<(), KeyFileError> {
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
    change(state_dir, |file| {
        let plain: BTreeMap<String, String> = keys
            .iter()
            .map(|(name, value)| (name.clone(), value.expose().to_owned()))
            .collect();
        if plain.is_empty() {
            file.items.remove(item).is_some()
        } else if file.items.get(item) == Some(&plain) {
            false
        } else {
            file.items.insert(item.to_owned(), plain);
            true
        }
    })
}

/// Wipes every key of `item`: a permanent delete (D-39).
pub fn wipe(state_dir: &Path, item: &str) -> Result<(), KeyFileError> {
    change(state_dir, |file| file.items.remove(item).is_some())
}

fn secrets(plain: BTreeMap<String, String>) -> Keys {
    plain
        .into_iter()
        .map(|(name, value)| (name, Secret::new(value)))
        .collect()
}

/// Runs `edit` on the file under its lock and writes it when `edit` says it
/// changed it. The file goes when it holds no item.
fn change(state_dir: &Path, edit: impl FnOnce(&mut File) -> bool) -> Result<(), KeyFileError> {
    let path = crate::layout::mcp_keys(state_dir);
    let io = |kind: ErrorKind| KeyFileError::Io {
        path: path.clone(),
        kind,
    };
    hide_platform::fs::private::create_dir_all(state_dir).map_err(|error| io(error.kind()))?;
    let _lock = lock(&path)?;
    let mut file = read(&path)?;
    if !edit(&mut file) {
        return Ok(());
    }
    if file.items.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io(error.kind())),
        };
    }
    file.schema = SCHEMA;
    let bytes =
        serde_json::to_vec(&file).map_err(|_| KeyFileError::Unreadable { path: path.clone() })?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(KeyFileError::TooLarge {
            what: "the key file",
        });
    }
    hide_platform::fs::atomic::write_file(&path, &bytes, hide_platform::fs::Access::Private)
        .map(drop)
        .map_err(|error| io(error.kind()))
}

fn lock(path: &Path) -> Result<hide_platform::fs::lock::Lock, KeyFileError> {
    let lock_path = path.with_extension("lock");
    let io = |kind: ErrorKind| KeyFileError::Io {
        path: lock_path.clone(),
        kind,
    };
    let file = hide_platform::fs::private::open_or_create_file(&lock_path)
        .map_err(|error| io(error.kind()))?;
    match hide_platform::fs::lock::lock_file(
        file,
        hide_platform::fs::lock::Mode::Exclusive,
        LOCK_WAIT,
        &|| false,
    )
    .map_err(|error| io(error.kind()))?
    {
        hide_platform::fs::lock::Waited::Locked(lock) => Ok(lock),
        hide_platform::fs::lock::Waited::TimedOut | hide_platform::fs::lock::Waited::Cancelled => {
            Err(io(ErrorKind::TimedOut))
        }
    }
}

fn read(path: &Path) -> Result<File, KeyFileError> {
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
