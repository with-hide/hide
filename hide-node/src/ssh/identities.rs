//! Which keys a device's sign-in offers, in the order OpenSSH offers them.
//!
//! `ssh <alias>` in the operator's terminal is the reference: every
//! `IdentityFile` the config gives the alias, or OpenSSH's default files in
//! `~/.ssh` when it gives none, and the agent's keys unless `IdentitiesOnly`
//! is set (ssh_config(5), IdentityFile and IdentitiesOnly). The order is
//! OpenSSH's own (`pubkey_prepare` in sshconnect2.c): agent keys that are one
//! of the files first, then the agent's other keys, then the files the agent
//! does not hold. A file Hide cannot sign with itself, one behind a
//! passphrase or on a security key, is offered only through the agent, and a
//! sign-in that fails names it with the command that loads it there.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use russh::keys::ssh_key::{Algorithm, PrivateKey, PublicKey};
use russh::keys::{Error as KeyError, decode_secret_key, load_public_key};
use serde::{Deserialize, Serialize};

/// OpenSSH's default identity files, in its order, for a config that names
/// none (ssh_config(5), IdentityFile, OpenSSH 9.8 and later; DSA and XMSS
/// keys are no longer built into its client).
pub const DEFAULT_IDENTITY_FILES: [&str; 5] = [
    "id_rsa",
    "id_ecdsa",
    "id_ecdsa_sk",
    "id_ed25519",
    "id_ed25519_sk",
];

/// One key file the alias's sign-in may read.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct IdentityFile {
    /// Where the key is read on this machine.
    pub path: PathBuf,
    /// How a failure names it: as the config wrote it, or `~/.ssh/<name>`
    /// for a default, so the home folder's own path never reaches a message.
    pub shown: String,
    /// OpenSSH's default rather than an `IdentityFile` line: a default that
    /// does not exist is passed over without a word, as OpenSSH does.
    pub default: bool,
}

/// The files OpenSSH tries for an alias whose config names none.
pub fn default_identity_files(ssh_dir: &Path) -> Vec<IdentityFile> {
    DEFAULT_IDENTITY_FILES
        .iter()
        .map(|name| IdentityFile {
            path: ssh_dir.join(name),
            shown: format!("~/.ssh/{name}"),
            default: true,
        })
        .collect()
}

/// Expands one `IdentityFile` value against the account's home. OpenSSH also
/// expands `%h`, `%r` and the other connection tokens and `${VAR}`; Hide
/// refuses those rather than read a file whose name it guessed.
pub fn expand_identity_file(value: &str, home: &Path) -> Result<IdentityFile, String> {
    let shown = value.trim_matches('"').to_owned();
    let (base, rest) = if shown == "~" {
        (Some(home), "")
    } else if let Some(rest) = shown.strip_prefix("~/").or(shown.strip_prefix("%d/")) {
        (Some(home), rest)
    } else {
        (None, shown.as_str())
    };
    if rest.contains('$') {
        return Err(format!(
            "IdentityFile {shown} names an environment variable Hide does not expand"
        ));
    }
    let mut expanded = String::with_capacity(rest.len());
    let mut characters = rest.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            expanded.push(character);
            continue;
        }
        match characters.next() {
            Some('%') => expanded.push('%'),
            Some('d') => expanded.push_str(&home.to_string_lossy()),
            _ => {
                return Err(format!(
                    "IdentityFile {shown} carries a percent token Hide does not expand"
                ));
            }
        }
    }
    let path = match base {
        Some(home) => home.join(expanded),
        None => PathBuf::from(expanded),
    };
    if !path.is_absolute() {
        return Err(format!(
            "IdentityFile {shown} must be an absolute path or start with ~/"
        ));
    }
    Ok(IdentityFile {
        path,
        shown,
        default: false,
    })
}

/// Why a key file cannot sign here and needs the agent instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentOnly {
    /// Hide never asks for a passphrase.
    Passphrase,
    /// A FIDO security key signs only through the agent or ssh itself.
    SecurityKey,
}

