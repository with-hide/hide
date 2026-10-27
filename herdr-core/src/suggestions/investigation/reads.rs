//! The read tools an investigation may ask Hide for (PRD hide-ai-suggestions
//! D-38) and the one guard every result passes before it can reach a
//! provider (D-22). The model names a read in its answer; Hide resolves the
//! record it names, refuses a credential file before anything is read,
//! runs its own fixed read on the Project's node through [`ReadTools`], and
//! hands the model the result redacted and bounded. Nothing here writes,
//! and no read runs a program the model chose.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::super::ledger::{RecordId, RecordIndex, SessionRef, Signal};
use super::super::policy;

/// One read as an answer asks for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum Read {
    /// The conversation of the session a record happened in: the events
    /// around the record, or `count` events from event `from`.
    Session {
        record: RecordId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
    /// The end of the failed log of the CI check a record names.
    CiLog { record: RecordId },
    /// Lines of a file of the repository, by its path from the root.
    File {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from_line: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<u32>,
    },
    /// Lines of the repository's tracked files that hold `text`.
    Search {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    /// The newest commits, of the repository or of one path.
    GitLog {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
}

impl Read {
    /// The tool's name in a diagnostic, which carries no argument (B23).
    pub(crate) fn tool(&self) -> &'static str {
        match self {
            Self::Session { .. } => "session",
            Self::CiLog { .. } => "ci_log",
            Self::File { .. } => "file",
            Self::Search { .. } => "search",
            Self::GitLog { .. } => "git_log",
        }
    }
}

/// Which events of a session to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionWindow {
    /// `count` events centred on the first event at or after this time.
    Around {
        at_unix_ms: u64,
        count: u32,
    },
    From {
        from: u32,
        count: u32,
    },
}

/// What a read tool answers, before the guard redacts and bounds it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Excerpt {
    pub text: String,
    /// The tool left part of what it found out.
    pub truncated: bool,
    /// Where the excerpt sits, for the model's next read
    /// (`events 12-27 of 140`, `lines 1-120 of 300`).
    pub span: Option<String>,
}

/// One line a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub path: String,
    pub line: u32,
    pub text: String,
}

/// Why a tool read nothing, as a code the model is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadRefused {
    NotFound,
    /// The node, the session store or GitHub could not answer; the code
    /// names what, never a path or a message.
    Unavailable(String),
}

/// The fixed reads Hide runs for an investigation, on the Project's node.
/// An implementation bounds its own work by the counts it is given; the
/// guard redacts and bounds what it answers.
pub trait ReadTools: Send {
    fn session(
        &mut self,
        session: &SessionRef,
        window: SessionWindow,
    ) -> Result<Excerpt, ReadRefused>;
    /// The failed log of the check named `check` on pull request `pr`.
    fn ci_log(&mut self, pr: u32, check: &str) -> Result<Excerpt, ReadRefused>;
    fn file(&mut self, path: &str, from_line: u32, lines: u32) -> Result<Excerpt, ReadRefused>;
    /// At most [`policy::SEARCH_HITS`] hits for a fixed text, case ignored.
    fn search(&mut self, text: &str, path: Option<&str>) -> Result<Vec<SearchHit>, ReadRefused>;
    fn git_log(&mut self, path: Option<&str>, count: u32) -> Result<Excerpt, ReadRefused>;
}

/// A path the model may name: below the root, one name per `/`, and no
/// part of it a credential file, a folder named for secrets or Git's own
/// folder (whose configuration can hold a token in a remote's address).
pub(crate) fn readable_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() || path.chars().count() > policy::READ_PATH_CHARS {
        return Err("invalid_path");
    }
    if path.contains('\\') || path.chars().any(char::is_control) {
        return Err("invalid_path");
    }
    hide_platform::path::RelPath::parse(path).map_err(|_| "invalid_path")?;
    let mut names = path.split('/');
    if names.any(|name| name == ".git" || hide_memory::sensitive_path(name)) {
        return Err("credential_file");
    }
    Ok(())
}

/// Text as it may leave the machine: credentials masked, at most `bytes`
/// long and cut on a character. Answers whether anything was cut.
pub(crate) fn egress(text: &str, bytes: usize) -> (String, bool) {
    let redacted = hide_memory::redact(text).text;
    if redacted.len() <= bytes {
        return (redacted, false);
    }
    let mut end = bytes;
    while !redacted.is_char_boundary(end) {
        end -= 1;
    }
    (redacted[..end].to_owned(), true)
}

fn line(text: &str) -> String {
    text.chars().take(policy::READ_LINE_CHARS).collect()
}

