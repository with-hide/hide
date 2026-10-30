//! Every agent's session read as a thread of ACP updates (PRD
//! session-thread-acp). Each agent's sample is a real session of the same
//! task, recorded in a throwaway checkout and stripped to the conversation:
//! describe `mock.png`, write `login.html` after it, make `greet` in
//! `app.js` return `` `Hello, ${name}!` ``, and copy `mock.png` to
//! `docs/mock-copy.png`. The expected answers come from that task and from
//! the checkout's own git diff, never from the converter's output.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use hide_agent_adapter::usage::Support;
use hide_agent_adapter::{ADAPTERS, SessionFormat, ThreadPart, ThreadReading};
use hide_session::thread::{THREAD_PAGE_BYTES, ThreadCursor, ThreadPage, read_thread, reads};
use hide_session::{Agent, SessionReadScope};
use serde_json::{Value, json};

/// The first words of the task every sample was given.
const TASK: &str = "mock.png는 로그인 카드 디자인이야";
/// The fixtures' checkout, replaced by a real folder when a sample is laid out.
const CHECKOUT: &str = "/work/app";

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/thread")
}

/// One agent's same-scenario session, laid out in a home of its own.
struct Sample {
    format: SessionFormat,
    /// Puts the sample into `home` for the checkout `cwd` and answers the
    /// session file and the scope a catalog row would name.
    place: fn(home: &Path, cwd: &Path) -> (PathBuf, SessionReadScope),
}

const SAMPLES: &[Sample] = &[
    Sample {
        format: SessionFormat::Claude,
        place: |home, cwd| {
            let id = "b92ea92e-9a98-4f83-b0cd-566cf1cdc37a";
            let path = home.join(format!(".claude/projects/-work-app/{id}.jsonl"));
            install(
                &fixtures().join(format!("claude-2.1.314/projects/-work-app/{id}.jsonl")),
                &path,
                cwd,
            );
            (path, scope(id, cwd))
        },
    },
    Sample {
        format: SessionFormat::Codex,
        place: |home, cwd| {
            let name = "rollout-2026-10-30T12-07-30-01a18cb5-3c3a-7060-b86b-164ca517dc11.jsonl";
            let path = home.join(".codex/sessions/2026/10/30").join(name);
            install(
                &fixtures()
                    .join("codex-0.166.0/sessions/2026/10/30")
                    .join(name),
                &path,
                cwd,
            );
            (path, scope("01a18cb5-3c3a-7060-b86b-164ca517dc11", cwd))
        },
    },
    Sample {
        format: SessionFormat::Grok,
        place: |home, cwd| {
            let id = "01a18cbf-c706-7260-9188-d8a0c2fd7c59";
            let folder = home.join(".grok/sessions").join(url_group(cwd)).join(id);
            for name in ["updates.jsonl", "summary.json"] {
                install(
                    &fixtures().join("grok-1.0.70").join(name),
                    &folder.join(name),
                    cwd,
                );
            }
            (folder.join("updates.jsonl"), scope(id, cwd))
        },
    },
];