/// What reading one key file found.
#[derive(Debug)]
pub enum KeyFile {
    /// A private key Hide signs with itself.
    Usable(PrivateKey),
    /// A private key only the agent can use; its public half, when one could
    /// be read, matches it to an agent key.
    AgentOnly(AgentOnly, Option<PublicKey>),
    /// Only the public half is here (the file is a public key, or the private
    /// file is absent beside a `.pub`): the agent has to hold the private key.
    PublicOnly(PublicKey),
    Missing,
    /// The file exists but is not a key Hide can read.
    Unreadable,
}

impl KeyFile {
    pub fn read(path: &Path) -> Self {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return match load_public_key(public_half_path(path)) {
                    Ok(public) => Self::PublicOnly(public),
                    Err(_) => Self::Missing,
                };
            }
            Err(_) => return Self::Unreadable,
        };
        match decode_secret_key(&text, None) {
            Ok(key) if is_security_key(key.algorithm()) => {
                Self::AgentOnly(AgentOnly::SecurityKey, Some(key.public_key().clone()))
            }
            Ok(key) => Self::Usable(key),
            Err(KeyError::KeyIsEncrypted) => {
                // An OpenSSH-format key keeps its public half unencrypted; an
                // older PEM key has only its `.pub` beside it.
                let public = PrivateKey::from_openssh(&text)
                    .map(|key| key.public_key().clone())
                    .ok()
                    .or_else(|| load_public_key(public_half_path(path)).ok());
                Self::AgentOnly(AgentOnly::Passphrase, public)
            }
            Err(_) => match PublicKey::from_openssh(text.trim()) {
                Ok(public) => Self::PublicOnly(public),
                Err(_) => Self::Unreadable,
            },
        }
    }

    fn public(&self) -> Option<&PublicKey> {
        match self {
            Self::Usable(key) => Some(key.public_key()),
            Self::AgentOnly(_, public) => public.as_ref(),
            Self::PublicOnly(public) => Some(public),
            Self::Missing | Self::Unreadable => None,
        }
    }
}

fn public_half_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".pub");
    PathBuf::from(name)
}

fn is_security_key(algorithm: Algorithm) -> bool {
    matches!(
        algorithm,
        Algorithm::SkEd25519 | Algorithm::SkEcdsaSha2NistP256
    )
}

/// One key sign-in offers the server.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Offer {
    /// The agent's key at this index, signed by the agent.
    Agent(usize),
    /// The file at this index, signed here.
    File(usize),
}

/// The keys sign-in offers, in order, and which files the agent signs for.
#[derive(Debug, Eq, PartialEq)]
pub struct Plan {
    pub offers: Vec<Offer>,
    /// Per file: the agent holds its key, so the agent's offer is the file's.
    pub covered: Vec<bool>,
}

impl Plan {
    /// OpenSSH's offer order (`pubkey_prepare`): agent keys that are one of
    /// the files, in the agent's order; then, unless `identities_only`, the
    /// agent's other keys; then the files the agent does not hold that Hide
    /// can sign with.
    pub fn new(agent: &[PublicKey], files: &[KeyFile], identities_only: bool) -> Self {
        let mut covered = vec![false; files.len()];
        let mut offers = Vec::new();
        let mut others = Vec::new();
        for (index, key) in agent.iter().enumerate() {
            let file = (0..files.len()).find(|file| {
                !covered[*file] && files[*file].public().is_some_and(|public| same_key(public, key))
            });
            match file {
                Some(file) => {
                    covered[file] = true;
                    offers.push(Offer::Agent(index));
                }
                None if !identities_only => others.push(Offer::Agent(index)),
                None => {}
            }
        }
        offers.extend(others);
        offers.extend(
            files
                .iter()
                .enumerate()
                .filter(|(index, read)| !covered[*index] && matches!(read, KeyFile::Usable(_)))
                .map(|(index, _)| Offer::File(index)),
        );
        Self { offers, covered }
    }
}