/// Runs one read the model asked for and answers what the model is shown
/// of it: the read itself, then either its redacted, bounded text or the
/// code of why nothing was read. `budget` is what the turn's results may
/// still take, in bytes of the JSON they are sent as.
pub(crate) fn fulfil(
    read: &Read,
    index: &RecordIndex,
    tools: &mut dyn ReadTools,
    budget: usize,
) -> (Value, Result<(), String>) {
    let outcome = run(read, index, tools);
    let shown = serde_json::to_value(read).expect("a read serializes");
    match outcome {
        Err(code) => (json!({"read": shown, "refused": code}), Err(code)),
        Ok(excerpt) => {
            let mut limit = policy::READ_RESULT_BYTES.min(budget);
            loop {
                let (text, cut) = egress(&excerpt.text, limit);
                let value = json!({
                    "read": shown,
                    "text": text,
                    "truncated": excerpt.truncated || cut,
                    "span": excerpt.span,
                });
                let size = serde_json::to_string(&value).map_or(usize::MAX, |s| s.len());
                if size <= budget {
                    return (value, Ok(()));
                }
                // Escaping makes the JSON longer than the text: cut the text
                // by what is over, and again until the result fits.
                let over = size - budget;
                if limit == 0 {
                    let refused = "turn_read_budget".to_owned();
                    return (json!({"read": shown, "refused": refused}), Err(refused));
                }
                limit = limit.saturating_sub(over.max(64));
            }
        }
    }
}

