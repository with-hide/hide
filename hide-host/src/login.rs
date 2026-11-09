//! Whether a profile's account is signed in, as this node finds it (PRD
//! agent-profiles D-16, B10): signed in, signed out, or not known because
//! the place that says so did not answer.
//!
//! Each agent keeps its sign-in its own way, so the check is the agent's:
//! Claude Code's keychain item on macOS (asked whether it is there, never
//! read) or its credentials file; the sign-in file Codex, pi and Grok write
//! in the account (`AccountPath::Login`). An agent with no such place, or
//! one whose sign-in is a database Hide does not read (omp), has no check,
//! and its profiles show none.
//!
//! A list asks for every profile at once, and the screen lists again as it
//! opens and after each change, so each answer is kept for
//! [`LOGIN_TTL`]: a keychain question starts `/usr/bin/security`, and a
//! list never starts one per profile again within that time.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hide_agent_adapter::{Account, AccountPath, AgentId};
use hide_node_link::profiles::ProfileLogin;
use serde_json::Value;

/// How long one account's answer stands before it is looked at again.
pub(crate) const LOGIN_TTL: Duration = Duration::from_secs(30);
/// The largest sign-in file read: the agents' own are a few KiB.
const LOGIN_FILE_LIMIT: u64 = 64 * 1024;
/// The most answers kept, one per profile a machine can keep.
const KEPT: usize = hide_node_link::profiles::MAX_PROFILES;

/// What each account answered, by its agent and key, and when.
static ANSWERS: Mutex<BTreeMap<(String, hide_agent_adapter::AccountKey), (Instant, ProfileLogin)>> =
    Mutex::new(BTreeMap::new());

/// The sign-in of `account`, an account of the `kind` agent's own (not its
/// default one); `None` when the agent's sign-in cannot be looked at.
pub(crate) fn login(kind: &str, account: &Account) -> Option<ProfileLogin> {
    let check = check_of(kind)?;
    let key = (kind.to_owned(), account.key.clone());
    let now = Instant::now();
    if let Some((at, answer)) = answers().get(&key)
        && now.duration_since(*at) < LOGIN_TTL
    {
        return Some(*answer);
    }
    let answer = check(account);
    let mut answers = answers();
    answers.retain(|_, (at, _)| now.duration_since(*at) < LOGIN_TTL);
    if answers.len() < KEPT {
        answers.insert(key, (now, answer));
    }
    Some(answer)
}

fn answers() -> std::sync::MutexGuard<
    'static,
    BTreeMap<(String, hide_agent_adapter::AccountKey), (Instant, ProfileLogin)>,
> {
    ANSWERS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

type Check = fn(&Account) -> ProfileLogin;

/// The check of the `kind` agent's sign-in, if it has one.
fn check_of(kind: &str) -> Option<Check> {
    let id = hide_agent_adapter::adapter(kind)?.id;
    let is = |agent: AgentId| agent.adapter().id == id;
    if is(AgentId::ClaudeCode) {
        Some(claude)
    } else if is(AgentId::Codex) || is(AgentId::Pi) || is(AgentId::Grok) {
        Some(signed_in_file)
    } else {
        None
    }
}

/// Claude Code's sign-in: its keychain item on macOS, else (and on macOS
/// with no item) the credentials file in the account's folder.
fn claude(account: &Account) -> ProfileLogin {
    #[cfg(target_os = "macos")]
    match crate::usage::claude_keychain_item(
        &crate::usage::claude_keychain_service(&account.key),
        false,
    ) {
        Ok(Some(_)) => return ProfileLogin::SignedIn,
        Ok(None) => {}
        Err(_) => {
            log(account, "keychain_unreadable");
            return ProfileLogin::Unknown;
        }
    }
    match account.path(AccountPath::Usage) {
        Some(file) => match file.try_exists() {
            Ok(true) => ProfileLogin::SignedIn,
            Ok(false) => ProfileLogin::SignedOut,
            Err(_) => {
                log(account, "credentials_unreadable");
                ProfileLogin::Unknown
            }
        },
        None => ProfileLogin::Unknown,
    }
}

/// A sign-in file the agent writes in the account (`AccountPath::Login`):
/// signed in while it holds a JSON object with something in it; missing or
/// empty, signed out.
fn signed_in_file(account: &Account) -> ProfileLogin {
    let Some(file) = account.path(AccountPath::Login) else {
        return ProfileLogin::Unknown;
    };
    match read_object(file) {
        Ok(Some(object)) if !object.is_empty() => ProfileLogin::SignedIn,
        Ok(_) => ProfileLogin::SignedOut,
        Err(kind) => {
            log(account, kind);
            ProfileLogin::Unknown
        }
    }
}

/// The JSON object in `file`; `None` when there is no file.
fn read_object(file: &Path) -> Result<Option<serde_json::Map<String, Value>>, &'static str> {
    let opened = match std::fs::File::open(file) {
        Ok(opened) => opened,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("login_unreadable"),
    };
    let mut bytes = Vec::new();
    opened
        .take(LOGIN_FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "login_unreadable")?;
    if bytes.len() as u64 > LOGIN_FILE_LIMIT {
        return Err("login_too_large");
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Some(serde_json::Map::new()));
    }
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(Value::Object(object)) => Ok(Some(object)),
        _ => Err("login_damaged"),
    }
}