/// The same key whatever comment each copy carries.
fn same_key(left: &PublicKey, right: &PublicKey) -> bool {
    left.key_data() == right.key_data()
}

/// What the agent gave sign-in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AgentState {
    /// `IdentityAgent none`.
    Disabled,
    /// No `IdentityAgent` and `SSH_AUTH_SOCK` is not set.
    NotSet,
    /// The socket did not answer.
    Unreachable,
    /// It answered with this many keys.
    Held(usize),
}

/// The sentence a refused sign-in reports: what was offered and, for each
/// key that never was, what the operator does about it. "Holds no
/// identities" and "rejected every key" stay apart, because one is "load a
/// key" and the other is "authorize this key". `source` names where the
/// agent was looked for, never its path.
pub fn refusal(
    agent: AgentState,
    source: &str,
    plan: &Plan,
    files: &[IdentityFile],
    reads: &[KeyFile],
) -> String {
    let from_agent = plan
        .offers
        .iter()
        .filter(|offer| matches!(offer, Offer::Agent(_)))
        .count();
    let from_files = plan.offers.len() - from_agent;
    let mut sentence = if plan.offers.is_empty() {
        let why = match agent {
            AgentState::Disabled => "the ssh config sets IdentityAgent none".to_owned(),
            AgentState::NotSet => {
                "no IdentityAgent is set and SSH_AUTH_SOCK is not set".to_owned()
            }
            AgentState::Unreachable => {
                format!("the SSH agent reached through {source} did not answer")
            }
            AgentState::Held(0) => {
                format!("the SSH agent reached through {source} holds no identities")
            }
            // With keys in the agent and nothing offered, IdentitiesOnly
            // turned every one of them away.
            AgentState::Held(held) => format!(
                "IdentitiesOnly is set and none of the {held} identities the SSH agent reached through {source} holds is one of the key files"
            ),
        };
        format!("no key was offered: {why}")
    } else {
        let mut parts = Vec::new();
        if from_agent > 0 {
            parts.push(format!(
                "{from_agent} from the SSH agent reached through {source}"
            ));
        }
        if from_files > 0 {
            parts.push(format!("{from_files} from key files"));
        }
        format!(
            "the server rejected every key offered ({})",
            parts.join(", ")
        )
    };

    let mut notes = Vec::new();
    for ((file, read), covered) in files.iter().zip(reads).zip(&plan.covered) {
        if *covered {
            continue;
        }
        let shown = &file.shown;
        match read {
            KeyFile::AgentOnly(AgentOnly::Passphrase, _) => notes.push(format!(
                "{shown} is protected by a passphrase Hide cannot enter: load it into the agent with `{}` or use a key without a passphrase",
                add_to_agent(shown)
            )),
            KeyFile::AgentOnly(AgentOnly::SecurityKey, _) => notes.push(format!(
                "{shown} is a security key only the agent can use: load it with `{}`",
                add_to_agent(shown)
            )),
            KeyFile::PublicOnly(_) => notes.push(format!(
                "{shown} is a public key whose private key the agent does not hold"
            )),
            KeyFile::Missing if !file.default => notes.push(format!("{shown} does not exist")),
            KeyFile::Unreadable => notes.push(format!("{shown} is not a key Hide can read")),
            KeyFile::Usable(_) | KeyFile::Missing => {}
        }
    }
    let no_default_exists = !files.is_empty()
        && files
            .iter()
            .zip(reads)
            .all(|(file, read)| file.default && matches!(read, KeyFile::Missing));
    if no_default_exists {
        notes.push("none of OpenSSH's default key files exists in ~/.ssh".to_owned());
    }
    for note in notes {
        let _ = write!(sentence, "; {note}");
    }
    sentence
}

/// The command that puts a key into the agent the operator's own `ssh`
/// uses; on macOS it keeps the passphrase in the login keychain, so the key
/// is back after a restart.
fn add_to_agent(shown: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("ssh-add --apple-use-keychain {shown}")
    } else {
        format!("ssh-add {shown}")
    }
}