/// A checkout as Grok names its group folder: every byte but
/// `A-Za-z0-9-_.~` as `%XX`.
fn url_group(cwd: &Path) -> String {
    cwd.to_str()
        .unwrap()
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

/// Copies a fixture file, naming the real checkout wherever the fixture
/// names its placeholder, in the JSON spelling of this system's path.
fn install(fixture: &Path, to: &Path, cwd: &Path) {
    let text = std::fs::read_to_string(fixture).unwrap();
    let native = serde_json::to_string(&cwd.to_str().unwrap()).unwrap();
    let native = &native[1..native.len() - 1];
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::write(to, text.replace(CHECKOUT, native)).unwrap();
}

fn scope(id: &str, cwd: &Path) -> SessionReadScope {
    SessionReadScope {
        id: id.to_owned(),
        cwd: cwd.to_str().unwrap().to_owned(),
    }
}

/// A laid-out sample: its home, checkout and the session a read names.
struct Placed {
    _home: tempfile::TempDir,
    home: PathBuf,
    cwd: PathBuf,
    agent: Agent,
    path: PathBuf,
    scope: SessionReadScope,
}

impl Placed {
    fn new(sample: &Sample) -> Self {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().canonicalize().unwrap();
        let cwd = root.join("app");
        std::fs::create_dir(&cwd).unwrap();
        let (path, scope) = (sample.place)(&root, &cwd);
        Self {
            _home: home,
            home: root,
            cwd,
            agent: Agent::from_format(sample.format),
            path,
            scope,
        }
    }

    fn read(&self, before: Option<&ThreadCursor>) -> Result<ThreadPage, hide_session::ReadRefusal> {
        read_thread(
            &self.home,
            self.agent,
            Some(&self.path),
            &self.scope,
            before,
        )
    }

    /// Every page, oldest first, as the updates' JSON.
    fn read_all(&self) -> Vec<Value> {
        let mut pages = Vec::new();
        let mut before = None;
        loop {
            let page = self.read(before.as_ref()).unwrap();
            pages.push(json_of(&page));
            match page.before {
                Some(cursor) => before = Some(cursor),
                None => break,
            }
        }
        pages.into_iter().rev().flatten().collect()
    }

    /// The checkout path a thread names, in the `/` form.
    fn wire(&self, relative: &str) -> String {
        hide_platform::path::to_wire(&self.cwd.join(relative)).unwrap()
    }
}

fn json_of(page: &ThreadPage) -> Vec<Value> {
    page.updates
        .iter()
        .map(|update| serde_json::to_value(update).unwrap())
        .collect()
}

fn of_kind<'a>(updates: &'a [Value], kind: &'a str) -> impl Iterator<Item = &'a Value> {
    updates
        .iter()
        .filter(move |update| update["sessionUpdate"] == kind)
}

/// The content blocks of every message chunk and tool result.
fn blocks(updates: &[Value]) -> Vec<&Value> {
    let mut found = Vec::new();
    for update in updates {
        match update["sessionUpdate"].as_str() {
            Some("user_message_chunk" | "agent_message_chunk") => found.push(&update["content"]),
            Some("tool_call" | "tool_call_update") => found.extend(
                update["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|item| item["type"] == "content")
                    .map(|item| &item["content"]),
            ),
            _ => {}
        }
    }
    found
}

fn diffs<'a>(updates: &'a [Value], path: &str) -> Vec<&'a Value> {
    updates
        .iter()
        .filter(|update| update["content"].is_array())
        .flat_map(|update| update["content"].as_array().unwrap())
        .filter(|item| item["type"] == "diff" && item["path"] == path)
        .collect()
}

/// Each call as a client shows it: its `tool_call` with every later
/// update's fields laid over it, an update's content replacing the call's.
fn calls(updates: &[Value]) -> std::collections::BTreeMap<String, Value> {
    let mut calls = std::collections::BTreeMap::new();
    for update in updates {
        let Some(id) = update["toolCallId"].as_str() else {
            continue;
        };
        match update["sessionUpdate"].as_str() {
            Some("tool_call") => {
                calls.insert(id.to_owned(), update.clone());
            }
            Some("tool_call_update") => {
                if let Some(call) = calls.get_mut(id) {
                    for (key, value) in update.as_object().unwrap() {
                        if key != "sessionUpdate" {
                            call[key] = value.clone();
                        }
                    }
                }
            }
            _ => {}
        }
    }
    calls
}

