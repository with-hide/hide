//! Whether a profile's account is signed in, as this node finds it (PRD
//! agent-profiles D-16, B10): signed in, signed out, or not known because
//! the place that says so did not answer.
//!
//! Each agent keeps its sign-in its own way, so the check is the agent's:
//! Claude Code's keychain item on macOS (asked whether it is there, never
//! read) or its credentials file; the sign-in file Codex, pi and Grok write
//! in the account (`AccountPath::Login`). A file is read as its agent writes
//! it (`credentials`): signed in only while it holds the values the agent
//! runs with. An agent with no such place, or one whose sign-in is a
//! database Hide does not read (omp), has no check, and its profiles show
//! none.
//!
//! A list asks for every profile at once, and the screen lists again as it
//! opens and after each change, so each answer is kept for
//! [`LOGIN_TTL`]: a keychain question starts `/usr/bin/security`, and a
//! list never starts one per profile again within that time.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use hide_agent_adapter::Account;
use hide_node_link::profiles::ProfileLogin;

use crate::credentials;

/// How long one account's answer stands before it is looked at again.
pub(crate) const LOGIN_TTL: Duration = Duration::from_secs(30);
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

mod claude;

/// A definition's sign-in rule; only Claude's keychain needs code of its own.
fn check_of(kind: &str) -> Option<impl Fn(&Account) -> ProfileLogin> {
    let adapter = hide_agent_adapter::adapter(kind)?;
    let rule = adapter.account_rule()?.login.as_ref()?;
    let keychain = adapter.id == hide_agent_adapter::AgentId::ClaudeCode.adapter().id;
    Some(move |account: &Account| {
        if keychain && let Some(answer) = claude::keychain(account) {
            return answer;
        }
        match account.path(rule.at) {
            Some(file) => signed_in(account, file, rule),
            None => ProfileLogin::Unknown,
        }
    })
}

/// Signed in while `file` holds what its agent needs (`holds`); missing, or
/// without those values, signed out.
fn signed_in(
    account: &Account,
    file: &Path,
    rule: &hide_agent_adapter::definition::LoginRule,
) -> ProfileLogin {
    match credentials::read(file)
        .and_then(|bytes| bytes.map(|bytes| credentials::object(&bytes)).transpose())
    {
        Ok(Some(login)) if credentials::signed_in(&login, rule) => ProfileLogin::SignedIn,
        Ok(_) => ProfileLogin::SignedOut,
        Err(kind) => {
            log(account, kind);
            ProfileLogin::Unknown
        }
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
    use hide_agent_adapter::{AccountPath, AccountRef, NodeEnv, Profile, ProfileId};

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
        let check = check_of("codex").unwrap();
        assert_eq!(check(&codex), ProfileLogin::SignedOut);
        std::fs::write(&file, "{}").unwrap();
        assert_eq!(check(&codex), ProfileLogin::SignedOut);
        std::fs::write(
            &file,
            r#"{"tokens":{"access_token":"t","refresh_token":"r"}}"#,
        )
        .unwrap();
        assert_eq!(check(&codex), ProfileLogin::SignedIn);
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(check(&codex), ProfileLogin::Unknown);
    }

    #[test]
    fn a_sign_in_file_that_keeps_no_credentials_reads_signed_out() {
        // Each file as its agent writes it, with and without the values the
        // agent needs to run signed in.
        let cases = [
            (
                "codex",
                r#"{"OPENAI_API_KEY":null,"tokens":null,"last_refresh":null}"#,
                ProfileLogin::SignedOut,
            ),
            (
                "codex",
                r#"{"tokens":{"id_token":"","access_token":"","refresh_token":""}}"#,
                ProfileLogin::SignedOut,
            ),
            (
                "codex",
                r#"{"tokens":{"access_token":"a"}}"#,
                ProfileLogin::SignedIn,
            ),
            (
                "codex",
                r#"{"OPENAI_API_KEY":"sk-x","tokens":null}"#,
                ProfileLogin::SignedIn,
            ),
            (
                "codex",
                r#"{"tokens":{"id_token":"i","access_token":"a","refresh_token":"r","account_id":"c"},"last_refresh":"2026-11-09T00:00:00Z"}"#,
                ProfileLogin::SignedIn,
            ),
            (
                "pi",
                r#"{"anthropic":{"type":"oauth"}}"#,
                ProfileLogin::SignedOut,
            ),
            (
                "pi",
                r#"{"anthropic":{"type":"api_key","key":""}}"#,
                ProfileLogin::SignedOut,
            ),
            (
                "pi",
                r#"{"anthropic":{"type":"api_key","key":"k"}}"#,
                ProfileLogin::SignedIn,
            ),
            (
                "pi",
                r#"{"openrouter":{"type":"oauth","access":"a","refresh":"r","expires":1}}"#,
                ProfileLogin::SignedIn,
            ),
            (
                "grok",
                r#"{"https://auth.x.ai::u":{"auth_mode":"oidc","email":"e"}}"#,
                ProfileLogin::SignedOut,
            ),
            (
                "grok",
                r#"{"https://auth.x.ai::u":{"key":"k","auth_mode":"oidc"}}"#,
                ProfileLogin::SignedIn,
            ),
        ];
        for (kind, written, expected) in cases {
            let folder = tempfile::tempdir().unwrap();
            let account = account(kind, folder.path());
            std::fs::write(account.path(AccountPath::Login).unwrap(), written).unwrap();
            assert_eq!(
                check_of(kind).unwrap()(&account),
                expected,
                "{kind} {written}"
            );
        }
        // Claude Code's credentials file; a keychain item is asked for first
        // on macOS, and an account folder of a test has none.
        for (written, expected) in [
            ("{}", ProfileLogin::SignedOut),
            (r#"{"claudeAiOauth":null}"#, ProfileLogin::SignedOut),
            (
                r#"{"claudeAiOauth":{"accessToken":"","refreshToken":""}}"#,
                ProfileLogin::SignedOut,
            ),
            (
                r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"r","expiresAt":1}}"#,
                ProfileLogin::SignedIn,
            ),
        ] {
            let folder = tempfile::tempdir().unwrap();
            let claude = account("claude-code", folder.path());
            std::fs::write(claude.path(AccountPath::Usage).unwrap(), written).unwrap();
            assert_eq!(
                check_of("claude-code").unwrap()(&claude),
                expected,
                "{written}"
            );
        }
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
        std::fs::write(&file, r#"{"anthropic":{"type":"api_key","key":"k"}}"#).unwrap();
        // Within the time an answer stands, the list says what it said.
        assert_eq!(login("pi", &pi), Some(ProfileLogin::SignedOut));
        answers().clear();
        assert_eq!(login("pi", &pi), Some(ProfileLogin::SignedIn));
    }
}
