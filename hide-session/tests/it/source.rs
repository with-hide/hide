//! The contract of the one layer that reads session files (`source`), held
//! for every agent format: each edge case a session file can present is
//! written once here and read through the readers features use (the label
//! worker's read, which proves the owner and folds turns, and the whole-text
//! parse catalog, archive and phone pages use).
//!
//! - A record over the record cap, such as a first message carrying a
//!   pasted screenshot, is passed over and never refuses the session; the
//!   cursor and a whole-text read settle it the same way.
//! - A last record without its newline is not read until it has one, and is
//!   then read once.
//! - A record that is not JSON is counted and reading goes on.
//! - A file replaced at its path, or truncated, is read again from its start
//!   and says why.
//! - A record that names the session (Codex `session_meta`, a Pi or omp
//!   header, Grok's `summary.json`) over the naming cap is refused for its
//!   size, never read as a session without one.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use hide_session::label_transcript::{
    LabelEventKind, LabelTranscript, LabelTranscriptRequest, read,
};
use hide_session::turns::Waiting;
use hide_session::{Agent, NAMING_RECORD_LIMIT_BYTES, ReadRefusal};
use serde_json::{Value, json};

use crate::adapters::{self, START, home, request};

/// The agents whose sessions are files of records.
const FILE_AGENTS: [Agent; 6] = [
    Agent::Claude,
    Agent::Codex,
    Agent::Grok,
    Agent::Pi,
    Agent::Omp,
    Agent::Antigravity,
];

/// One agent's fixture session where that agent keeps it.
struct Session {
    agent: Agent,
    home: tempfile::TempDir,
    /// The file of records; `None` for OpenCode's database.
    path: Option<PathBuf>,
    request: LabelTranscriptRequest,
}

impl Session {
    fn of(agent: Agent) -> Self {
        let (home, path, request) = match agent {
            Agent::Claude => {
                let home = home(agent);
                let path = home
                    .path()
                    .join(".claude/projects/-work-app/a1b2c3d4-0000-4000-8000-000000000001.jsonl");
                (home, Some(path), request(agent))
            }
            Agent::Codex => {
                let home = home(agent);
                let path = home.path().join(
                    ".codex/sessions/2026/10/03/rollout-2026-10-03T01-00-00-0199a000-0000-7000-8000-000000000002.jsonl",
                );
                (home, Some(path), request(agent))
            }
            Agent::OpenCode => (home(agent), None, request(agent)),
            Agent::Grok => {
                let native = adapters::grok::Native::new();
                (native.home, Some(native.path), native.request)
            }
            Agent::Pi => {
                let native = adapters::pi::Native::new();
                (native.home, Some(native.path), native.request)
            }
            Agent::Omp => {
                let native = adapters::omp::Native::new();
                (native.home, Some(native.path), native.request)
            }
            Agent::Antigravity => {
                let native = adapters::antigravity::Native::new();
                let path =
                    hide_session::antigravity::transcript(native.home.path(), adapters::AGY_ID);
                (native.home, Some(path), native.request)
            }
            Agent::Cursor => unreachable!("Cursor keeps a graph, not a file of records"),
        };
        Self {
            agent,
            home,
            path,
            request,
        }
    }

    fn path(&self) -> &Path {
        self.path.as_deref().expect("a file of records")
    }

    /// Every read until nothing is left, from `request`, as the label
    /// worker reads.
    fn read_from(&self, mut request: LabelTranscriptRequest) -> Reads {
        let mut reads = Reads {
            events: Vec::new(),
            skipped: BTreeMap::new(),
            rescanned: Vec::new(),
            last: None,
            next: request.clone(),
        };
        loop {
            let answer = read(self.home.path(), &request).unwrap_or_else(|refusal| {
                panic!("{:?} read refused: {refusal:?}", self.agent);
            });
            reads.events.extend(
                answer
                    .events
                    .iter()
                    .map(|event| (event.kind, event.text.clone())),
            );
            for (reason, count) in &answer.skipped_reasons {
                *reads.skipped.entry(reason.clone()).or_default() += count;
            }
            reads.rescanned.extend(answer.rescanned.clone());
            let more = answer.has_more;
            request.checkpoint = Some(answer.checkpoint.clone());
            request.subagents = answer.subagents.clone();
            request.turns = answer.turns.clone();
            request.tools = answer.tools.clone();
            reads.last = Some(answer);
            reads.next = request.clone();
            if !more {
                return reads;
            }
        }
    }

