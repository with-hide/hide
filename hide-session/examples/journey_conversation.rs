//! The agent journey check's conversation reader (`scripts/agent_journey`):
//! one session's whole conversation as Hide reads it, located and read by the
//! same calls the node answers `Call::SessionConversation` with, so the check
//! never parses a native transcript itself. Not a product binary.
//!
//! journey_conversation <home> <agent kind> <id|path> <reference> <cwd>
//! prints {"path": ..., "events": [{"role", "kind", "text", "at"}...]}, `at`
//! the record's time in Unix milliseconds as the session wrote it, or exits 1
//! with the reader's own refusal on stderr.

use std::path::Path;

/// The pages one read takes before the answer is refused as too long; a
/// journey session holds a few short turns.
const MAX_PAGES: usize = 64;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [home, kind, reference_kind, reference, cwd] = args.as_slice() else {
        eprintln!("usage: journey_conversation <home> <agent kind> <id|path> <reference> <cwd>");
        std::process::exit(2);
    };
    match read(Path::new(home), kind, reference_kind, reference, cwd) {
        Ok(answer) => println!("{answer}"),
        Err(reason) => {
            eprintln!("{reason}");
            std::process::exit(1);
        }
    }
}

fn read(
    home: &Path,
    kind: &str,
    reference_kind: &str,
    reference: &str,
    cwd: &str,
) -> Result<serde_json::Value, String> {
    let agent = hide_session::Agent::from_kind(kind).ok_or("agent_kind_unknown")?;
    let identity = match reference_kind {
        "id" => hide_session::SessionIdentity::id(reference),
        "path" => hide_session::SessionIdentity::path(reference),
        _ => return Err("reference_kind_unknown".to_owned()),
    };
    let path = hide_session::SessionLocator::new(home)
        .locate("journey", agent, Some(&identity), Some(cwd))
        .map_err(|error| format!("session_locate_failed: {error}"))?;
    let id = match &identity {
        hide_session::SessionIdentity::Id(id) => id.clone(),
        hide_session::SessionIdentity::Path(_) => path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or("session_id_unavailable")?
            .to_owned(),
    };
    let scope = hide_session::SessionReadScope {
        id,
        cwd: cwd.to_owned(),
    };
    let mut events = Vec::new();
    let mut checkpoint = None;
    for _ in 0..MAX_PAGES {
        let page = hide_session::read_session_file(home, agent, &path, Some(&scope), || {
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
