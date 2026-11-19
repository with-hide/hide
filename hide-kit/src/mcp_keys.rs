//! The operator's MCP keys (PRD skill-mcp-manager B10, B12, B13, B18, D-11,
//! D-39): the values of a server's variables and headers, kept in one
//! owner-only file, [`FILE`], in the store's folder of the item they belong
//! to, and handed to `hide_agent_hooks::mcp` only when an agent's file is
//! written.
//!
//! A server set aside from an agent's file when the operator took its name
//! over (B21) keeps the entry as the file spelled it in the same file, since
//! that holds its keys too; the next write of the folder's keys drops it.
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

use hide_agent_hooks::mcp::{Keys, Secret, SetAside};
use serde::{Deserialize, Serialize};

/// The key file's name in an item's folder.
pub const FILE: &str = "keys.json";
/// The store's secret's file in the store's folder ([`KeysSecret`]).
pub const SECRET_FILE: &str = "keys.secret";
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

/// The store's own secret, which keys the fingerprints of key values the
/// store's record keeps (which keys Hide wrote where, so a key the operator
/// typed into a file by hand reads as theirs): 32 random bytes, in the
/// owner-only [`SECRET_FILE`] beside the store's record. A fingerprint read
/// without it tells nothing of a value, a short one included. It prints
/// nothing of itself.
#[derive(Clone, Eq, PartialEq)]
pub struct KeysSecret([u8; 32]);

impl KeysSecret {
    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for KeysSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("KeysSecret(••••)")
    }
}

/// The secret of the store in `store_dir`, made the first time when `make`
/// asks; none when it is not there and `make` does not. A file that is not
/// the account's own private file, or not a secret this Hide made, is
/// refused rather than used or replaced.
pub fn secret(store_dir: &Path, make: bool) -> Result<Option<KeysSecret>, KeyFileError> {
    let path = store_dir.join(SECRET_FILE);
    let io = |kind: ErrorKind| KeyFileError::Io {
        path: path.clone(),
        kind,
    };
    match hide_platform::fs::private::open_own_file(&path, false) {
        Ok(file) => {
            if !hide_platform::fs::private::is_private(&path).unwrap_or(false) {
                return Err(KeyFileError::NotPrivate { path });
            }
            let mut text = String::new();
            file.take(128)
                .read_to_string(&mut text)
                .map_err(|error| io(error.kind()))?;
            let text = text.trim();
            let unreadable = || KeyFileError::Unreadable { path: path.clone() };
            if text.len() != 64 {
                return Err(unreadable());
            }
            let mut bytes = [0u8; 32];
            for (at, byte) in bytes.iter_mut().enumerate() {
                *byte =
                    u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).map_err(|_| unreadable())?;
            }
            Ok(Some(KeysSecret(bytes)))
        }
        Err(error) if error.kind() == ErrorKind::NotFound && make => {
            let mut bytes = [0u8; 32];
            getrandom::getrandom(&mut bytes).map_err(|_| io(ErrorKind::Other))?;
            let text: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
            hide_platform::fs::private::create_dir_all(store_dir)
                .map_err(|error| io(error.kind()))?;
            hide_platform::fs::atomic::write_file(
                &path,
                format!("{text}\n").as_bytes(),
                hide_platform::fs::Access::Private,
            )
            .map_err(|error| io(error.kind()))?;
            Ok(Some(KeysSecret(bytes)))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) if error.kind() == ErrorKind::PermissionDenied => {
            Err(KeyFileError::NotPrivate { path })
        }
        Err(error) => Err(io(error.kind())),
    }
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    schema: u32,
    /// The values by variable or header name.
    keys: BTreeMap<String, String>,
    /// The agent's entry this copy was set aside from, for putting it back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    set_aside: Option<SetAside>,
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

/// The agent's entry kept in the item folder `folder` when it was set aside.
pub fn set_aside_of(folder: &Path) -> Result<Option<SetAside>, KeyFileError> {
    Ok(read_file(&folder.join(FILE))?.set_aside)
}

/// Keeps `keys` and the agent's entry `entry` they were read from in the
/// item folder `folder`.
pub fn keep_set_aside(folder: &Path, keys: &Keys, entry: &SetAside) -> Result<(), KeyFileError> {
    save(folder, keys, Some(entry.clone()))
}

/// Replaces the keys kept in the item folder `folder` with `keys`; an empty
/// `keys` takes the file out. The file is written only when it changes.
pub fn write(folder: &Path, keys: &Keys) -> Result<(), KeyFileError> {
    save(folder, keys, None)
}

fn save(folder: &Path, keys: &Keys, set_aside: Option<SetAside>) -> Result<(), KeyFileError> {
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
    if current.keys == plain && current.set_aside.is_none() && set_aside.is_none() {
        return Ok(());
    }
    if plain.is_empty() && set_aside.is_none() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io(error.kind())),
        };
    }
    let bytes = serde_json::to_vec(&File {
        schema: SCHEMA,
        keys: plain,
        set_aside,
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