fn run(read: &Read, index: &RecordIndex, tools: &mut dyn ReadTools) -> Result<Excerpt, String> {
    let refused = |error: ReadRefused| match error {
        ReadRefused::NotFound => "not_found".to_owned(),
        ReadRefused::Unavailable(code) => code,
    };
    match read {
        Read::Session {
            record,
            from,
            count,
        } => {
            let entry = index.get(*record).ok_or("unknown_record")?;
            let session = entry.session.as_ref().ok_or("record_has_no_session")?;
            let count = count
                .unwrap_or(policy::SESSION_EVENTS)
                .clamp(1, policy::SESSION_EVENTS_MAX);
            let window = match from {
                Some(from) => SessionWindow::From { from: *from, count },
                None => SessionWindow::Around {
                    at_unix_ms: entry.at_unix_ms,
                    count,
                },
            };
            tools.session(session, window).map_err(refused)
        }
        Read::CiLog { record } => {
            let entry = index.get(*record).ok_or("unknown_record")?;
            match (entry.signal, entry.pr) {
                (Signal::CheckFailed, Some(pr)) => tools.ci_log(pr, &entry.tag).map_err(refused),
                _ => Err("record_is_not_a_check".into()),
            }
        }
        Read::File {
            path,
            from_line,
            lines,
        } => {
            readable_path(path)?;
            let from = from_line.unwrap_or(1).max(1);
            let lines = lines
                .unwrap_or(policy::FILE_LINES)
                .clamp(1, policy::FILE_LINES_MAX);
            tools.file(path, from, lines).map_err(refused)
        }
        Read::Search { text, path } => {
            let text = text.trim();
            if text.is_empty() || text.chars().count() > policy::SEARCH_TEXT_CHARS {
                return Err("invalid_search".into());
            }
            if let Some(path) = path {
                readable_path(path)?;
            }
            let hits = tools.search(text, path.as_deref()).map_err(refused)?;
            let found = hits.len();
            // A hit in a file the model may not open is not shown either.
            let shown: Vec<String> = hits
                .into_iter()
                .filter(|hit| readable_path(&hit.path).is_ok())
                .take(policy::SEARCH_HITS)
                .map(|hit| format!("{}:{}: {}", hit.path, hit.line, line(&hit.text)))
                .collect();
            Ok(Excerpt {
                truncated: found >= policy::SEARCH_HITS,
                span: Some(format!("{} hits", shown.len())),
                text: shown.join("\n"),
            })
        }
        Read::GitLog { path, count } => {
            if let Some(path) = path {
                readable_path(path)?;
            }
            let count = count
                .unwrap_or(policy::GIT_LOG_COMMITS)
                .clamp(1, policy::GIT_LOG_COMMITS_MAX);
            tools.git_log(path.as_deref(), count).map_err(refused)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suggestions::ledger::RecordEntry;

    #[derive(Default)]
    struct Shelf {
        files: Vec<String>,
        hits: Vec<SearchHit>,
    }

    impl ReadTools for Shelf {
        fn session(&mut self, _: &SessionRef, _: SessionWindow) -> Result<Excerpt, ReadRefused> {
            Err(ReadRefused::NotFound)
        }
        fn ci_log(&mut self, _: u32, _: &str) -> Result<Excerpt, ReadRefused> {
            Err(ReadRefused::Unavailable("github_unavailable".into()))
        }
        fn file(&mut self, path: &str, _: u32, _: u32) -> Result<Excerpt, ReadRefused> {
            self.files.push(path.to_owned());
            Ok(Excerpt {
                text: "export API_KEY=sk-livevalue1234567890abcdef\nrun tests".into(),
                ..Excerpt::default()
            })
        }
        fn search(&mut self, _: &str, _: Option<&str>) -> Result<Vec<SearchHit>, ReadRefused> {
            Ok(self.hits.clone())
        }
        fn git_log(&mut self, _: Option<&str>, _: u32) -> Result<Excerpt, ReadRefused> {
            Ok(Excerpt {
                text: "x".repeat(3 * policy::READ_RESULT_BYTES),
                ..Excerpt::default()
            })
        }
    }

    fn file(path: &str) -> Read {
        Read::File {
            path: path.into(),
            from_line: None,
            lines: None,
        }
    }

    #[test]
    fn a_credential_file_is_never_read_and_its_search_hits_are_not_shown() {
        let mut shelf = Shelf {
            hits: vec![
                SearchHit {
                    path: "config/.env.local".into(),
                    line: 1,
                    text: "TOKEN=abc".into(),
                },
                SearchHit {
                    path: "scripts/test.sh".into(),
                    line: 4,
                    text: "cargo test".into(),
                },
            ],
            ..Shelf::default()
        };
        let index = RecordIndex::default();
        for path in [
            ".env",
            "apps/web/.env.production",
            "deploy/server.pem",
            "home/.ssh/id_ed25519",
            "config/secrets.yml",
            "secrets/db.yml",
            "aws/credentials",
            ".git/config",
            "../outside.txt",
            "/etc/passwd",
        ] {
            let (value, outcome) = fulfil(&file(path), &index, &mut shelf, usize::MAX);
            assert!(outcome.is_err(), "{path} was read");
            assert!(value.get("text").is_none());
        }
        assert!(shelf.files.is_empty(), "no tool was asked for them");
        let (value, _) = fulfil(
            &Read::Search {
                text: "token".into(),
                path: None,
            },
            &index,
            &mut shelf,
            usize::MAX,
        );
        assert_eq!(value["text"], "scripts/test.sh:4: cargo test");
    }

    #[test]
    fn every_result_is_redacted_and_bounded_by_the_result_and_the_turn() {
        let mut shelf = Shelf::default();
        let index = RecordIndex::default();
        let (value, outcome) = fulfil(&file("scripts/env.sh"), &index, &mut shelf, usize::MAX);
        assert_eq!(outcome, Ok(()));
        let text = value["text"].as_str().unwrap();
        assert!(!text.contains("sk-livevalue"), "{text}");
        assert!(text.contains("run tests"));

        let history = Read::GitLog {
            path: None,
            count: None,
        };
        let (value, _) = fulfil(&history, &index, &mut shelf, usize::MAX);
        assert!(serde_json::to_string(&value).unwrap().len() <= policy::READ_RESULT_BYTES + 128);
        assert_eq!(value["truncated"], true);
        let (value, _) = fulfil(&history, &index, &mut shelf, 2_000);
        assert!(serde_json::to_string(&value).unwrap().len() <= 2_000);
    }

    #[test]
    fn a_session_or_ci_read_names_a_record_of_the_right_kind() {
        let mut shelf = Shelf::default();
        let index = RecordIndex {
            entries: vec![RecordEntry {
                id: RecordId(4),
                signal: Signal::FailedTool,
                tag: "cargo test".into(),
                session: None,
                pr: None,
                at_unix_ms: 1,
                always: false,
                improvement: None,
                line: None,
                fresh: true,
            }],
        };
        let refusal = |read: Read, shelf: &mut Shelf| fulfil(&read, &index, shelf, usize::MAX).1;
        assert_eq!(
            refusal(
                Read::CiLog {
                    record: RecordId(4)
                },
                &mut shelf
            ),
            Err("record_is_not_a_check".into())
        );
        assert_eq!(
            refusal(
                Read::Session {
                    record: RecordId(9),
                    from: None,
                    count: None
                },
                &mut shelf
            ),
            Err("unknown_record".into())
        );
        assert_eq!(
            refusal(
                Read::Session {
                    record: RecordId(4),
                    from: None,
                    count: None
                },
                &mut shelf
            ),
            Err("record_has_no_session".into())
        );
    }
}