    fn read_all(&self) -> Reads {
        self.read_from(self.request.clone())
    }

    fn refusal(&self) -> ReadRefusal {
        read(self.home.path(), &self.request).unwrap_err()
    }

    /// Rewrites the file so its records are `change`d; each record keeps its
    /// own line.
    fn rewrite(&self, change: impl FnOnce(&mut Vec<String>)) {
        let mut lines = fs::read_to_string(self.path())
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        change(&mut lines);
        fs::write(
            self.path(),
            lines
                .iter()
                .map(|line| format!("{line}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }

    /// The records before which no other record may stand: the header Pi
    /// and omp name the session by, omp's title slot before it, Codex's
    /// `session_meta`.
    fn fixed_records(&self) -> usize {
        match self.agent {
            Agent::Omp => 2,
            Agent::Pi | Agent::Codex => 1,
            _ => 0,
        }
    }
}

struct Reads {
    events: Vec<(LabelEventKind, String)>,
    skipped: BTreeMap<String, usize>,
    rescanned: Vec<String>,
    last: Option<LabelTranscript>,
    /// The request that reads on from where these reads ended.
    next: LabelTranscriptRequest,
}

/// What a pasted screenshot adds to a record: more than one record's cap
/// and one read's budget, as the 1.83 MB first message of a Conductor whose
/// letters were held did.
fn screenshot() -> String {
    "A".repeat(2 * hide_session::SESSION_INCREMENT_READ_LIMIT_BYTES as usize)
}

/// Rewrites the session file at `path` so the first record `pick` chooses
/// carries what `paste` puts in it; every other line stays as written.
fn paste_screenshot(path: &Path, pick: impl Fn(&Value) -> bool, paste: impl FnOnce(&mut Value)) {
    let mut paste = Some(paste);
    let mut rewritten = String::new();
    for line in fs::read_to_string(path).unwrap().lines() {
        let mut record: Value = serde_json::from_str(line).unwrap();
        match paste.take_if(|_| pick(&record)) {
            Some(paste) => {
                paste(&mut record);
                rewritten.push_str(&record.to_string());
            }
            None => rewritten.push_str(line),
        }
        rewritten.push('\n');
    }
    assert!(paste.is_none(), "the session holds the picked record");
    fs::write(path, rewritten).unwrap();
}

/// Puts a pasted screenshot into the session's first message from the
/// operator, as each agent records one.
fn paste_into_first_message(session: &Session) {
    if session.agent == Agent::OpenCode {
        // OpenCode keeps a message's parts as database rows: the pasted image
        // is a row of the first message over the row cap.
        rusqlite::Connection::open(
            session
                .home
                .path()
                .join(".local/share/opencode/opencode.db"),
        )
        .unwrap()
        .execute(
            "INSERT INTO part VALUES ('prt_screenshot', 'msg_01', ?1, ?2, ?2, ?3)",
            rusqlite::params![
                session.request.reference_value,
                START + 1,
                json!({"type": "file", "mime": "image/png", "filename": "screenshot.png",
                    "url": format!("data:image/png;base64,{}", screenshot())})
                .to_string()
            ],
        )
        .unwrap();
        return;
    }
    let path = session.path();
    match session.agent {
        Agent::Claude => {
            // The records Claude Code writes before the first message come
            // first; the session id is on that message's record and after.
            let fixture = fs::read_to_string(path).unwrap();
            fs::write(
                path,
                format!(
                    "{}\n{fixture}",
                    json!({"type": "permission-mode", "permissionMode": "default",
                        "sessionId": "a1b2c3d4-0000-4000-8000-000000000001"})
                ),
            )
            .unwrap();
            paste_screenshot(
                path,
                |record| record["type"] == "user",
                |record| {
                    let text = record["message"]["content"].take();
                    record["message"]["content"] = json!([
                        {"type": "text", "text": text},
                        {"type": "image", "source": {"type": "base64",
                            "media_type": "image/png", "data": screenshot()}}]);
                },
            );
        }
        Agent::Codex => paste_screenshot(
            path,
            |record| {
                record["payload"]["role"] == "user"
                    && record["payload"]["content"][0]["text"]
                        .as_str()
                        .is_some_and(|text| text.starts_with("요청 보기"))
            },
            |record| {
                record["payload"]["content"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"type": "input_image",
                        "image_url": format!("data:image/png;base64,{}", screenshot())}));
            },
        ),
        // Grok writes a pasted image inline in the prompt's record.
        Agent::Grok => paste_screenshot(
            path,
            |record| record["params"]["update"]["sessionUpdate"] == "user_message_chunk",
            |record| {
                record["params"]["update"]["content"] =
                    json!({"type": "image", "data": screenshot(), "mimeType": "image/png"});
            },
        ),
        Agent::Pi | Agent::Omp => paste_screenshot(
            path,
            |record| record["message"]["role"] == "user",
            |record| {
                record["message"]["content"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!({"type": "image", "data": screenshot(), "mimeType": "image/png"}));
            },
        ),
        Agent::Antigravity => paste_screenshot(
            path,
            |record| record["type"] == "USER_INPUT",
            |record| {
                record["content"] =
                    json!(format!("<USER_REQUEST>\n{}\n</USER_REQUEST>", screenshot()));
            },
        ),
        Agent::OpenCode | Agent::Cursor => unreachable!(),
    }
}

/// The reasons a capacity-passed record was counted under.
fn capacity_reasons(skipped: &BTreeMap<String, usize>) -> BTreeMap<String, usize> {
    skipped
        .iter()
        .filter(|(reason, _)| reason.contains("capacity"))
        .map(|(reason, count)| (reason.clone(), *count))
        .collect()
}

/// A session that opens with a pasted screenshot is still proven and read
/// to its end by every agent's reader: the reply after the screenshot is
/// read, and an agent whose read reports turns gives the doorbell a finished
/// turn (`Waiting::Nothing`), not an unread one. A whole-text read counts the
/// passed record under the same reason the cursor does.
#[test]
fn a_record_over_the_cap_is_passed_over_and_never_refuses_the_session() {
    for agent in FILE_AGENTS.into_iter().chain([Agent::OpenCode]) {
        let session = Session::of(agent);
        paste_into_first_message(&session);
        let reads = session.read_all();
        assert!(
            reads
                .events
                .iter()
                .any(|(kind, text)| *kind == LabelEventKind::Assistant
                    && text.starts_with("PR을 열었습니다")),
            "{agent:?} reads the reply after the screenshot"
        );
        if agent.reports_turns() {
            assert_eq!(
                reads
                    .last
                    .as_ref()
                    .and_then(|answer| answer.turns.as_ref())
                    .and_then(|turns| turns.waiting()),
                Some(Waiting::Nothing),
                "{agent:?} settles its turn"
            );
        }
        if session.path.is_some() {
            let whole = hide_session::parse_events(
                agent,
                &hide_session::read_bounded(session.path(), hide_session::SESSION_READ_LIMIT_BYTES)
                    .unwrap(),
            );
            let whole = whole
                .skipped_reasons
                .iter()
                .map(|(reason, count)| (reason.as_str().to_owned(), *count))
                .collect();
            assert_eq!(
                capacity_reasons(&whole),
                capacity_reasons(&reads.skipped),
                "{agent:?}: a whole-text read settles the record as the cursor does"
            );
            assert!(
                !capacity_reasons(&whole).is_empty(),
                "{agent:?} counts the passed record"
            );
        }
    }
}

/// A last record still being written is not read until its newline is,
/// and then exactly once: the reads before and after it hold what one read
/// of the finished file holds.
#[test]
fn a_last_record_without_its_newline_waits_and_is_read_once() {
    for agent in FILE_AGENTS {
        let session = Session::of(agent);
        let whole = session.read_all().events;
        let bytes = fs::read(session.path()).unwrap();
        assert_eq!(bytes.last(), Some(&b'\n'));
        fs::write(session.path(), &bytes[..bytes.len() - 1]).unwrap();
        let before = session.read_all();
        let unfinished = bytes[..bytes.len() - 1]
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |newline| newline as u64 + 1);
        assert_eq!(
            before.last.as_ref().unwrap().checkpoint.offset(),
            unfinished,
            "{agent:?} stops at the unfinished record"
        );
        fs::OpenOptions::new()
            .append(true)
            .open(session.path())
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        let after = session.read_from(before.next.clone());
        let texts = |events: &[(LabelEventKind, String)]| {
            events
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<String>()
        };
        assert_eq!(
            texts(&[before.events, after.events].concat()),
            texts(&whole),
            "{agent:?} reads the finished record once"
        );
    }
}

/// A line that is no JSON is counted as such and the records after it are
/// read as if it were not there.
#[test]
fn a_record_that_is_not_json_is_counted_and_reading_goes_on() {
    for agent in FILE_AGENTS {
        let session = Session::of(agent);
        let whole = session.read_all().events;
        let at = session.fixed_records() + 1;
        session.rewrite(|lines| lines.insert(at, "{\"type\": \"user\", \"cut".to_owned()));
        let reads = session.read_all();
        assert_eq!(reads.events, whole, "{agent:?} reads past the broken line");
        assert_eq!(
            reads.skipped.get("malformed_json"),
            Some(&1),
            "{agent:?} counts the broken line"
        );
    }
}

/// A file replaced at its path (an editor's or a tool's atomic write) is
/// read again from its start, and the read says so.
#[test]
fn a_replaced_file_is_read_again_from_its_start() {
    for agent in FILE_AGENTS {
        let session = Session::of(agent);
        let first = session.read_all();
        let copy = session.path().with_extension("replacement");
        fs::copy(session.path(), &copy).unwrap();
        fs::rename(&copy, session.path()).unwrap();
        let again = session.read_from(first.next.clone());
        assert_eq!(again.rescanned, ["replaced"], "{agent:?}");
        assert_eq!(again.events, first.events, "{agent:?} reads it whole again");
    }
}

/// A file cut shorter than where a reader stood is read again from its
/// start, and the read says so.
#[test]
fn a_truncated_file_is_read_again_from_its_start() {
    for agent in FILE_AGENTS {
        let session = Session::of(agent);
        let first = session.read_all();
        session.rewrite(|lines| {
            lines.pop();
        });
        let again = session.read_from(first.next.clone());
        assert_eq!(again.rescanned, ["truncated"], "{agent:?}");
        assert!(!again.events.is_empty(), "{agent:?} reads what is left");
    }
}

/// The record that names a session holds its id and checkout at a fixed
/// place and nothing is pasted into it, so one over the naming cap is
/// refused for its size, not passed over and not read as a session that
/// names no one; within the cap, past the record cap, it is read.
#[test]
fn a_naming_record_over_its_cap_is_refused_for_its_size() {
    let refused = ReadRefusal::Refused(format!(
        "session_capacity:line_bytes:{NAMING_RECORD_LIMIT_BYTES}"
    ));
    let over = "A".repeat(NAMING_RECORD_LIMIT_BYTES + 1);
    let within = "A".repeat(2 * hide_session::SESSION_LINE_LIMIT_BYTES);
    for (agent, pick, key) in [
        (Agent::Codex, "session_meta", "instructions"),
        (Agent::Pi, "session", "name"),
        (Agent::Omp, "session", "title"),
    ] {
        for (pad, answer) in [(&within, None), (&over, Some(&refused))] {
            let session = Session::of(agent);
            paste_screenshot(
                session.path(),
                |record| record["type"] == pick,
                |record| match agent {
                    Agent::Codex => record["payload"][key] = json!(pad),
                    _ => record[key] = json!(pad),
                },
            );
            match answer {
                Some(refused) => assert_eq!(&session.refusal(), refused, "{agent:?}"),
                None => assert!(!session.read_all().events.is_empty(), "{agent:?}"),
            }
        }
    }
    let grok = adapters::grok::Native::new();
    grok.summary(|summary| summary["generated_title"] = json!(over));
    assert_eq!(
        read(grok.home.path(), &grok.request).unwrap_err(),
        refused,
        "Grok's summary.json"
    );
    // `plan_mode.json` is a whole file of a few fields Grok rewrites, under
    // its own small cap.
    let grok = adapters::grok::Native::new();
    fs::write(
        grok.folder().join("plan_mode.json"),
        json!({"state": "Active", "awaiting_plan_approval": true, "pad": " ".repeat(4 * 1024)})
            .to_string(),
    )
    .unwrap();
    assert_eq!(
        read(grok.home.path(), &grok.request).unwrap_err(),
        ReadRefusal::Refused("label_session_metadata_line_capacity".into()),
    );
}
