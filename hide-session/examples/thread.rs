//! A development reader for session threads (PRD session-thread-acp D-13):
//! one session read as ACP session updates, through the same call a product
//! reader would make. Not a product binary.
//!
//! thread <home> <agent kind> <session id> <cwd> [<session file>]
//! prints one ACP update per line, oldest first, and what the read passed
//! over on stderr, or exits 1 with the reader's refusal on stderr. Without a
//! session file, the session is located from its id as Hide locates it.

use std::path::{Path, PathBuf};

use hide_session::thread::{ThreadCursor, read_thread};
use hide_session::{Agent, SessionIdentity, SessionLocator, SessionReadScope};

/// The pages one read takes before it stops: past this the session is
/// longer than a development read should print.
const MAX_PAGES: usize = 256;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (home, kind, id, cwd, path) = match args.as_slice() {
        [home, kind, id, cwd] => (home, kind, id, cwd, None),
        [home, kind, id, cwd, path] => (home, kind, id, cwd, Some(PathBuf::from(path))),
        _ => {
            eprintln!("usage: thread <home> <agent kind> <session id> <cwd> [<session file>]");
            std::process::exit(2);
        }
    };
    if let Err(reason) = run(Path::new(home), kind, id, cwd, path) {
        eprintln!("{reason}");
        std::process::exit(1);
    }
}

fn run(home: &Path, kind: &str, id: &str, cwd: &str, path: Option<PathBuf>) -> Result<(), String> {
    let agent = Agent::from_kind(kind).ok_or("agent_kind_unknown")?;
    let path = match path {
        Some(path) => Some(path),
        None if agent.has_session_file() => Some(
            SessionLocator::new(home)
                .locate("thread", agent, Some(&SessionIdentity::id(id)), Some(cwd))
                .map_err(|error| format!("session_locate_failed: {error}"))?,
        ),
        None => None,
    };
    let scope = SessionReadScope {
        id: id.to_owned(),
        cwd: cwd.to_owned(),
    };
    let mut pages = Vec::new();
    let mut before: Option<ThreadCursor> = None;
    for _ in 0..MAX_PAGES {
        let page = read_thread(home, agent, path.as_deref(), &scope, before.as_ref())
            .map_err(|refusal| refusal.reason().to_owned())?;
        for (reason, count) in &page.skipped {
            eprintln!("skipped {reason}: {count}");
        }
        before = page.before.clone();
        pages.push(page.updates);
        if before.is_none() {
            break;
        }
    }
    if before.is_some() {
        return Err(format!("thread_longer_than_{MAX_PAGES}_pages"));
    }
    for update in pages.iter().rev().flatten() {
        println!(
            "{}",
            serde_json::to_string(update).map_err(|error| error.to_string())?
        );
    }
    Ok(())
}