/// Which of the checked parts the sample's thread shows.
fn parts_shown(placed: &Placed, updates: &[Value]) -> BTreeSet<ThreadPart> {
    let mut shown = BTreeSet::new();
    let picture = blocks(updates).into_iter().any(|block| {
        (block["type"] == "image" && block["data"].as_str().is_some_and(|data| !data.is_empty()))
            || (block["type"] == "resource_link"
                && block["uri"]
                    .as_str()
                    .is_some_and(|uri| uri.ends_with("mock.png")))
    });
    if picture {
        shown.insert(ThreadPart::Image);
    }
    let created = diffs(updates, &placed.wire("login.html"))
        .into_iter()
        .any(|diff| {
            diff["oldText"].is_null()
                && diff["newText"]
                    .as_str()
                    .is_some_and(|text| text.to_ascii_lowercase().contains("<html"))
        });
    let changed = diffs(updates, &placed.wire("app.js"))
        .into_iter()
        .any(|diff| {
            diff["oldText"]
                .as_str()
                .is_some_and(|text| text.contains("return \"Hi\""))
                && diff["newText"]
                    .as_str()
                    .is_some_and(|text| text.contains("return `Hello, ${name}!`"))
        });
    if created && changed {
        shown.insert(ThreadPart::FileChange);
    }
    let ran = calls(updates).values().any(|call| {
        call["kind"] == "execute"
            && matches!(call["status"].as_str(), Some("completed" | "failed"))
            && blocks(std::slice::from_ref(call)).iter().any(|block| {
                block["type"] == "text"
                    && block["text"]
                        .as_str()
                        .is_some_and(|text| !text.trim().is_empty())
            })
    });
    if ran {
        shown.insert(ThreadPart::CommandOutput);
    }
    shown
}

fn declaration(format: SessionFormat) -> ThreadReading {
    match format.adapter().thread {
        Support::Supported(reading) => reading,
        Support::Unsupported(absence) => {
            panic!("{format:?} declares no thread: {}", absence.reason)
        }
    }
}

#[test]
fn every_agents_sample_reads_to_the_task_it_recorded() {
    for sample in SAMPLES {
        let name = format!("{:?}", sample.format);
        let placed = Placed::new(sample);
        let updates = placed.read_all();

        let opening = updates
            .iter()
            .find(|update| update["_meta"]["hide"]["origin"].is_null())
            .unwrap();
        assert_eq!(opening["sessionUpdate"], "user_message_chunk", "{name}");
        let task = updates
            .iter()
            .skip_while(|update| !update["_meta"]["hide"]["origin"].is_null())
            .take_while(|update| update["sessionUpdate"] == "user_message_chunk")
            .filter_map(|update| update["content"]["text"].as_str())
            .any(|text| text.contains(TASK));
        assert!(task, "{name}: the operator's task opens the thread");
        assert_eq!(
            updates.last().unwrap()["sessionUpdate"],
            "agent_message_chunk",
            "{name}: the agent's answer closes it"
        );

        let mut called = BTreeSet::new();
        for update in &updates {
            let id = update["toolCallId"].as_str();
            match update["sessionUpdate"].as_str() {
                Some("tool_call") => assert!(called.insert(id.unwrap()), "{name}: one call per id"),
                Some("tool_call_update") => assert!(
                    called.contains(id.unwrap()),
                    "{name}: a result follows its call"
                ),
                _ => {}
            }
        }

        let shown = parts_shown(&placed, &updates);
        let absent = declaration(sample.format)
            .absent
            .iter()
            .map(|(part, _)| *part)
            .collect::<BTreeSet<_>>();
        for part in ThreadPart::ALL {
            assert!(
                shown.contains(&part) != absent.contains(&part),
                "{name}: {part:?} is shown={} but declared absent={}",
                shown.contains(&part),
                absent.contains(&part)
            );
        }
    }
}

