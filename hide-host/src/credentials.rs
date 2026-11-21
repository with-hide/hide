//! What each agent's login file holds: the one reading of Codex's
//! `auth.json`, Claude Code's credentials, and pi's and Grok's `auth.json`,
//! shared by the sign-in check (`login`, PRD agent-profiles D-16) and the
//! usage reads (`usage`). A file says an account is signed in only when it
//! holds the values its agent runs with, not merely when it is there: an
//! agent that signs out may leave the file behind with them emptied.
//!
//! Deliberately private: nothing read here is printed, logged or sent
//! anywhere but to the endpoint the agent's own login is for.

use std::io::Read;
use std::path::Path;

use serde_json::Value;

/// The largest login file read: the agents' own are a few KiB.
pub(crate) const LIMIT: u64 = 64 * 1024;

/// The bytes of the login file `path`; `None` when there is no such file.
/// The error is the kind of failure, for the log.
pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, &'static str> {
    let opened = match std::fs::File::open(path) {
        Ok(opened) => opened,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("login_unreadable"),
    };
    let mut bytes = Vec::new();
    opened
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "login_unreadable")?;
    if bytes.len() as u64 > LIMIT {
        return Err("login_too_large");
    }
    Ok(Some(bytes))
}

/// The JSON object in a login file's bytes: one with nothing but blanks in
/// it holds nothing, as an empty object does.
pub(crate) fn object(bytes: &[u8]) -> Result<Value, &'static str> {
    if bytes.len() as u64 > LIMIT {
        return Err("login_too_large");
    }
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    match serde_json::from_slice::<Value>(bytes) {
        Ok(value @ Value::Object(_)) => Ok(value),
        _ => Err("login_damaged"),
    }
}

/// The string at `pointer` in `value`, when it has something in it.
fn text<'v>(value: &'v Value, pointer: &str) -> Option<&'v str> {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
}

/// Codex's `auth.json`: an API key in `OPENAI_API_KEY`, or the ChatGPT
/// sign-in's `tokens`, which Codex needs both of to run and renew.
pub(crate) mod codex {
    use super::{Value, text};

    /// The ChatGPT sign-in's access token and account, which the usage
    /// endpoint takes.
    pub(crate) fn usage_login(auth: &Value) -> Option<(&str, &str)> {
        Some((
            text(auth, "/tokens/access_token")?,
            text(auth, "/tokens/account_id")?,
        ))
    }
}

/// Claude Code's login (its keychain item's password, or the
/// `.credentials.json` in the account's folder): `claudeAiOauth`, whose
/// refresh token keeps an account signed in after its access token expires.
pub(crate) mod claude {
    use super::{Value, text};

    /// The access token and when it expires (Unix milliseconds), if the
    /// login says.
    pub(crate) fn access_token(login: &Value) -> Option<(&str, Option<u64>)> {
        Some((
            text(login, "/claudeAiOauth/accessToken")?,
            login
                .pointer("/claudeAiOauth/expiresAt")
                .and_then(Value::as_u64),
        ))
    }
}

/// Whether a definition's credential paths hold any nonblank value.
/// Files are already bounded by `LIMIT`; pointers and their depth are
/// bounded while reading the definition.
pub(crate) fn signed_in(value: &Value, rule: &hide_agent_adapter::definition::LoginRule) -> bool {
    rule.any.iter().any(|pointer| {
        if let Some(pointer) = pointer.strip_prefix("/*") {
            value
                .as_object()
                .is_some_and(|entries| entries.values().any(|entry| text(entry, pointer).is_some()))
        } else {
            text(value, pointer).is_some()
        }
    })
}
