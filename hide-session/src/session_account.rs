//! Which account the node registered holds a session that a resume names
//! without one (PRD agent-profiles B19): a session line of the Sessions tab
//! or a session record carries the session's file or id, and the profile the
//! resume runs with is the one whose account holds it, as the node that keeps
//! the accounts answers ([`crate::Accounts`]), never a guess from the path's
//! spelling by the core.

use std::path::Path;

use hide_agent_adapter::AccountKey;
use serde::{Deserialize, Serialize};

use crate::{Accounts, Agent, ReadRefusal, RootRefusal};

/// Two accounts' OpenCode databases both hold the session id, so no one
/// account can be named for it.
pub const AMBIGUOUS: &str = "session_account_ambiguous";

/// A Claude Code or Codex resume names its session by id alone while the
/// node keeps an account of a profile for the agent besides the default, so
/// which account holds it is not known.
pub const UNPROVEN: &str = "session_account_unproven";

/// The session a resume names: its file (`path`) or, for an agent that keeps
/// no file per session, its id (`id`), as the agent's proof reference kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAccountRequest {
    pub agent: Agent,
    pub reference_kind: String,
    pub reference_value: String,
}

/// The account that holds the session `request` names.
///
/// A file is proven where every read of it is ([`crate::inside_session_root`],
/// D-36) and belongs to the innermost account that holds it. An OpenCode id
/// is looked up in each account's database, each database once however many
/// accounts name it; one that two databases hold is [`AMBIGUOUS`]. A
/// database that is not there, or holds no such session, is passed over; one
/// that cannot be read refuses the answer rather than naming another account.
///
/// A Claude Code or Codex id with no file is the start these agents always
/// had, which their own command finds and resumes: it keeps the default
/// account while that is the only one the node keeps for the agent, and is
/// [`UNPROVEN`] once a profile adds another; this proves nothing of the
/// session itself.
pub fn read(
    accounts: &Accounts,
    request: &SessionAccountRequest,
) -> Result<AccountKey, ReadRefusal> {
    let agent = request.agent;
    if request.reference_kind == "id" && !agent.requires_native_proof() {
        return default_alone(accounts, agent, &request.reference_value);
    }
    if request.reference_kind != agent.proof_reference_kind() {
        return Err("session_kind_unsupported".into());
    }
    match agent {
        Agent::OpenCode => opencode(accounts, &request.reference_value),
        // agy's sessions stay in the default account (it keeps no profile
        // account), as do retired Cursor's.
        Agent::Antigravity | Agent::Cursor => Ok(AccountKey::Default),
        agent => file(accounts, agent, Path::new(&request.reference_value)),
    }
}

fn default_alone(accounts: &Accounts, agent: Agent, id: &str) -> Result<AccountKey, ReadRefusal> {
    if !crate::valid_plain_id(id) {
        return Err("session_id_invalid".into());
    }
    if accounts.default_alone(agent)? {
        Ok(AccountKey::Default)
    } else {
        Err(UNPROVEN.into())
    }
}

/// Gives Claude Code the exact transcript when its old checkout is gone.
/// The old path is a lookup hint, and the provider account and session id
/// are proven on the machine that holds the file.
pub fn claude_resume_path(
    accounts: &Accounts,
    account: &AccountKey,
    session_id: &str,
    previous_checkout: &str,
) -> Result<String, ReadRefusal> {
    let accounts = accounts.reading(Agent::Claude, account)?;
    let native = hide_platform::path::from_wire(previous_checkout).ok();
    let (path, _) = crate::label_transcript::locate_confirmed(
        &accounts,
        Agent::Claude,
        "id",
        session_id,
        native.as_ref().and_then(|path| path.to_str()),
    )?;
    path.to_str()
        .filter(|path| path.len() <= 4096 && !path.chars().any(char::is_control))
        .map(str::to_owned)
        .ok_or_else(|| "label_session_path_invalid".into())
}

fn file(accounts: &Accounts, agent: Agent, path: &Path) -> Result<AccountKey, ReadRefusal> {
    if !path.is_absolute() {
        return Err(RootRefusal::Outside.reason().into());
    }
    crate::inside_session_root(accounts, &[agent], path)
        .map_err(|refusal| ReadRefusal::from(refusal.reason()))?;
    Ok(accounts
        .account_of(agent, path)
        .cloned()
        .unwrap_or_default())
}

fn opencode(accounts: &Accounts, id: &str) -> Result<AccountKey, ReadRefusal> {
    let mut seen = Vec::new();
    let mut holding: Option<AccountKey> = None;
    for (key, account) in accounts.each(Agent::OpenCode) {
        // Two accounts' folders can lead to one database (a folder spelled
        // two ways, a link to it): it is one place, read once.
        // An account whose database the node cannot name keeps none.
        let Ok(database) = crate::opencode::database_path(&account) else {
            continue;
        };
        let database = match hide_platform::fs::identity::canonical(&database) {
            Ok(database) => database,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err("opencode_db_unreadable".into()),
        };
        if seen.contains(&database) {
            continue;
        }
        seen.push(database);
        let found = crate::opencode::with_read(&account, |connection, schema| {
            crate::opencode::prove(connection, schema, id, crate::opencode::Owner::Recorded)
                .map(|_| ())
        });
        match found {
            Ok(Ok(())) => {
                if holding.is_some() {
                    return Err(AMBIGUOUS.into());
                }
                holding = Some(key);
            }
            Ok(Err(reason)) | Err(ReadRefusal::Refused(reason))
                if reason == RootRefusal::Missing.reason() => {}
            Ok(Err(reason)) => return Err(reason.into()),
            Err(refusal) => return Err(refusal),
        }
    }
    holding.ok_or_else(|| RootRefusal::Missing.reason().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_resume_path_proves_the_session_after_its_checkout_is_removed() {
        let home = tempfile::tempdir().unwrap();
        let accounts = Accounts::new(home.path());
        let gone = hide_platform::path::to_wire(&home.path().join("gone checkout")).unwrap();
        let logs = home.path().join(".claude/projects/gone-checkout");
        std::fs::create_dir_all(&logs).unwrap();
        let id = "b13b7d92-6623-4572-9c31-a5776b532dd7";
        let log = logs.join(format!("{id}.jsonl"));
        let line = |native_id| serde_json::json!({"type": "user", "sessionId": native_id, "message": {"role": "user", "content": "Saved conversation"}});
        std::fs::write(&log, format!("{}\n", line(id))).unwrap();
        let path = claude_resume_path(&accounts, &AccountKey::Default, id, &gone).unwrap();
        assert_eq!(
            std::fs::canonicalize(path).unwrap(),
            std::fs::canonicalize(&log).unwrap()
        );

        std::fs::write(
            &log,
            format!("{}\n", line("f3044bf8-a75a-4073-8b0a-3053564e53a2")),
        )
        .unwrap();
        assert!(claude_resume_path(&accounts, &AccountKey::Default, id, &gone).is_err());
    }
}