#[test]
fn every_declaration_matches_its_converter_and_has_a_sample() {
    let sampled = SAMPLES
        .iter()
        .map(|sample| sample.format)
        .collect::<Vec<_>>();
    for row in ADAPTERS {
        let Some(format) = row.session else {
            assert!(
                matches!(row.thread, Support::Unsupported(_)),
                "{}: no session, no thread",
                row.id
            );
            continue;
        };
        match row.thread {
            Support::Supported(reading) => {
                assert_eq!(
                    reading.format, format,
                    "{}: another agent's converter",
                    row.id
                );
                assert!(reads(format), "{}: declared without a converter", row.id);
                assert!(
                    sampled.contains(&format),
                    "{}: no same-scenario sample",
                    row.id
                );
                for (part, absence) in reading.absent {
                    assert!(
                        !absence.reason.is_empty() && !absence.would_change.is_empty(),
                        "{} {part:?}: an absence says why and what would change it",
                        row.id
                    );
                    assert!(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("..")
                            .join(absence.evidence)
                            .exists(),
                        "{} {part:?}: evidence {}",
                        row.id,
                        absence.evidence
                    );
                }
            }
            Support::Unsupported(absence) => {
                assert!(
                    !reads(format),
                    "{}: a converter it does not declare",
                    row.id
                );
                assert!(!absence.reason.is_empty(), "{}", row.id);
            }
        }
    }
    let cursor = hide_agent_adapter::SessionFormat::Cursor;
    assert!(
        matches!(cursor.adapter().thread, Support::Unsupported(_)),
        "Cursor's records are declared unreadable (B15)"
    );
}

#[test]
fn a_thread_read_writes_nothing() {
    for sample in SAMPLES {
        let placed = Placed::new(sample);
        let before = std::fs::read(&placed.path).unwrap();
        let modified = std::fs::metadata(&placed.path).unwrap().modified().unwrap();
        placed.read_all();
        assert_eq!(
            std::fs::read(&placed.path).unwrap(),
            before,
            "{:?}",
            sample.format
        );
        assert_eq!(
            std::fs::metadata(&placed.path).unwrap().modified().unwrap(),
            modified
        );
    }
}

/// The Claude sample repeated past one page, read back page by page.
#[test]
fn a_session_longer_than_a_page_reads_back_whole_from_its_cursors() {
    let placed = Placed::new(&SAMPLES[0]);
    let one = placed.read_all();
    let sample = std::fs::read_to_string(&placed.path).unwrap();
    let copies = (THREAD_PAGE_BYTES as usize / sample.len()) * 2 + 1;
    // Each copy is a session of its own calls, as a long session would be.
    let long = (0..copies)
        .map(|copy| sample.replace("toolu_", &format!("toolu_{copy}_")))
        .collect::<String>();
    std::fs::write(&placed.path, long).unwrap();

    let first = placed.read(None).unwrap();
    assert!(first.before.is_some(), "more than one page");
    let all = placed.read_all();
    assert_eq!(all.len(), one.len() * copies);
    for (index, chunk) in all.chunks(one.len()).enumerate() {
        let chunk = serde_json::to_string(chunk)
            .unwrap()
            .replace(&format!("toolu_{index}_"), "toolu_");
        assert!(
            chunk == serde_json::to_string(&one).unwrap(),
            "copy {index} reads as the sample"
        );
    }
}

fn claude_line(text: &str) -> String {
    format!(
        "{}\n",
        json!({"type":"user","uuid":text,"isSidechain":false,"message":{"role":"user","content":text}})
    )
}

#[test]
fn a_record_past_the_page_is_counted_and_the_records_around_it_still_read() {
    let placed = Placed::new(&SAMPLES[0]);
    let long = claude_line(&"x".repeat(THREAD_PAGE_BYTES as usize + 1));
    let contents = [claude_line("older"), long, claude_line("newer")].concat();
    std::fs::write(&placed.path, contents).unwrap();

    let mut texts = Vec::new();
    let mut capacity = 0;
    let mut before = None;
    loop {
        let page = placed.read(before.as_ref()).unwrap();
        capacity += page.skipped.get("line_capacity").copied().unwrap_or(0);
        texts.extend(
            json_of(&page)
                .iter()
                .rev()
                .map(|update| update["content"]["text"].as_str().unwrap().to_owned()),
        );
        match page.before {
            Some(cursor) => before = Some(cursor),
            None => break,
        }
    }
    assert_eq!(texts, ["newer", "older"]);
    assert_eq!(capacity, 1);
}

