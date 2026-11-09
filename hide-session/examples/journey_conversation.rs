//! The agent journey check's conversation reader (`scripts/agent_journey`):
//! one session's whole conversation as Hide reads it, located and read by the
//! same calls the node answers `Call::SessionConversation` with, so the check
//! never parses a native transcript itself. Not a product binary.
//!
//! journey_conversation <home> <agent kind> <id|path> <reference> <cwd> [<account>]
//! prints {"path": ..., "events": [{"role", "kind", "text", "at"}...]}, `at`
//! the record's time in Unix milliseconds as the session wrote it, or exits 1
//! with the reader's own refusal on stderr. `<account>`, `folder:<path>` or
//! `name:<name>`, is the account of the profile the agent was started with,
//! which the session is read in alone, as the node reads a profile agent's.

use std::path::Path;

/// The pages one read takes before the answer is refused as too long; a
/// journey session holds a few short turns.
const MAX_PAGES: usize = 64;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (home, kind, reference_kind, reference, cwd, account) = match args.as_slice() {
        [home, kind, reference_kind, reference, cwd] => {
            (home, kind, reference_kind, reference, cwd, None)
        }
        [home, kind, reference_kind, reference, cwd, account] => (
            home,
            kind,
            reference_kind,
            reference,
            cwd,
            Some(account.as_str()),
        ),
        _ => {
            eprintln!(
                "usage: journey_conversation <home> <agent kind> <id|path> <reference> <cwd> [<account>]"
            );
            std::process::exit(2);
        }
    };
    let home = Path::new(home);
    let answer = accounts(home, kind, account)
        .and_then(|accounts| read(&accounts, home, kind, reference_kind, reference, cwd));
    match answer {
        Ok(answer) => println!("{answer}"),
        Err(reason) => {
            eprintln!("{reason}");
            std::process::exit(1);
        }
    }
}

/// The accounts the session is read in: the agent's default under `home`,
/// or the profile account `account` names alone.
fn accounts(
    home: &Path,
    kind: &str,
    account: Option<&str>,
) -> Result<hide_session::Accounts, String> {
    let all = hide_session::Accounts::new(home);
    let Some(account) = account else {
        return Ok(all);
    };
    let spelled = match account.split_once(':') {
        Some(("folder", folder)) => hide_agent_adapter::AccountRef::Folder(folder.to_owned()),
        Some(("name", name)) => hide_agent_adapter::AccountRef::Name(name.to_owned()),
        _ => return Err("account_unknown: folder:<path> or name:<name>".to_owned()),
    };
    let adapter = hide_agent_adapter::adapter(kind).ok_or("agent_kind_unknown")?;
    let agent = hide_session::Agent::from_kind(kind).ok_or("agent_kind_unknown")?;
    let profile = hide_agent_adapter::Profile {
        account: Some(spelled),
        ..hide_agent_adapter::Profile::builtin_default(adapter.id)
    };
    let key = adapter
        .resolve(
            &profile,
            &hide_agent_adapter::NodeEnv::new(Some(home.to_path_buf())),
        )
        .map_err(|refusal| format!("account_refused: {refusal}"))?
        .key;
    all.with_profiles([&profile])
        .reading(agent, &key)
        .map_err(|refusal| format!("account_unread: {refusal}"))
}

fn read(
    accounts: &hide_session::Accounts,
    home: &Path,
    kind: &str,
    reference_kind: &str,
    reference: &str,
    cwd: &str,
) -> Result<serde_json::Value, String> {
    let agent = hide_session::Agent::from_kind(kind).ok_or("agent_kind_unknown")?;
    if agent == hide_session::Agent::OpenCode {
        return read_opencode(accounts, reference_kind, reference, cwd);
    }
    let identity = match reference_kind {
        "id" => hide_session::SessionIdentity::id(reference),
        "path" => hide_session::SessionIdentity::path(reference),
        _ => return Err("reference_kind_unknown".to_owned()),
    };
    let path = hide_session::SessionLocator::new(accounts)
        .locate("journey", agent, Some(&identity), Some(cwd))
        .map_err(|error| format!("session_locate_failed: {error}"))?;
    let id = match &identity {
        hide_session::SessionIdentity::Id(id) => id.clone(),
        // The id the file itself names, as Hide's own reads prove it in the
        // checkout; a file name is not one (Pi's carries a timestamp before its id).
        hide_session::SessionIdentity::Path(_) => {
            hide_session::confirm_session_file(accounts, agent, &path, None, Some(cwd))
                .map_err(|error| format!("session_id_unconfirmed: {error}"))?
                .native_session_id
                .ok_or("session_id_unavailable")?
        }
    };
    let scope = hide_session::SessionReadScope {
        id,
        cwd: cwd.to_owned(),
    };
    let mut events = Vec::new();
    let mut checkpoint = None;
    for _ in 0..MAX_PAGES {
        let page = hide_session::read_session_file(accounts, agent, &path, Some(&scope), || {
            hide_session::read_conversation(home, agent, &path, &scope, checkpoint.take())
                .map_err(|error| format!("session_conversation_read_failed: {error}"))
        })?;
        events.extend(page.events.into_iter().map(
            |event| serde_json::json!({"role": event.role, "kind": event.kind, "text": event.text, "at": event.at_unix_ms}),
        ));
        if !page.has_more {
            return Ok(serde_json::json!({"path": path, "events": events}));
        }
        checkpoint = Some(page.checkpoint);
    }
    Err("session_conversation_over_page_budget".to_owned())
}

/// OpenCode keeps no session file: its database is read by the id the scope
/// names, as the node answers `Call::SessionConversation` for it.
fn read_opencode(
    accounts: &hide_session::Accounts,
    reference_kind: &str,
    reference: &str,
    cwd: &str,
) -> Result<serde_json::Value, String> {
    if reference_kind != "id" {
        return Err("session_kind_unsupported".to_owned());
    }
    let scope = hide_session::SessionReadScope {
        id: reference.to_owned(),
        cwd: cwd.to_owned(),
    };
    let mut events = Vec::new();
    let mut checkpoint = None;
    for _ in 0..MAX_PAGES {
        let page = hide_session::opencode::read_conversation(accounts, &scope, checkpoint.take())
            .map_err(|error| format!("session_conversation_read_failed: {error}"))?;
        events.extend(page.events.into_iter().map(
            |event| serde_json::json!({"role": event.role, "kind": event.kind, "text": event.text, "at": event.at_unix_ms}),
        ));
        if !page.has_more {
            let path = format!("{}{reference}", hide_session::links::OPENCODE_PREFIX);
            return Ok(serde_json::json!({"path": path, "events": events}));
        }
        checkpoint = Some(page.checkpoint);
    }
    Err("session_conversation_over_page_budget".to_owned())
}