/// Why an account's sign-in was not known, for the log: the kind of
/// failure alone, never a path or a file's contents.
fn log(account: &Account, kind: &str) {
    eprintln!(
        "{}",
        serde_json::json!({
            "component": "profiles",
            "kind": "login.unknown",
            "reason": kind,
            "default": account.key.is_default(),
        })
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use hide_agent_adapter::{AccountRef, NodeEnv, Profile, ProfileId};

    fn account(kind: &str, folder: &Path) -> Account {
        let profile = Profile {
            id: ProfileId(format!("p-{kind}")),
            kind: kind.to_owned(),
            name: "work".to_owned(),
            account: Some(AccountRef::Folder(folder.display().to_string())),
            program: None,
            env: Vec::new(),
            version: 1,
        };
        hide_agent_adapter::adapter(kind)
            .unwrap()
            .resolve(&profile, &NodeEnv::new(Some(folder.to_path_buf())))
            .unwrap()
    }

    #[test]
    fn a_sign_in_file_says_whether_the_account_is_signed_in() {
        let folder = tempfile::tempdir().unwrap();
        let codex = account("codex", folder.path());
        let file = codex.path(AccountPath::Login).unwrap().to_path_buf();
        assert_eq!(signed_in_file(&codex), ProfileLogin::SignedOut);
        std::fs::write(&file, "{}").unwrap();
        assert_eq!(signed_in_file(&codex), ProfileLogin::SignedOut);
        std::fs::write(&file, r#"{"tokens":{"access_token":"t"}}"#).unwrap();
        assert_eq!(signed_in_file(&codex), ProfileLogin::SignedIn);
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(signed_in_file(&codex), ProfileLogin::Unknown);
    }

    #[test]
    fn only_agents_whose_sign_in_hide_can_look_at_have_a_check() {
        for kind in ["claude-code", "codex", "pi", "grok"] {
            assert!(check_of(kind).is_some(), "{kind}");
        }
        for kind in ["omp", "opencode", "antigravity"] {
            assert!(check_of(kind).is_none(), "{kind}");
        }
    }

    #[test]
    fn an_answer_is_kept_for_a_while_and_then_looked_at_again() {
        let folder = tempfile::tempdir().unwrap();
        let pi = account("pi", folder.path());
        let file = pi.path(AccountPath::Login).unwrap().to_path_buf();
        assert_eq!(login("pi", &pi), Some(ProfileLogin::SignedOut));
        std::fs::write(&file, r#"{"anthropic":{"type":"oauth"}}"#).unwrap();
        // Within the time an answer stands, the list says what it said.
        assert_eq!(login("pi", &pi), Some(ProfileLogin::SignedOut));
        answers().clear();
        assert_eq!(login("pi", &pi), Some(ProfileLogin::SignedIn));
    }
}