#[test]
fn a_page_of_unreadable_records_is_refused_by_name() {
    let placed = Placed::new(&SAMPLES[0]);
    std::fs::write(&placed.path, "{\"type\":\"user\",\"message\n{not json}\n").unwrap();
    assert_eq!(
        placed.read(None).unwrap_err().reason(),
        "thread_records_unreadable"
    );
    std::fs::write(
        &placed.path,
        format!(
            "{}\n",
            json!({"type":"assistant","isSidechain":false,"message":{"content":7}})
        ),
    )
    .unwrap();
    assert_eq!(
        placed.read(None).unwrap_err().reason(),
        "thread_records_unreadable",
        "a record whose shape changed"
    );
}

#[test]
fn a_cursor_of_a_replaced_file_is_refused() {
    let placed = Placed::new(&SAMPLES[0]);
    let sample = std::fs::read_to_string(&placed.path).unwrap();
    std::fs::write(&placed.path, sample.repeat(400)).unwrap();
    let cursor = placed.read(None).unwrap().before.unwrap();
    let replacement = placed.path.with_extension("new");
    std::fs::write(&replacement, sample.repeat(400)).unwrap();
    std::fs::rename(&replacement, &placed.path).unwrap();
    assert_eq!(
        placed.read(Some(&cursor)).unwrap_err().reason(),
        "thread_cursor_stale"
    );
}

#[test]
fn a_format_without_a_converter_and_a_path_outside_the_sessions_are_refused() {
    let placed = Placed::new(&SAMPLES[0]);
    assert_eq!(
        read_thread(
            &placed.home,
            Agent::Cursor,
            Some(&placed.path),
            &placed.scope,
            None
        )
        .unwrap_err()
        .reason(),
        "thread_unsupported"
    );
    let outside = placed.cwd.join("session.jsonl");
    std::fs::copy(&placed.path, &outside).unwrap();
    assert_eq!(
        read_thread(
            &placed.home,
            placed.agent,
            Some(&outside),
            &placed.scope,
            None
        )
        .unwrap_err()
        .reason(),
        "thread_outside_session_root"
    );
}

#[test]
fn hide_and_harness_wrappers_stay_apart_from_the_operators_words() {
    let placed = Placed::new(&SAMPLES[0]);
    let letter = "Hide letter letter-1 from ci-lead (claude) [request]\nCI 다시 봐줘\nFull letter: hide request show letter-1";
    let records = [
        json!({"type":"user","uuid":"a","message":{"content":"고쳐줘\n<pasted_content id=\"p1\">\npanic at main.rs:3\n</pasted_content>\n왜 그래?"}}),
        json!({"type":"user","uuid":"b","isMeta":true,"message":{"content":"Caveat: local command output"}}),
        json!({"type":"user","uuid":"c","message":{"content":"<system-reminder>hook said hi</system-reminder>"}}),
        json!({"type":"user","uuid":"d","message":{"content":letter}}),
        json!({"type":"user","uuid":"e","message":{"content":"[Request interrupted by user]"}}),
    ];
    let contents = records
        .iter()
        .map(|record| format!("{record}\n"))
        .collect::<String>();
    std::fs::write(&placed.path, contents).unwrap();
    let updates = json_of(&placed.read(None).unwrap());
    let marks = updates
        .iter()
        .map(|update| {
            (
                update["content"]["text"].as_str().unwrap().trim(),
                update["_meta"]["hide"]["origin"].as_str(),
                update["content"]["_meta"]["hide"]["pasted"] == true,
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        marks,
        [
            ("고쳐줘", None, false),
            ("panic at main.rs:3", None, true),
            ("왜 그래?", None, false),
            ("Caveat: local command output", Some("injected"), false),
            (
                "<system-reminder>hook said hi</system-reminder>",
                Some("injected"),
                false
            ),
            (letter, Some("letter"), false),
            ("[Request interrupted by user]", Some("interrupted"), false),
        ]
    );
}
