//! The session adapter contract (PRD overview-request-view D-14, D-18):
//! every agent's real record format, at the version a fixture names, reads
//! to the same facts. A new agent, or a new version of one, is supported
//! when its fixture passes this test.
//!
//! Each fixture is one session in that agent's own format holding the same
//! conversation: the operator's two-line request, a hook's injected text,
//! a tool that printed a pull request address, a Hide letter from
//! `ci-lead`, an image sent alone, and a reply that mentions the pull
//! request and an older one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hide_session::label_transcript::{
    LabelEventKind, LabelTranscript, LabelTranscriptRequest, read,
};
use hide_session::turns::{Waiting, WakeLoss};
use hide_session::{Agent, PrSighting};

#[test]
fn an_oversized_codex_compaction_keeps_its_reset_across_cursor_restarts() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("rollout.jsonl");
    let prefix = "{\"type\":\"session_meta\",\"payload\":{}}\n";
    let record = serde_json::json!({"type":"compacted", "timestamp":"2026-10-21T01:00:00Z", "payload":{
        "replacement_history":[{"role":"user","content":"PRIVATE_PAYLOAD".repeat(100_000),"timestamp":"PRIVATE_PAYLOAD".repeat(100_000)}],
        "retained_context":null
    }});
    std::fs::write(&path, format!("{prefix}{record}\n")).unwrap();
    let mut cursor = hide_session::ConversationCursor::new();
    let mut events = Vec::new();
    let mut offsets = Vec::new();
    let mut polls = 0;
    loop {
        let read = cursor.read(Agent::Codex, &path).unwrap();
        events.extend(read.events);
        offsets.extend(read.event_offsets);
        polls += 1;
        let checkpoint = cursor.checkpoint();
        let more = cursor.has_more();
        let serialized = serde_json::to_string(&checkpoint).unwrap();
        assert!(!serialized.contains("PRIVATE_PAYLOAD"));
        cursor =
            hide_session::ConversationCursor::restore(serde_json::from_str(&serialized).unwrap());
        if !more {
            break;
        }
    }
    assert!(polls > 1);
    assert_eq!(offsets, vec![prefix.len() as u64]);
    assert_eq!(events.len(), 1);
    assert!(events[0].context_reset && events[0].text.is_empty());
    assert_eq!(events[0].at_unix_ms, 1792544400000);
    assert!(cursor.read(Agent::Codex, &path).unwrap().events.is_empty());
}

#[test]
fn context_resets_require_native_metadata_and_keep_their_record_offsets() {
    let at = "2026-10-21T01:00:00Z";
    let rows = [
        serde_json::json!({"type":"user","timestamp":at,"origin":{"kind":"human"},"message":{"content":"SessionStart:compact - please compact this context"}}),
        serde_json::json!({"type":"attachment","timestamp":at,"attachment":{"type":"hook_success","hookName":"SessionStart:compact"}}),
        serde_json::json!({"type":"attachment","timestamp":at,"attachment":{"type":"hook_success","hookName":"SessionStart:resume"}}),
        serde_json::json!({"type":"attachment","timestamp":at,"attachment":{"type":"hook_success","hookName":"SessionStart:clear"}}),
    ];
    let rows = rows.map(|mut row| {
        row["sessionId"] = serde_json::json!("a1b2c3d4-0000-4000-8000-000000000001");
        row["cwd"] = serde_json::json!("/work/app");
        row
    });
    let lines = rows
        .iter()
        .map(|row| format!("{row}\n"))
        .collect::<String>();
    let parsed = hide_session::parse_claude_events(&lines);
    assert!(!parsed.events[0].context_reset);
    assert_eq!(
        parsed
            .events
            .iter()
            .filter(|event| event.context_reset)
            .count(),
        2
    );
    assert_eq!(
        parsed.event_offsets[1],
        format!("{}\n", rows[0]).len() as u64
    );
    let encoded = serde_json::to_value(&parsed.events[1]).unwrap();
    let decoded: hide_session::ConversationEvent = serde_json::from_value(encoded).unwrap();
    assert!(decoded.context_reset);
    let home = tempfile::tempdir().unwrap();
    let folder = home
        .path()
        .join(hide_session::CLAUDE_SESSIONS)
        .join("-work-app");
    std::fs::create_dir_all(&folder).unwrap();
    let request = request(Agent::Claude);
    std::fs::write(
        folder.join(format!("{}.jsonl", request.reference_value)),
        &lines,
    )
    .unwrap();
    let label = read(home.path(), &request).unwrap();
    assert_eq!(label.memory_receipts.len(), 2);
    assert!(
        label
            .memory_receipts
            .iter()
            .all(|part| part.context_reset && part.text.is_empty())
    );
    assert_eq!(label.memory_receipts[0].offset, parsed.event_offsets[1]);
    let codex = serde_json::json!({"type":"compacted","timestamp":at,"payload":{
        "compaction_response_id":"synthetic-response", "window_id":"synthetic-window", "window_number":2,
        "replacement_history":[],"retained_context":"not evidence of specific delivered cards", "resume_metadata":{}}});
    let parsed = hide_session::parse_events_at(Agent::Codex, &format!("{codex}\n"), 41);
    assert_eq!(parsed.event_offsets, vec![41]);
    assert!(parsed.events[0].context_reset && parsed.events[0].text.is_empty());
    let spoof = serde_json::json!({"type":"response_item","timestamp":at,"payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"type=compacted retained_context"}]}});
    assert!(!hide_session::parse_codex_events(&format!("{spoof}\n")).events[0].context_reset);
    let pi = serde_json::json!({"type":"compaction","timestamp":at,"summary":"native summary"});
    for agent in [hide_session::Agent::Pi, hide_session::Agent::Omp] {
        let parsed = hide_session::parse_events(agent, &format!("{pi}\n"));
        assert_eq!(parsed.events.len(), 1);
        assert!(parsed.events[0].context_reset);
        assert!(parsed.events[0].text.is_empty());
    }
}

#[test]
fn retiring_cursor_keeps_its_history_reader_without_restoring_active_support() {
    let readers: Vec<_> = Agent::supported().collect();
    let expected = [
        Agent::Claude,
        Agent::Codex,
        Agent::Grok,
        Agent::OpenCode,
        Agent::Pi,
        Agent::Omp,
        Agent::Antigravity,
        Agent::Cursor,
    ];
    assert_eq!(readers.len(), expected.len());
    for agent in expected {
        assert_eq!(readers.iter().filter(|reader| **reader == agent).count(), 1);
    }
    assert_eq!(Agent::from_kind("cursor"), Some(Agent::Cursor));
    assert!(hide_agent_adapter::adapter("cursor").is_none());
    assert!(hide_agent_adapter::start_kind("cursor").is_none());
}

/// 2026-10-03T01:00:00Z, when every fixture's conversation starts.
pub(crate) const START: u64 = 1_790_989_200_000;

pub(crate) fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/adapters")
}

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// A home holding one agent's fixture where that agent keeps its sessions.
pub(crate) fn home(agent: Agent) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    match agent {
        Agent::Claude => copy_tree(
            &fixtures().join("claude-2.1.288"),
            &home.path().join(".claude"),
        ),
        Agent::Codex => copy_tree(
            &fixtures().join("codex-0.160.0"),
            &home.path().join(".codex"),
        ),
        Agent::Grok | Agent::Pi | Agent::Omp | Agent::Cursor | Agent::Antigravity => {
            unreachable!("Native file fixtures bind their cwd to an owned checkout")
        }
        Agent::OpenCode => {
            let folder = home.path().join(".local/share/opencode");
            std::fs::create_dir_all(&folder).unwrap();
            let database = rusqlite::Connection::open(folder.join("opencode.db")).unwrap();
            database
                .execute_batch(
                    &std::fs::read_to_string(fixtures().join("opencode-1.18.30/opencode.sql"))
                        .unwrap(),
                )
                .unwrap();
        }
    }
    home
}

pub(crate) fn request(agent: Agent) -> LabelTranscriptRequest {
    let id = match agent {
        Agent::Claude => "a1b2c3d4-0000-4000-8000-000000000001",
        Agent::Codex => "0199a000-0000-7000-8000-000000000002",
        Agent::Grok => GROK_ID,
        Agent::Pi => "pi-native-a",
        Agent::Omp => "omp-native-a",
        Agent::Cursor => "a1b2c3d4-0000-4000-8000-000000000001",
        Agent::OpenCode => "ses_0a1b2c3d4e5f60718293a4b5c6",
        Agent::Antigravity => AGY_ID,
    };
    LabelTranscriptRequest {
        agent,
        reference_kind: "id".to_owned(),
        reference_value: id.to_owned(),
        cwd: Some("/work/app".to_owned()),
        checkpoint: None,
        subagents: BTreeMap::new(),
        turns: None,
    }
}

/// Reads until nothing is left, as the label worker does.
pub(crate) fn read_whole(home: &Path, agent: Agent) -> LabelTranscript {
    let mut request = request(agent);
    let mut whole = read(home, &request).unwrap();
    while whole.has_more {
        request.checkpoint = Some(whole.checkpoint.clone());
        request.subagents = whole.subagents.clone();
        let next = read(home, &request).unwrap();
        whole.events.extend(next.events);
        whole.pr_sightings.extend(next.pr_sightings);
        whole.memory_receipts.extend(next.memory_receipts);
        whole.has_more = next.has_more;
        whole.checkpoint = next.checkpoint;
        whole.subagents = next.subagents;
        whole.title = next.title.or(whole.title);
        whole.custom_title = next.custom_title.or(whole.custom_title);
    }
    whole
}

pub(crate) fn sighted(transcript: &LabelTranscript, number: u64) -> Option<&PrSighting> {
    transcript
        .pr_sightings
        .iter()
        .find(|sighting| sighting.repository == "acme/app" && sighting.number == number)
}

/// Real provider records with deliberately priced byte lengths, rather than
/// a mocked reader: the adapter contract admits 2MiB of subagents per poll.
fn subagent_poll_fixture() -> (tempfile::TempDir, PathBuf, LabelTranscriptRequest) {
    let home = tempfile::tempdir().unwrap();
    let project = home.path().join(".claude/projects/-work-app");
    std::fs::create_dir_all(&project).unwrap();
    let path = project.join("budget-session.jsonl");
    let record = serde_json::json!({"type":"user","sessionId":"budget-session",
        "origin":{"kind":"human"},"timestamp":"2026-10-03T01:00:00Z",
        "message":{"role":"user","content":"read subagent pull requests"}});
    std::fs::write(&path, format!("{record}\n")).unwrap();
    let folder = project.join("budget-session/subagents");
    std::fs::create_dir_all(&folder).unwrap();
    let request = LabelTranscriptRequest {
        agent: Agent::Claude,
        reference_kind: "path".to_owned(),
        reference_value: path.display().to_string(),
        cwd: None,
        checkpoint: None,
        subagents: BTreeMap::new(),
        turns: None,
    };
    (home, folder, request)
}

fn sized_tool_record(number: u64, bytes: usize) -> String {
    sized_tool_result_record(number, bytes, Some("t"))
}

fn sized_tool_result_record(number: u64, bytes: usize, tool_use_id: Option<&str>) -> String {
    let mut record = serde_json::json!({"type":"user","sessionId":"budget-session",
        "timestamp":"2026-10-03T01:00:01Z","message":{"role":"user",
        "content":[{"type":"tool_result",
        "content":format!("https://github.com/acme/app/pull/{number}")}]}});
    if let Some(id) = tool_use_id {
        record["message"]["content"][0]["tool_use_id"] = id.into();
    }
    let padding = bytes.checked_sub(record.to_string().len() + 1).unwrap();
    let text = record["message"]["content"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    record["message"]["content"][0]["content"] = format!("{text}{}", " ".repeat(padding)).into();
    let line = format!("{record}\n");
    assert_eq!(line.len(), bytes);
    line
}

fn write_poll_subagent(folder: &Path, index: u64, contents: &str) {
    let path = folder.join(format!("agent-{index}.jsonl"));
    std::fs::write(&path, contents).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(
            std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 - index),
        )
        .unwrap();
}

fn resume_subagent_poll(request: &mut LabelTranscriptRequest, answer: &LabelTranscript) {
    // A relaunch restores only durable checkpoints, including the bounded
    // classifier of an oversized record; no pending transcript is persisted.
    request.checkpoint =
        Some(serde_json::from_slice(&serde_json::to_vec(&answer.checkpoint).unwrap()).unwrap());
    request.subagents =
        serde_json::from_slice(&serde_json::to_vec(&answer.subagents).unwrap()).unwrap();
}

#[test]
fn claude_subagent_poll_caps_aggregate_bytes_and_resumes_a_partial_record_once() {
    let (home, folder, mut request) = subagent_poll_fixture();
    for index in 1..=3 {
        let contents: String = (1..=9)
            .map(|record| sized_tool_record(index * 100 + record, 100 * 1024))
            .collect();
        write_poll_subagent(&folder, index, &contents);
    }
    let first = read(home.path(), &request).unwrap();
    let committed_bytes: u64 = first.subagents.values().map(|cursor| cursor.offset()).sum();
    assert!(
        committed_bytes <= 2 * 1024 * 1024,
        "one subagent poll committed {committed_bytes} bytes against the 2MiB contract"
    );
    assert_eq!(first.subagents["agent-1.jsonl"].offset(), 900 * 1024);
    assert_eq!(first.subagents["agent-2.jsonl"].offset(), 900 * 1024);
    assert_eq!(first.subagents["agent-3.jsonl"].offset(), 200 * 1024);
    assert!(first.has_more);
    let mut sightings: Vec<_> = first.pr_sightings.iter().map(|pr| pr.number).collect();
    assert_eq!(
        sightings,
        (101..=109)
            .chain(201..=209)
            .chain(301..=302)
            .collect::<Vec<_>>()
    );
    resume_subagent_poll(&mut request, &first);
    let next = read(home.path(), &request).unwrap();
    sightings.extend(next.pr_sightings.iter().map(|pr| pr.number));
    assert_eq!(
        sightings,
        (101..=109)
            .chain(201..=209)
            .chain(301..=309)
            .collect::<Vec<_>>()
    );
    assert_eq!(next.subagents["agent-3.jsonl"].offset(), 900 * 1024);
    assert!(!next.has_more);
    resume_subagent_poll(&mut request, &next);
    assert!(read(home.path(), &request).unwrap().pr_sightings.is_empty());
}

#[test]
fn claude_subagent_poll_resumes_an_oversized_tool_discard_across_its_budget() {
    let (home, folder, mut request) = subagent_poll_fixture();
    for index in 1..=2 {
        let contents: String = (1..=7)
            .map(|record| sized_tool_record(index * 100 + record, 100 * 1024))
            .collect();
        write_poll_subagent(&folder, index, &contents);
    }
    write_poll_subagent(
        &folder,
        3,
        // This byte-budget test discards only an uncorrelated result.
        // A correlated native result may answer a question and must refuse
        // at the line cap instead of authorizing a silent discard.
        &(sized_tool_result_record(300, 900 * 1024, None) + &sized_tool_record(301, 1024)),
    );
    let first = read(home.path(), &request).unwrap();
    assert_eq!(first.subagents["agent-3.jsonl"].offset(), 648 * 1024);
    assert!(first.has_more);
    assert!(first.pr_sightings.iter().all(|pr| pr.number < 300));
    resume_subagent_poll(&mut request, &first);
    let next = read(home.path(), &request).unwrap();
    assert_eq!(
        next.pr_sightings
            .iter()
            .map(|pr| pr.number)
            .collect::<Vec<_>>(),
        [301]
    );
    assert!(!next.has_more);
    resume_subagent_poll(&mut request, &next);
    assert!(read(home.path(), &request).unwrap().pr_sightings.is_empty());
}

#[test]
fn claude_subagent_poll_counts_the_bytes_of_a_skipped_oversized_record() {
    let (home, folder, mut request) = subagent_poll_fixture();
    let oversized = serde_json::json!({"type":"assistant","sessionId":"budget-session",
        "message":{"role":"assistant","content":[{"type":"text","text":"x".repeat(900 * 1024)}]}});
    for index in 1..=2 {
        write_poll_subagent(&folder, index, &format!("{oversized}\n"));
    }
    let contents: String = (301..=309)
        .map(|record| sized_tool_record(record, 100 * 1024))
        .collect();
    write_poll_subagent(&folder, 3, &contents);
    let first = read(home.path(), &request).unwrap();
    assert_eq!(
        first
            .pr_sightings
            .iter()
            .map(|pr| pr.number)
            .collect::<Vec<_>>(),
        [301, 302]
    );
    assert!(first.has_more);
    // The oversized text is lost, and the files are read past it.
    for index in 1..=2 {
        assert_eq!(
            first.subagents[&format!("agent-{index}.jsonl")].offset(),
            format!("{oversized}\n").len() as u64
        );
    }
    resume_subagent_poll(&mut request, &first);
    let next = read(home.path(), &request).unwrap();
    assert_eq!(
        next.pr_sightings
            .iter()
            .map(|pr| pr.number)
            .collect::<Vec<_>>(),
        (303..=309).collect::<Vec<_>>()
    );
    assert!(!next.has_more);
}

/// PRD agent-blocked-state B18, D-26: only Claude Code's records prove a
/// background task is running, and a device never outlives the process that
/// started it. Every other agent reports none, so its wait stays Herdr's.
mod wake_devices {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn session_file(home: &Path, agent: Agent) -> PathBuf {
        match agent {
            Agent::Claude => home.join(".claude/projects/-work-app/a1b2c3d4-0000-4000-8000-000000000001.jsonl"),
            _ => home.join(".codex/sessions/2026/10/03/rollout-2026-10-03T01-00-00-0199a000-0000-7000-8000-000000000002.jsonl"),
        }
    }

    fn append(path: &Path, records: &[serde_json::Value]) {
        let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
        for record in records {
            writeln!(file, "{record}").unwrap();
        }
    }

    fn boot(name: &str) -> serde_json::Value {
        json!({"type": "attachment", "timestamp": "2026-10-03T01:10:00.000Z",
            "attachment": {"type": "hook_success", "hookName": name}})
    }

    fn started(text: &str) -> serde_json::Value {
        json!({"type": "user", "timestamp": "2026-10-03T01:10:00.000Z",
            "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "toolu_x", "content": text}]}})
    }

    fn ended(id: &str) -> serde_json::Value {
        json!({"type": "queue-operation", "operation": "enqueue", "timestamp": "2026-10-03T01:20:00.000Z",
            "content": format!("<task-notification>\n<task-id>{id}</task-id>\n<status>completed</status>\n</task-notification>")})
    }

    fn live(answer: &LabelTranscript) -> Vec<Option<u64>> {
        answer
            .turns
            .as_ref()
            .unwrap()
            .work_records(0)
            .into_iter()
            .filter(|record| record.state == hide_agent_adapter::work::WorkState::Running)
            .map(|record| record.expires_at_unix_ms)
            .collect()
    }

    #[test]
    fn a_background_task_is_proven_from_its_process_start_until_its_notification() {
        let home = home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        assert_eq!(
            live(&read_whole(home.path(), Agent::Claude)),
            [],
            "no process start is read"
        );

        append(
            &path,
            &[started("Command running in background with ID: early")],
        );
        assert_eq!(
            live(&read_whole(home.path(), Agent::Claude)),
            [],
            "a start before any boundary is unproven"
        );

        append(
            &path,
            &[
                boot("SessionStart:resume"),
                started("Command running in background with ID: bg1"),
            ],
        );
        assert_eq!(live(&read_whole(home.path(), Agent::Claude)), [None]);

        append(&path, &[ended("bg1")]);
        assert_eq!(live(&read_whole(home.path(), Agent::Claude)), []);

        append(
            &path,
            &[
                started("Command running in background with ID: bg2"),
                started("Monitor started (task mon1, expires in 30m unless the source ends first)"),
            ],
        );
        let two = live(&read_whole(home.path(), Agent::Claude));
        assert_eq!(two.len(), 2);
        assert_eq!(two[0], None);
        assert_eq!(two[1], Some(1_790_989_200_000 + 10 * 60_000 + 30 * 60_000));

        assert!(
            !read_whole(home.path(), Agent::Claude)
                .turns
                .unwrap()
                .wake_vanished()
        );
        append(&path, &[boot("SessionStart:startup")]);
        let restarted = read_whole(home.path(), Agent::Claude);
        assert_eq!(
            live(&restarted),
            [],
            "a new process ends the old one's tasks"
        );
        assert!(
            restarted.turns.unwrap().wake_vanished(),
            "the work it waited for is gone"
        );
        append(
            &path,
            &[started("Command running in background with ID: fresh")],
        );
        let fresh = read_whole(home.path(), Agent::Claude);
        assert_eq!(live(&fresh), [None]);
        assert!(
            !fresh.turns.unwrap().wake_vanished(),
            "a new device begins again"
        );
    }

    #[test]
    fn several_start_hook_records_in_a_row_keep_the_vanished_device_known() {
        let home = home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        append(
            &path,
            &[
                boot("SessionStart:startup"),
                started("Command running in background with ID: bg1"),
                boot("SessionStart:resume"),
                boot("SessionStart:resume"),
                boot("SessionStart:resume"),
            ],
        );
        let read = read_whole(home.path(), Agent::Claude);
        assert_eq!(live(&read), []);
        assert!(read.turns.unwrap().wake_vanished());
    }

    #[test]
    fn a_start_marker_quoted_inside_a_tool_result_proves_no_device() {
        let home = home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        append(
            &path,
            &[
                boot("SessionStart:startup"),
                started(
                    "docs/status-model.md:106: the text 'Command running in background with ID: X' starts a device",
                ),
                started(
                    "note: the shell said Command did not complete within its 5s timeout and was moved to the background (ID: Y)",
                ),
                started("see Monitor started (task Z, expires in 30m)"),
                started("quoted Async agent launched successfully agentId: W"),
            ],
        );
        assert_eq!(live(&read_whole(home.path(), Agent::Claude)), []);

        append(
            &path,
            &[started(
                "Command did not complete within its 600s timeout and was moved to the background (ID: real)",
            )],
        );
        assert_eq!(live(&read_whole(home.path(), Agent::Claude)), [None]);
    }

    #[test]
    fn an_incremental_read_continues_the_devices_of_the_read_before_it() {
        let home = home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        append(
            &path,
            &[
                boot("SessionStart:startup"),
                started("Command running in background with ID: bg1"),
            ],
        );
        let first = read_whole(home.path(), Agent::Claude);
        assert_eq!(live(&first), [None]);

        append(
            &path,
            &[started("Command running in background with ID: bg2")],
        );
        let mut request = request(Agent::Claude);
        request.checkpoint = Some(first.checkpoint.clone());
        request.turns = first.turns.clone();
        let second = read(home.path(), &request).unwrap();
        assert_eq!(live(&second).len(), 2, "the first task is remembered");
        request.checkpoint = Some(second.checkpoint.clone());
        request.turns = second.turns.clone();
        append(&path, &[ended("bg1")]);
        let third = read(home.path(), &request).unwrap();
        assert_eq!(live(&third).len(), 1);

        // The same read repeated from the earlier checkpoint folds nothing twice.
        request.checkpoint = Some(first.checkpoint.clone());
        request.turns = third.turns.clone();
        assert_eq!(live(&read(home.path(), &request).unwrap()).len(), 1);
    }

    #[test]
    fn more_devices_than_the_cap_prove_none_until_the_next_process() {
        let home = home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        append(&path, &[boot("SessionStart:startup")]);
        let many: Vec<_> = (0..=hide_session::turns::WAKE_DEVICE_LIMIT)
            .map(|index| started(&format!("Command running in background with ID: t{index}")))
            .collect();
        append(&path, &many);
        let answer = read_whole(home.path(), Agent::Claude);
        assert_eq!(live(&answer), []);
        assert_eq!(
            answer.turns.as_ref().unwrap().wake_loss(),
            Some(WakeLoss::Capacity)
        );
        append(
            &path,
            &[
                boot("SessionStart:resume"),
                started("Command running in background with ID: again"),
            ],
        );
        let answer = read_whole(home.path(), Agent::Claude);
        assert_eq!(live(&answer), [None]);
        assert_eq!(answer.turns.as_ref().unwrap().wake_loss(), None);
    }

    /// What Claude Code writes for a `TaskStop` call, and for the result of
    /// one (`is_error` only on a failure), as found in real sessions.
    fn stop_call(call: &str, task: &str) -> serde_json::Value {
        json!({"type": "assistant", "timestamp": "2026-10-03T01:11:00.000Z",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": call,
                "name": "TaskStop", "input": {"task_id": task}}]}})
    }

    fn stopped(call: &str, task: &str) -> serde_json::Value {
        stop_result(
            call,
            &format!(
                "{{\"message\":\"Successfully stopped task: {task} (sleep 600)\",\"task_id\":\"{task}\",\"task_type\":\"local_bash\"}}"
            ),
            false,
        )
    }

    fn stop_refused(call: &str, task: &str) -> serde_json::Value {
        stop_result(
            call,
            &format!(
                "<tool_use_error>Task {task} is not running (status: completed)</tool_use_error>"
            ),
            true,
        )
    }

    fn stop_result(call: &str, text: &str, is_error: bool) -> serde_json::Value {
        let mut block = json!({"type": "tool_result", "tool_use_id": call, "content": text});
        if is_error {
            block["is_error"] = json!(true);
        }
        json!({"type": "user", "timestamp": "2026-10-03T01:11:05.000Z",
            "message": {"role": "user", "content": [block]}})
    }

    /// PRD agent-blocked-state B18: a `TaskStop` call ends a device only when
    /// its own result says it stopped the task, found by `tool_use_id`, or when
    /// the task's notification says it ended (#906). Until a result arrives
    /// the task is still running as far as the records show.
    #[test]
    fn a_task_stop_ends_a_device_only_when_its_result_says_it_stopped() {
        let home = home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        append(
            &path,
            &[
                boot("SessionStart:startup"),
                started("Command running in background with ID: bg1"),
                started("Command running in background with ID: bg2"),
                stop_call("toolu_s1", "bg1"),
            ],
        );
        assert_eq!(
            live(&read_whole(home.path(), Agent::Claude)).len(),
            2,
            "a stop with no result yet has stopped nothing"
        );

        append(&path, &[stop_refused("toolu_s1", "bg1")]);
        assert_eq!(
            live(&read_whole(home.path(), Agent::Claude)).len(),
            2,
            "a stop the tool refused has stopped nothing"
        );

        append(
            &path,
            &[stop_call("toolu_s2", "bg1"), stopped("toolu_s2", "bg1")],
        );
        assert_eq!(
            live(&read_whole(home.path(), Agent::Claude)).len(),
            1,
            "a stop that succeeded ends its task and no other"
        );

        append(&path, &[stopped("toolu_unrelated", "bg2")]);
        assert_eq!(
            live(&read_whole(home.path(), Agent::Claude)).len(),
            1,
            "a result of a call that was no stop of bg2 ends nothing"
        );

        append(&path, &[ended("bg2")]);
        assert_eq!(live(&read_whole(home.path(), Agent::Claude)), []);
    }

    #[test]
    fn a_task_stop_answered_in_a_later_read_ends_the_device_it_named() {
        let home = home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        append(
            &path,
            &[
                boot("SessionStart:startup"),
                started("Command running in background with ID: bg1"),
                stop_call("toolu_s1", "bg1"),
            ],
        );
        let first = read_whole(home.path(), Agent::Claude);
        assert_eq!(live(&first), [None]);

        append(&path, &[stopped("toolu_s1", "bg1")]);
        let mut request = request(Agent::Claude);
        request.checkpoint = Some(first.checkpoint.clone());
        request.turns = first.turns.clone();
        let second = read(home.path(), &request).unwrap();
        assert_eq!(
            live(&second),
            [],
            "the tracker remembers the call it waits on"
        );
    }

    #[test]
    fn a_stop_nobody_answers_before_the_next_process_ends_with_that_process() {
        let home = home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        append(
            &path,
            &[
                boot("SessionStart:startup"),
                started("Command running in background with ID: bg1"),
                stop_call("toolu_s1", "bg1"),
                boot("SessionStart:resume"),
                started("Command running in background with ID: bg1"),
            ],
        );
        // The call belonged to the process before; its result, if one is ever
        // written, does not stop the task the new process started.
        append(&path, &[stopped("toolu_s1", "bg1")]);
        assert_eq!(live(&read_whole(home.path(), Agent::Claude)), [None]);
    }

    /// A call to the tool `name`, and the result of a call, as Claude Code
    /// writes them.
    fn call(name: &str, id: &str) -> serde_json::Value {
        json!({"type": "assistant", "timestamp": "2026-10-03T01:11:00.000Z",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": id,
                "name": name, "input": {"command": "sleep 600"}}]}})
    }

    fn result_of(id: &str, text: &str) -> serde_json::Value {
        stop_result(id, text, false)
    }

    /// A line over `SESSION_LINE_LIMIT_BYTES`, in the shape of `record` with
    /// the string at `pointer` padded.
    fn oversized(mut record: serde_json::Value, pointer: &str) -> serde_json::Value {
        let slot = record.pointer_mut(pointer).unwrap();
        let text = slot.as_str().unwrap().to_owned();
        *slot = json!(format!(
            "{text}{}",
            "x".repeat(hide_session::SESSION_LINE_LIMIT_BYTES)
        ));
        record
    }

    /// PRD agent-blocked-state B18: a record too large to keep is read by its
    /// structure alone, so what it said about a device is known only by which
    /// kind of record it is and which call it names (#904). One that may have
    /// started, ended or stopped a device, or begun a process, proves no
    /// device until the next process starts, as a record the reader cannot
    /// follow does.
    #[test]
    fn a_record_too_large_to_keep_that_may_carry_a_device_proves_none() {
        let ending = oversized(
            json!({"type": "queue-operation", "operation": "enqueue", "timestamp": "2026-10-03T01:20:00.000Z",
                "content": "<task-notification>\n<task-id>bg1</task-id>\n<status>completed</status>\n</task-notification>\n"}),
            "/content",
        );
        // The answer to a call that can start work, whose output is longer
        // than the reader keeps: it may be the start of a task.
        let starting = [
            call("Bash", "toolu_big"),
            oversized(
                result_of(
                    "toolu_big",
                    "Command did not complete within its 600s timeout and was moved to the background (ID: big1). Output:\n",
                ),
                "/message/content/0/content",
            ),
        ];
        let stopping = json!({"type": "assistant", "timestamp": "2026-10-03T01:11:00.000Z",
            "message": {"role": "assistant", "content": [
                {"type": "tool_use", "id": "toolu_w", "name": "Write",
                    "input": {"file_path": "/x", "content": "y".repeat(hide_session::SESSION_LINE_LIMIT_BYTES)}},
                {"type": "tool_use", "id": "toolu_s", "name": "TaskStop", "input": {"task_id": "bg1"}}]}});
        let booting = oversized(
            json!({"type": "attachment", "timestamp": "2026-10-03T01:30:00.000Z",
                "attachment": {"type": "hook_success", "hookName": "SessionStart:resume", "content": "ctx\n"}}),
            "/attachment/content",
        );
        // A result that names no call cannot be paired with one, so nothing
        // says it is not the answer to a command.
        let unpaired = oversized(
            json!({"type": "user", "timestamp": "2026-10-03T01:12:00.000Z",
                "message": {"role": "user", "content": [{"type": "tool_result", "content": "out\n"}]}}),
            "/message/content/0/content",
        );
        // An empty id names no call either, whichever half of the pair holds it.
        let empty_call = oversized(call("Bash", ""), "/message/content/0/input/command");
        let empty_result = oversized(result_of("", "out\n"), "/message/content/0/content");
        let queued = oversized(
            json!({"type": "attachment", "timestamp": "2026-10-03T01:30:00.000Z",
                "attachment": {"type": "queued_command", "prompt": "<task-notification>\n<task-id>bg1</task-id>\n<status>killed</status>\n</task-notification>\n"}}),
            "/attachment/prompt",
        );
        for (records, what) in [
            (vec![ending], "an end in a queued notification"),
            (vec![queued], "an end in a queued command"),
            (
                starting.to_vec(),
                "the output of a call that can start work",
            ),
            (vec![unpaired], "a result that names no call"),
            (vec![empty_call], "a call with an empty id"),
            (vec![empty_result], "a result with an empty id"),
            (vec![stopping], "a TaskStop call"),
            (vec![booting], "a process start"),
        ] {
            let home = home(Agent::Claude);
            let path = session_file(home.path(), Agent::Claude);
            append(
                &path,
                &[
                    boot("SessionStart:startup"),
                    started("Command running in background with ID: bg1"),
                ],
            );
            assert_eq!(live(&read_whole(home.path(), Agent::Claude)), [None]);

            append(&path, &records);
            let answer = read_whole(home.path(), Agent::Claude);
            assert_eq!(live(&answer), [], "{what} the reader could not keep");
            assert_eq!(
                answer.turns.as_ref().unwrap().wake_loss(),
                Some(WakeLoss::Lost),
                "{what} the reader could not keep is a loss, not a quiet session"
            );
        }
    }

    /// Real sessions write a screenshot inline: of 2,545 records over the
    /// limit in 1.55 million lines of one operator's sessions, 2,544 were
    /// user records and 2,407 of them the result of a file read (a picture),
    /// none the result of a command, a monitor or an agent. A record whose
    /// structure says it can carry none of a device's words changes nothing,
    /// whatever its text says.
    #[test]
    fn a_record_too_large_to_keep_that_cannot_carry_a_device_changes_nothing() {
        let image = || {
            json!([{"type": "text", "text": "Command running in background with ID: quoted"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png",
                    "data": "A".repeat(hide_session::SESSION_LINE_LIMIT_BYTES + 1024)}}])
        };
        let screenshot = |id: &str| {
            json!({"type": "user", "timestamp": "2026-10-03T01:12:00.000Z",
                "message": {"role": "user", "content": [{"type": "tool_result", "tool_use_id": id,
                    "content": image()}]}})
        };
        let pasted = json!({"type": "user", "timestamp": "2026-10-03T01:13:00.000Z", "origin": {"kind": "human"},
            "message": {"role": "user", "content": [
                {"type": "text", "text": "look at this"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png",
                    "data": "A".repeat(hide_session::SESSION_LINE_LIMIT_BYTES + 1024)}}]}});
        let long_write = json!({"type": "assistant", "timestamp": "2026-10-03T01:14:00.000Z",
            "message": {"role": "assistant", "content": [{"type": "tool_use", "id": "toolu_w", "name": "Write",
                "input": {"file_path": "/x", "content": "y".repeat(hide_session::SESSION_LINE_LIMIT_BYTES)}}]}});
        let snapshot = json!({"type": "file-history-snapshot", "messageId": "m1",
            "snapshot": {"trackedFileBackups": {"a": "z".repeat(hide_session::SESSION_LINE_LIMIT_BYTES)}}});
        for (records, what) in [
            (
                vec![call("Read", "toolu_shot"), screenshot("toolu_shot")],
                "the picture a file read returned",
            ),
            (
                vec![screenshot("toolu_unseen")],
                "a picture whose call is not in this read",
            ),
            (vec![pasted], "a pasted image"),
            (vec![long_write], "a long Write"),
            (vec![snapshot], "a record of a kind that carries no mark"),
        ] {
            let home = home(Agent::Claude);
            let path = session_file(home.path(), Agent::Claude);
            append(
                &path,
                &[
                    boot("SessionStart:startup"),
                    started("Command running in background with ID: bg1"),
                ],
            );
            append(&path, &records);
            let answer = read_whole(home.path(), Agent::Claude);
            assert_eq!(live(&answer), [None], "{what}");
            assert_eq!(answer.turns.as_ref().unwrap().wake_loss(), None, "{what}");
        }
    }

    /// A call too large to keep still names the result that answers it: a
    /// result that is kept starts its task as any other does, and one that is
    /// not may have.
    #[test]
    fn a_call_too_large_to_keep_is_still_paired_with_its_result() {
        let long_command = || {
            oversized(
                call("Bash", "toolu_long"),
                "/message/content/0/input/command",
            )
        };
        let home = home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        append(
            &path,
            &[
                boot("SessionStart:startup"),
                started("Command running in background with ID: bg1"),
                long_command(),
                result_of(
                    "toolu_long",
                    "Command running in background with ID: bg2. Output is being written to: /x",
                ),
            ],
        );
        let answer = read_whole(home.path(), Agent::Claude);
        assert_eq!(live(&answer).len(), 2);
        assert_eq!(answer.turns.as_ref().unwrap().wake_loss(), None);

        let home = super::home(Agent::Claude);
        let path = session_file(home.path(), Agent::Claude);
        append(
            &path,
            &[
                boot("SessionStart:startup"),
                started("Command running in background with ID: bg1"),
                long_command(),
                oversized(
                    result_of("toolu_long", "out\n"),
                    "/message/content/0/content",
                ),
            ],
        );
        let answer = read_whole(home.path(), Agent::Claude);
        assert_eq!(live(&answer), []);
        assert_eq!(
            answer.turns.as_ref().unwrap().wake_loss(),
            Some(WakeLoss::Lost)
        );
    }

    /// The survey in `agents/runs/agent-blocked-state/wake-device-survey.md`
    /// read real Codex, Grok, Pi, omp, Cursor and OpenCode sessions: none
    /// writes a record that proves a started task still runs or that a new
    /// process began. Each declares no device, and the records that look
    /// closest to one (and Claude's own) prove nothing when read as theirs.
    #[test]
    fn no_other_agent_reports_a_device() {
        for agent in [
            Agent::Codex,
            Agent::Grok,
            Agent::Pi,
            Agent::Omp,
            Agent::Cursor,
            Agent::OpenCode,
        ] {
            assert!(!agent.reports_wake_devices(), "{agent:?}");
        }
        assert!(Agent::Claude.reports_wake_devices());
        let records = [
            boot("SessionStart:startup"),
            started("Command running in background with ID: bg1"),
            ended("bg1"),
            // Codex: a running exec session answers with its session id and
            // writes nothing when it ends.
            json!({"type": "response_item", "timestamp": "2026-10-03T01:10:00.000Z",
                "payload": {"type": "function_call_output", "call_id": "c1", "output": "Process running with session ID 1234"}}),
            // Grok: a command moved to the background is `running` in the
            // record and its end does not create a turn or a record of its own.
            json!({"type": "tool_result", "tool_call_id": "call-1", "content": "<task-id>call-1</task-id>\n<task-type>bash</task-type>\n<status>running</status>\n<summary>Command has been automatically moved to background</summary>"}),
        ];
        let contents: String = records.iter().map(|record| format!("{record}\n")).collect();
        for agent in [
            Agent::Codex,
            Agent::Grok,
            Agent::Pi,
            Agent::Omp,
            Agent::Cursor,
            Agent::OpenCode,
        ] {
            let parsed = hide_session::parse_events(agent, &contents);
            assert!(
                !parsed
                    .turn_marks
                    .iter()
                    .any(|(_, mark)| matches!(mark, hide_session::turns::TurnMark::Wake(_))),
                "{agent:?} proved a device from a record that proves none"
            );
        }
        let home = home(Agent::Codex);
        append(&session_file(home.path(), Agent::Codex), &records);
        assert_eq!(
            live(&read_whole(home.path(), Agent::Codex)),
            [],
            "Codex records prove no running task"
        );
    }
}

#[test]
fn every_agent_reads_to_the_same_facts() {
    for agent in [Agent::Claude, Agent::Codex, Agent::OpenCode] {
        let home = home(agent);
        assert_same_facts(&format!("{agent:?}"), &read_whole(home.path(), agent));
    }
}

/// The facts every fixture's conversation reads to, whatever its format.
pub(crate) fn assert_same_facts(agent: &str, transcript: &LabelTranscript) {
    assert_eq!(
        transcript.title.as_deref(),
        Some("요청 보기 만들기"),
        "{agent} title"
    );

    let people: Vec<_> = transcript
        .events
        .iter()
        .filter(|event| event.kind == LabelEventKind::Human)
        .collect();
    assert_eq!(
        people.len(),
        3,
        "{agent}: injected text is no one's request"
    );
    assert_eq!(people[0].text, "요청 보기를 만들어줘\n긴 요청의 둘째 줄");
    assert_eq!(people[0].at_unix_ms, START, "{agent}");
    assert_eq!(people[0].sender, None);
    assert_eq!(people[1].sender.as_deref(), Some("ci-lead"), "{agent}");
    assert_eq!(
        people[1].text.trim(),
        "Hide letter letter-1 from ci-lead (claude) [request]\nCI 다시 봐줘\nFull letter: hide request show letter-1",
        "{agent}: the current delivery header and body stay readable"
    );
    assert_eq!(
        (people[2].text.trim(), people[2].images),
        ("", 1),
        "{agent}: an image sent alone, without what the agent wrapped it in"
    );

    let reply = transcript
        .events
        .iter()
        .rev()
        .find(|event| event.kind == LabelEventKind::Assistant)
        .unwrap();
    assert!(
        reply
            .text
            .starts_with("PR을 열었습니다: https://github.com/acme/app/pull/12"),
        "{agent}"
    );

    let created = sighted(transcript, 12).unwrap_or_else(|| panic!("{agent} sighting"));
    assert_eq!(
        created.at_unix_ms,
        START + 30_000,
        "{agent}: when the tool printed it"
    );
    assert!(
        sighted(transcript, 99).is_none(),
        "{agent}: a reply's mention is not a tool's output"
    );
}

/// A conversation page reaches the core as JSON over the node link
/// (`Call::SessionConversation`), and the core reads every event back. Codex
/// writes the instructions it adds to a turn as `developer` records, and a
/// page holding one was refused as `conversation_role_invalid`, so Project
/// Memory failed every Codex session again each time its file grew.
#[test]
fn every_agents_conversation_page_reads_back_from_the_node_link() {
    let file = |agent: Agent, home: &Path| {
        match agent {
        Agent::Claude => home.join(".claude/projects/-work-app/a1b2c3d4-0000-4000-8000-000000000001.jsonl"),
        Agent::Codex => home.join(".codex/sessions/2026/10/03/rollout-2026-10-03T01-00-00-0199a000-0000-7000-8000-000000000002.jsonl"),
        _ => unreachable!("the file fixtures above"),
    }
    };
    for agent in [Agent::Claude, Agent::Codex] {
        let home = home(agent);
        let scope = hide_session::SessionReadScope {
            id: request(agent).reference_value,
            cwd: "/work/app".to_owned(),
        };
        let page = hide_session::read_conversation(
            home.path(),
            agent,
            &file(agent, home.path()),
            &scope,
            None,
        )
        .unwrap();
        let wire = serde_json::to_vec(&page).unwrap();
        let read: hide_session::ConversationRead = serde_json::from_slice(&wire)
            .unwrap_or_else(|error| panic!("{agent:?}: the core refused the page: {error}"));
        assert_eq!(read.events, page.events, "{agent:?}");
    }
}

mod omp {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::io::{Seek, SeekFrom, Write};

    struct Native {
        home: tempfile::TempDir,
        path: PathBuf,
        request: LabelTranscriptRequest,
    }

    impl Native {
        fn new() -> Self {
            Self::with_checkout("checkout")
        }

        fn with_checkout(name: &str) -> Self {
            let home = tempfile::tempdir().unwrap();
            let cwd = home.path().join(name);
            fs::create_dir(&cwd).unwrap();
            let cwd = cwd.canonicalize().unwrap();
            // Upstream session-paths checks the system temporary root first,
            // even though this fixture checkout is also inside its home.
            let temp = std::env::temp_dir().canonicalize().unwrap();
            let relative = cwd.strip_prefix(temp).unwrap().to_string_lossy();
            let bucket = format!("-tmp-{}", relative.replace(['/', '\\', ':'], "-"));
            let path = home
                .path()
                .join(".omp/agent/sessions")
                .join(bucket)
                .join("date_omp-native-a.jsonl");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let fixture = fs::read_to_string(fixtures().join("omp-18.7.0/session.jsonl")).unwrap();
            let mut records: Vec<serde_json::Value> = fixture
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect();
            records[1]["cwd"] = json!(cwd);
            records[1]["parentSession"] = json!("/outside/opaque-fork-parent.jsonl");
            fs::write(
                &path,
                records.iter().map(|v| format!("{v}\n")).collect::<String>(),
            )
            .unwrap();
            let mut request = request(Agent::Omp);
            request.cwd = Some(cwd.display().to_string());
            Self {
                home,
                path,
                request,
            }
        }

        fn read(&self) -> Result<LabelTranscript, hide_session::ReadRefusal> {
            read(self.home.path(), &self.request)
        }
        fn resume(&mut self, answer: &LabelTranscript) {
            self.request.checkpoint = Some(answer.checkpoint.clone());
            self.request.turns = answer.turns.clone();
        }
        fn append(&self, record: serde_json::Value) {
            writeln!(
                fs::OpenOptions::new()
                    .append(true)
                    .open(&self.path)
                    .unwrap(),
                "{record}"
            )
            .unwrap();
        }
        fn title(&self, text: &str, source: &str) {
            let mut slot = json!({"type":"title","v":1,"title":text,"source":source,"updatedAt":"2026-10-03T02:00:00Z","pad":""});
            slot["pad"] = json!(" ".repeat(256 - slot.to_string().len() - 1));
            let line = format!("{slot}\n");
            assert_eq!(line.len(), 256);
            let mut file = fs::OpenOptions::new().write(true).open(&self.path).unwrap();
            file.seek(SeekFrom::Start(0)).unwrap();
            file.write_all(line.as_bytes()).unwrap();
        }
    }

    #[test]
    fn physical_title_changes_at_eof_clear_stale_names_without_replaying_events() {
        let mut native = Native::new();
        let first = native.read().unwrap();
        assert_eq!(first.custom_title.as_deref(), Some("요청 보기 만들기"));
        assert_eq!(first.title.as_deref(), Some(""));
        assert_eq!(first.events.len(), 4);
        assert_eq!(
            first.events[0].text,
            "요청 보기를 만들어줘\n긴 요청의 둘째 줄"
        );
        assert_eq!(first.events[0].at_unix_ms, START);
        assert_eq!(sighted(&first, 12).unwrap().at_unix_ms, START + 30_000);
        assert!(sighted(&first, 99).is_none());
        assert!(first.subagents.is_empty());
        let offset = first.checkpoint.offset();
        let bytes = fs::metadata(&native.path).unwrap().len();
        native.resume(&first);
        for (name, source, automatic, manual) in
            [("새 제목", "auto", "새 제목", ""), ("", "user", "", "")]
        {
            native.title(name, source);
            let next = native.read().unwrap();
            assert!(next.events.is_empty());
            assert_eq!(next.checkpoint.offset(), offset);
            assert_eq!(next.title.as_deref(), Some(automatic));
            assert_eq!(next.custom_title.as_deref(), Some(manual));
            assert_eq!(fs::metadata(&native.path).unwrap().len(), bytes);
            native.resume(&next);
        }
    }

    #[test]
    fn native_ask_waits_with_choices_and_only_correlated_answers_clear_it() {
        let mut native = Native::new();
        let first = native.read().unwrap();
        native.resume(&first);
        native.append(json!({"type":"message","timestamp":"2026-10-03T02:00:01Z","message":{"role":"assistant","content":[{"type":"toolCall","name":"ask","id":"q-1","arguments":{"questions":[{"id":"policy","question":"어느 쪽으로 할까요?","header":"선택","options":[{"label":"계속","description":"계속 진행"},{"label":"중단"}],"multi":false,"recommended":0}]}}]}}));
        let asked = native.read().unwrap();
        let fact = asked.turns.as_ref().unwrap().user_turn().unwrap();
        assert_eq!(fact.kind, hide_session::turns::UserTurnKind::Question);
        let content = fact.content.unwrap();
        assert_eq!(content.text(), "어느 쪽으로 할까요?");
        assert_eq!(content.choices(), ["계속", "중단"]);
        assert!(asked.events.is_empty());
        native.resume(&asked);
        for (call, waiting) in [("unrelated", Waiting::Question), ("q-1", Waiting::Nothing)] {
            native.append(json!({"type":"message","timestamp":"2026-10-03T02:00:02Z","message":{"role":"toolResult","toolCallId":call,"toolName":"ask","isError":true,"content":[{"type":"text","text":"aborted"}]}}));
            native.title("同時の名前", "auto");
            let answered = native.read().unwrap();
            assert_eq!(answered.turns.as_ref().unwrap().waiting(), Some(waiting));
            assert!(answered.events.is_empty());
            assert_eq!(answered.title.as_deref(), Some("同時の名前"));
            native.resume(&answered);
        }
    }

    /// Herdr reports an omp session as its file. The proof that names the
    /// pane's session from that file and the link reader name the same
    /// session, by the id in the header, so the core can key the link
    /// record's lineage by either.
    #[test]
    fn the_file_herdr_reports_and_the_link_reader_name_one_session() {
        let mut native = Native::new();
        native.request.reference_kind = "path".to_owned();
        native.request.reference_value = native.path.display().to_string();
        let pane = native.read().unwrap();
        let linked = hide_session::links::read(
            native.home.path(),
            &[hide_session::links::ReadRequest {
                agent: Agent::Omp,
                path: native.path.display().to_string(),
                checkpoint: None,
            }],
        )
        .remove(0);
        assert_eq!(linked.error, None);
        assert_eq!(
            pane.confirmed.native_session_id.as_deref(),
            Some("omp-native-a")
        );
        assert_eq!(linked.facts.session_id.as_deref(), Some("omp-native-a"));
    }

    #[test]
    fn legacy_current_header_wins_over_audit_and_nested_children_never_enter_root_catalog() {
        let native = Native::new();
        let body = fs::read_to_string(&native.path).unwrap();
        fs::write(&native.path, body.split_once('\n').unwrap().1).unwrap();
        let legacy = native.read().unwrap();
        assert_eq!(legacy.title.as_deref(), Some("stale header"));
        assert_eq!(legacy.custom_title.as_deref(), Some(""));
        let artifacts = native.path.with_extension("").join("subagents");
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(
            artifacts.join("child.jsonl"),
            fs::read_to_string(&native.path)
                .unwrap()
                .replace("omp-native-a", "child-native"),
        )
        .unwrap();
        let project =
            hide_project::resolve(Path::new(native.request.cwd.as_deref().unwrap()), "local")
                .unwrap();
        let catalog = hide_session::SessionCatalog::new(native.home.path(), "local")
            .project_sessions(&project)
            .unwrap()
            .sessions;
        assert_eq!(catalog.len(), 1);
        assert_eq!(catalog[0].agent, Agent::Omp);
        assert_eq!(catalog[0].id, "omp-native-a");
        assert_eq!(catalog[0].title.as_deref(), Some("stale header"));
        let linked = hide_session::links::read(
            native.home.path(),
            &[hide_session::links::ReadRequest {
                agent: Agent::Omp,
                path: native.path.display().to_string(),
                checkpoint: None,
            }],
        )
        .remove(0);
        assert!(linked.error.is_none());
        assert_eq!(linked.facts.session_id.as_deref(), Some("omp-native-a"));
        assert_eq!(linked.facts.prs.len(), 1);
        assert_eq!(linked.facts.prs[0].number, 12);
        assert!(native.read().unwrap().subagents.is_empty());
    }

    #[test]
    fn exact_identity_and_default_route_refuse_filename_aliases_before_effects() {
        let mut native = Native::new();
        let by_id = native.read().unwrap();
        native.request.reference_kind = "path".into();
        native.request.reference_value = native.path.display().to_string();
        let by_path = native.read().unwrap();
        assert_eq!(by_id.confirmed.owner, by_path.confirmed.owner);
        let action = hide_session::session_activity::SessionActivityRequest {
            agent: Agent::Omp,
            reference_kind: "path".into(),
            reference_value: native.path.display().to_string(),
            cwd: native.request.cwd.clone(),
            exact_route: true,
            expected_id: Some("omp-native-a".into()),
        };
        assert!(hide_session::session_activity::read(native.home.path(), &action).is_ok());
        let sibling = native
            .path
            .parent()
            .unwrap()
            .join("OMP-NATIVE-A-alias.jsonl");
        let other = fs::read_to_string(&native.path)
            .unwrap()
            .replace("omp-native-a", "other-owner");
        fs::write(&sibling, other).unwrap();
        assert_eq!(
            hide_session::session_activity::read(native.home.path(), &action).unwrap_err(),
            hide_session::ReadRefusal::Refused("session_route_ambiguous".into())
        );
        assert!(native.read().is_ok(), "ambiguity is action-local");
        fs::remove_file(sibling).unwrap();
        assert!(hide_session::session_activity::read(native.home.path(), &action).is_ok());
        native.request.cwd = Some(native.home.path().display().to_string());
        assert!(native.read().is_err());
    }

    #[test]
    fn option_like_native_ids_never_authorize_a_resume_or_fork() {
        let mut native = Native::new();
        native.request.reference_kind = "path".into();
        native.request.reference_value = native.path.display().to_string();
        let source = fs::read_to_string(&native.path).unwrap();
        for id in ["-", "--", "--no-session", "--yolo", "-r"] {
            // omp's optional-value --resume parser treats these as flags,
            // not its selector (args.ts, pinned 18.7.0); --fork requires a value.
            fs::write(&native.path, source.replace("omp-native-a", id)).unwrap();
            let action = hide_session::session_activity::SessionActivityRequest {
                agent: Agent::Omp,
                reference_kind: "path".into(),
                reference_value: native.path.display().to_string(),
                cwd: native.request.cwd.clone(),
                exact_route: true,
                expected_id: Some(id.into()),
            };
            assert!(
                hide_session::session_activity::read(native.home.path(), &action).is_err(),
                "a native flag cannot become a session selector: {id}"
            );
        }
        fs::write(&native.path, source).unwrap();
        assert!(native.read().is_ok());
    }

    #[test]
    fn native_preprocessing_inputs_refuse_effects_without_mutating_the_selected_source() {
        use sha2::{Digest, Sha256};
        // Pinned native regex replaces each invalid run separately from an
        // adjacent literal '-'; the expected readable names are native facts.
        for (checkout, readable) in [
            ("repo-한글x", "repo--x"),
            ("checkout", "checkout"),
            ("repo한글-x", "repo--x"),
            ("한글", "project"),
        ] {
            let native = Native::with_checkout(checkout);
            let source = fs::read(&native.path).unwrap();
            let alias_source = String::from_utf8(source.clone())
                .unwrap()
                .replace("omp-native-a", "different-native-owner");
            let cwd = Path::new(native.request.cwd.as_deref().unwrap());
            let root = native.home.path().join(".omp/agent/sessions");
            let action = hide_session::session_activity::SessionActivityRequest {
                agent: Agent::Omp,
                reference_kind: "path".into(),
                reference_value: native.path.display().to_string(),
                cwd: native.request.cwd.clone(),
                exact_route: true,
                expected_id: Some("omp-native-a".into()),
            };
            let check = || {
                hide_session::session_activity::read(native.home.path(), &action)
                    .map_err(String::from)
            };
            assert!(check().is_ok());
            // These names come from upstream session-paths/session-listing, not
            // Hide's policy helpers: native startup moves/promotes their aliases.
            let absolute = format!(
                "--{}--",
                cwd.to_str()
                    .unwrap()
                    .trim_start_matches(['/', '\\'])
                    .replace(['/', '\\', ':'], "-")
            );
            let hashed = format!(
                "tmp-{readable}-{:x}",
                Sha256::digest(cwd.to_str().unwrap().replace('\\', "/").as_bytes())
            );
            let old_home = format!(
                "--{}-{}--",
                native
                    .home
                    .path()
                    .to_str()
                    .unwrap()
                    .trim_start_matches(['/', '\\'])
                    .replace(['/', '\\', ':'], "-"),
                native
                    .path
                    .parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .strip_prefix('-')
                    .unwrap()
            );
            // Root-wide home migration runs before cwd legacy migration. This
            // entry first creates the absent absolute alias, then merges into
            // the selected default bucket during the same native startup.
            let old_absolute = format!(
                "--{}-{}--",
                native
                    .home
                    .path()
                    .to_str()
                    .unwrap()
                    .trim_start_matches(['/', '\\'])
                    .replace(['/', '\\', ':'], "-"),
                absolute.strip_prefix('-').unwrap()
            );
            for bucket in [
                absolute,
                format!("-{checkout}"),
                hashed,
                old_home,
                old_absolute,
            ] {
                let migrated = root.join(&bucket);
                fs::create_dir(&migrated).unwrap();
                let alias = migrated.join("date_omp-native-a-newer.jsonl");
                fs::write(&alias, &alias_source).unwrap();
                assert_eq!(
                    check().map(|_| ()),
                    Err("session_route_requires_native_migration".to_owned()),
                    "{checkout}: {bucket}"
                );
                assert!(native.read().is_ok(), "migration refusal is action-only");
                assert_eq!(fs::read(&native.path).unwrap(), source);
                assert!(alias.exists(), "Hide must not perform the native migration");
                fs::remove_dir_all(migrated).unwrap();
                assert!(check().is_ok(), "a resolved migration restores the route");
            }
            let backup = native
                .path
                .with_file_name("date_omp-native-a-newer.jsonl.123.bak");
            fs::write(&backup, &alias_source).unwrap();
            assert_eq!(
                check().unwrap_err(),
                "session_route_requires_native_recovery"
            );
            assert!(backup.exists(), "Hide must not promote the native backup");
            assert!(native.read().is_ok());
            assert_eq!(fs::read(&native.path).unwrap(), source);
            fs::remove_file(backup).unwrap();
            assert!(check().is_ok());
        }
    }

    #[test]
    fn native_unicode_filename_aliases_never_pass_an_ascii_route_audit() {
        let native = Native::new();
        let source = fs::read_to_string(&native.path)
            .unwrap()
            .replace("omp-native-a", "knative");
        fs::write(&native.path, &source).unwrap();
        let action = hide_session::session_activity::SessionActivityRequest {
            agent: Agent::Omp,
            reference_kind: "path".into(),
            reference_value: native.path.display().to_string(),
            cwd: native.request.cwd.clone(),
            exact_route: true,
            expected_id: Some("knative".into()),
        };
        let check = || hide_session::session_activity::read(native.home.path(), &action);
        assert!(check().is_ok());
        for name in ["Knative.jsonl", "date_Knative.jsonl"] {
            // ECMAScript lowercases Kelvin sign to ASCII k. Native OMP
            // matches both the entire filename and the final '_' suffix.
            let alias = native.path.with_file_name(name);
            fs::write(&alias, source.replace("knative", "other-owner")).unwrap();
            assert!(check().is_err(), "native Unicode alias must refuse: {name}");
            fs::remove_file(alias).unwrap();
            assert!(check().is_ok());
        }
    }

    #[test]
    fn a_dotdot_named_temp_child_uses_the_natives_absolute_bucket() {
        let mut native = Native::new();
        let checkout = tempfile::Builder::new()
            .prefix("..omp-route-")
            .tempdir()
            .unwrap();
        let cwd = checkout.path().canonicalize().unwrap();
        let temp = std::env::temp_dir().canonicalize().unwrap();
        let relative = cwd.strip_prefix(temp).unwrap().to_str().unwrap();
        assert!(relative.starts_with(".."));
        // Upstream isRelativeWithin rejects even this ordinary child name.
        let root = native.home.path().join(".omp/agent/sessions");
        let wrong = root
            .join(format!("-tmp-{relative}"))
            .join("date_omp-native-a.jsonl");
        let correct = root
            .join(format!(
                "--{}--",
                cwd.to_str()
                    .unwrap()
                    .trim_start_matches(['/', '\\'])
                    .replace(['/', '\\', ':'], "-")
            ))
            .join("date_omp-native-a.jsonl");
        let source = fs::read_to_string(&native.path).unwrap().replace(
            native.request.cwd.as_deref().unwrap(),
            cwd.to_str().unwrap(),
        );
        for path in [&wrong, &correct] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, &source).unwrap();
        }
        native.request.cwd = Some(cwd.display().to_string());
        native.request.reference_kind = "path".into();
        native.request.reference_value = wrong.display().to_string();
        assert!(
            native.read().is_err(),
            "the relative bucket is not a native route"
        );
        native.request.reference_value = correct.display().to_string();
        assert!(native.read().is_ok());
        let action = hide_session::session_activity::SessionActivityRequest {
            agent: Agent::Omp,
            reference_kind: "path".into(),
            reference_value: correct.display().to_string(),
            cwd: native.request.cwd.clone(),
            exact_route: true,
            expected_id: Some("omp-native-a".into()),
        };
        assert!(hide_session::session_activity::read(native.home.path(), &action).is_ok());
    }
}

const GROK_ID: &str = "0199b000-0000-7000-8000-000000000003";

mod grok {
    use super::*;
    use hide_session::turns::UserTurnKind;
    use hide_session::{SessionCatalog, SessionIdentity, SessionLocator};
    use serde_json::json;
    use std::fs;
    use std::io::Write;

    struct Native {
        home: tempfile::TempDir,
        cwd: PathBuf,
        path: PathBuf,
        request: LabelTranscriptRequest,
    }

    /// The native group name: every byte but `A-Za-z0-9-_.~` as `%XX`.
    fn group(cwd: &Path) -> String {
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

    impl Native {
        fn new() -> Self {
            let home = tempfile::tempdir().unwrap();
            let cwd = home.path().join("checkout 1");
            fs::create_dir(&cwd).unwrap();
            let cwd = fs::canonicalize(cwd).unwrap();
            let folder = home
                .path()
                .join(".grok/sessions")
                .join(group(&cwd))
                .join(GROK_ID);
            fs::create_dir_all(&folder).unwrap();
            let fixture = fixtures().join("grok-1.0.46");
            fs::copy(fixture.join("updates.jsonl"), folder.join("updates.jsonl")).unwrap();
            let native = Self {
                path: folder.join("updates.jsonl"),
                request: LabelTranscriptRequest {
                    cwd: Some(cwd.display().to_string()),
                    ..request(Agent::Grok)
                },
                home,
                cwd,
            };
            native.summary(|_| {});
            native
        }

        fn folder(&self) -> &Path {
            self.path.parent().unwrap()
        }

        /// Grok rewrites `summary.json` whole and renames it into place.
        fn summary(&self, change: impl FnOnce(&mut serde_json::Value)) {
            let mut summary: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(fixtures().join("grok-1.0.46/summary.json")).unwrap(),
            )
            .unwrap();
            summary["info"]["cwd"] = json!(self.cwd);
            change(&mut summary);
            let temporary = self.folder().join("summary.json.tmp");
            fs::write(&temporary, serde_json::to_vec_pretty(&summary).unwrap()).unwrap();
            fs::rename(temporary, self.folder().join("summary.json")).unwrap();
        }

        fn read(&self) -> Result<LabelTranscript, hide_session::ReadRefusal> {
            read(self.home.path(), &self.request)
        }

        /// The label store keeps a checkpoint as JSON, so each read resumes
        /// from one that went through it, even inside an oversized record.
        fn resume(&mut self, answer: &LabelTranscript) {
            let stored = serde_json::to_string(&answer.checkpoint).unwrap();
            self.request.checkpoint = Some(serde_json::from_str(&stored).unwrap());
            self.request.turns = answer.turns.clone();
        }

        fn append(&self, update: serde_json::Value, meta: serde_json::Value) {
            let method = if update["sessionUpdate"] == "turn_completed" {
                "_x.ai/session/update"
            } else {
                "session/update"
            };
            let mut params_meta = json!({"eventId":"e","agentTimestampMs": START + 200_000});
            params_meta
                .as_object_mut()
                .unwrap()
                .extend(meta.as_object().unwrap().clone());
            let record = json!({"timestamp": (START + 200_000) / 1000, "method": method,
                "params": {"sessionId": GROK_ID, "update": update, "_meta": params_meta}});
            writeln!(
                fs::OpenOptions::new()
                    .append(true)
                    .open(&self.path)
                    .unwrap(),
                "{record}"
            )
            .unwrap();
        }

        fn route(&self, cwd: &Path) -> Result<(), String> {
            self.route_by("id", GROK_ID, cwd, None)
        }

        fn route_by(
            &self,
            kind: &str,
            value: &str,
            cwd: &Path,
            expected_id: Option<&str>,
        ) -> Result<(), String> {
            hide_session::session_activity::read(
                self.home.path(),
                &hide_session::session_activity::SessionActivityRequest {
                    agent: Agent::Grok,
                    reference_kind: kind.into(),
                    reference_value: value.into(),
                    cwd: cwd.to_str().map(str::to_owned),
                    exact_route: true,
                    expected_id: expected_id.map(str::to_owned),
                },
            )
            .map(|_| ())
            .map_err(String::from)
        }
    }

    fn user_turn(answer: &LabelTranscript) -> Option<hide_session::turns::UserTurnFact> {
        answer.turns.as_ref().unwrap().user_turn()
    }

    #[test]
    fn native_history_has_the_shared_conversation_facts_with_split_records_joined() {
        let native = Native::new();
        let transcript = native.read().unwrap();
        assert_eq!(transcript.title.as_deref(), Some("요청 보기 만들기"));
        assert_eq!(transcript.custom_title.as_deref(), Some(""));
        let people: Vec<_> = transcript
            .events
            .iter()
            .filter(|event| event.kind == LabelEventKind::Human)
            .collect();
        assert_eq!(people.len(), 3, "Grok's own wake is no one's request");
        assert_eq!(people[0].text, "요청 보기를 만들어줘\n긴 요청의 둘째 줄");
        assert_eq!(people[0].at_unix_ms, START);
        assert_eq!(people[1].sender.as_deref(), Some("ci-lead"));
        assert_eq!((people[2].text.as_str(), people[2].images), ("", 1));
        let replies: Vec<_> = transcript
            .events
            .iter()
            .filter(|event| event.kind == LabelEventKind::Assistant)
            .collect();
        assert_eq!(replies.len(), 1, "a turn's text runs are one answer");
        assert_eq!(
            replies[0].text,
            "PR을 열었습니다: https://github.com/acme/app/pull/12\n\n예전 것은 https://github.com/acme/app/pull/99 입니다"
        );
        assert_eq!(replies[0].at_unix_ms, START + 129_000);
        assert_eq!(sighted(&transcript, 12).unwrap().at_unix_ms, START + 30_000);
        assert!(sighted(&transcript, 99).is_none());
        assert_eq!(
            transcript.confirmed.native_session_id.as_deref(),
            Some(GROK_ID)
        );
        assert_eq!(user_turn(&transcript), None);
        assert_eq!(
            transcript.turns.as_ref().unwrap().waiting(),
            Some(Waiting::Nothing)
        );
        assert!(transcript.subagents.is_empty());
    }

    #[test]
    fn the_blocks_of_one_prompt_appended_after_a_read_are_one_message() {
        let mut native = Native::new();
        let first = native.read().unwrap();
        native.resume(&first);
        let block = |content: serde_json::Value| {
            json!({"sessionUpdate":"user_message_chunk","content":content,
                "_meta":{"modelId":"grok-build","promptIndex":4}})
        };
        native.append(
            block(json!({"type":"text","text":"이 화면을 봐줘"})),
            json!({}),
        );
        native.append(
            block(json!({"type":"image","data":"iVBORw0KGgo=","mimeType":"image/png"})),
            json!({}),
        );
        let next = native.read().unwrap();
        assert_eq!(next.events.len(), 1);
        assert_eq!(
            (next.events[0].text.as_str(), next.events[0].images),
            ("이 화면을 봐줘", 1)
        );
        let mut whole =
            hide_session::parse_events(Agent::Grok, &fs::read_to_string(&native.path).unwrap());
        assert_eq!(whole.events.len(), 8, "one event per native record");
        whole.coalesce();
        assert_eq!(
            whole
                .events
                .iter()
                .filter(|event| event.kind == hide_session::EventKind::Human)
                .count(),
            4
        );
    }

    #[test]
    fn an_unanswered_question_waits_with_its_choices_until_its_tool_call_ends() {
        let mut native = Native::new();
        let first = native.read().unwrap();
        native.resume(&first);
        native.append(
            json!({"sessionUpdate":"user_message_chunk","content":{"type":"text","text":"배포해줘"},
                "_meta":{"promptIndex":4}}),
            json!({}),
        );
        let input = json!({"questions":[{"question":"배포 대상을 골라주세요",
            "options":[{"label":"미리보기","description":"preview"},{"label":"운영"}],"multi_select":false}]});
        native.append(
            json!({"sessionUpdate":"tool_call","toolCallId":"call-ask","title":"ask_user_question",
                "rawInput":input,"_meta":{"x.ai/tool":{"version":1,"name":"ask_user_question","kind":"ask_user"}}}),
            json!({"promptId":"p-4"}),
        );
        native.append(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"call-ask","title":"Ask: 배포 대상을 골라주세요",
                "kind":"other","rawInput":input,"_meta":{"x.ai/tool":{"version":1,"name":"ask_user_question","kind":"ask_user"}}}),
            json!({"promptId":"p-4"}),
        );
        let asked = native.read().unwrap();
        let fact = user_turn(&asked).unwrap();
        assert_eq!(fact.kind, UserTurnKind::Question);
        let content = fact.content.unwrap();
        assert_eq!(content.text(), "배포 대상을 골라주세요");
        assert_eq!(content.choices(), ["미리보기", "운영"]);
        native.resume(&asked);
        native.append(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"call-ask","status":"completed",
                "content":[{"type":"content","content":{"type":"text","text":"미리보기"}}]}),
            json!({"promptId":"p-4"}),
        );
        let answered = native.read().unwrap();
        assert_eq!(user_turn(&answered), None);
        assert!(answered.events.is_empty(), "an answer is no message");

        native.resume(&answered);
        native.append(
            json!({"sessionUpdate":"tool_call","toolCallId":"call-ask-2","title":"ask_user_question",
                "rawInput":input,"_meta":{"x.ai/tool":{"kind":"ask_user"}}}),
            json!({"promptId":"p-4"}),
        );
        let again = native.read().unwrap();
        assert_eq!(user_turn(&again).unwrap().kind, UserTurnKind::Question);
        native.resume(&again);
        native.append(
            json!({"sessionUpdate":"turn_completed","prompt_id":"p-4","stop_reason":"cancelled"}),
            json!({}),
        );
        let cancelled = native.read().unwrap();
        assert_eq!(user_turn(&cancelled), None, "a cancelled turn asks nothing");
        assert_eq!(cancelled.events[0].kind, LabelEventKind::Interrupted);
    }

    /// Grok writes a pasted image inline and a tool's output twice, so a
    /// record over the line cap is ordinary: it is read without its body,
    /// across as many polls as it takes, and a question still waits.
    #[test]
    fn a_record_over_the_line_cap_keeps_its_turn_without_its_body() {
        let mut native = Native::new();
        let first = native.read().unwrap();
        native.resume(&first);
        let block = |index: u64, content: serde_json::Value| {
            json!({"sessionUpdate":"user_message_chunk","content":content,
                "_meta":{"modelId":"grok-build","promptIndex":index}})
        };
        native.append(
            block(4, json!({"type":"text","text":"이 화면을 봐줘"})),
            json!({}),
        );
        native.append(
            block(
                4,
                json!({"type":"image","data":"A".repeat(1536 * 1024),"mimeType":"image/png"}),
            ),
            json!({}),
        );
        native.append(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"call-read","status":"completed",
                "content":[{"type":"content","content":{"type":"text",
                    "text":format!("{} https://github.com/acme/app/pull/77", "x".repeat(300 * 1024))}}]}),
            json!({"promptId":"p-4"}),
        );
        native.append(
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"다 봤습니다"}}),
            json!({"promptId":"p-4"}),
        );
        native.append(
            json!({"sessionUpdate":"turn_completed","prompt_id":"p-4","stop_reason":"end_turn"}),
            json!({}),
        );
        let mut events = Vec::new();
        let mut reasons = std::collections::BTreeMap::new();
        let mut polls = 0;
        let last = loop {
            let next = native.read().unwrap();
            polls += 1;
            events.extend(
                next.events
                    .iter()
                    .map(|event| (event.kind, event.text.clone(), event.images)),
            );
            for (reason, count) in &next.skipped_reasons {
                *reasons.entry(reason.clone()).or_insert(0) += count;
            }
            native.resume(&next);
            if !next.has_more {
                break next;
            }
        };
        assert!(polls > 1, "the image spans more than one poll");
        // Blocks are joined within one poll; a poll boundary between them
        // leaves two events, which the phone joins again when it pages.
        assert_eq!(
            events,
            [
                (LabelEventKind::Human, "이 화면을 봐줘".to_owned(), 0),
                (LabelEventKind::Human, String::new(), 1),
                (LabelEventKind::Assistant, "다 봤습니다".to_owned(), 0),
            ]
        );
        assert_eq!(reasons.get("body_capacity"), Some(&2));
        assert!(sighted(&last, 77).is_none(), "a dropped body is never read");
        assert_eq!(
            last.turns.as_ref().unwrap().waiting(),
            Some(Waiting::Nothing)
        );

        // A prompt whose own text is over the cap still starts its turn,
        // which answers the question left before it.
        native.append(
            json!({"sessionUpdate":"tool_call","toolCallId":"call-ask","title":"ask_user_question",
                "rawInput":{"questions":[{"question":"계속할까요?","options":[]}]},
                "_meta":{"x.ai/tool":{"kind":"ask_user"}}}),
            json!({"promptId":"p-4"}),
        );
        let asked = native.read().unwrap();
        assert_eq!(user_turn(&asked).unwrap().kind, UserTurnKind::Question);
        native.resume(&asked);
        native.append(
            block(5, json!({"type":"text","text":"y".repeat(300 * 1024)})),
            json!({}),
        );
        let long = native.read().unwrap();
        assert!(long.events.is_empty(), "no text is invented");
        assert_eq!(user_turn(&long), None, "the typed turn answered it");
        // A whole-file read keeps the same records without their bodies.
        let mut whole =
            hide_session::parse_events(Agent::Grok, &fs::read_to_string(&native.path).unwrap());
        whole.coalesce();
        assert!(whole.events.iter().any(|event| {
            event.kind == hide_session::EventKind::Human
                && (event.text.as_str(), event.images) == ("이 화면을 봐줘", 1)
        }));

        // A question whose body is over the cap still waits, without
        // content, and its answer clears it.
        native.resume(&long);
        native.append(
            json!({"sessionUpdate":"tool_call","toolCallId":"call-big","title":"ask_user_question",
                "rawInput":{"questions":[{"question":"z".repeat(300 * 1024),"options":[]}]},
                "_meta":{"x.ai/tool":{"kind":"ask_user"}}}),
            json!({"promptId":"p-5"}),
        );
        let asked = native.read().unwrap();
        let fact = user_turn(&asked).unwrap();
        assert_eq!((fact.kind, fact.content), (UserTurnKind::Question, None));
        native.resume(&asked);
        native.append(
            json!({"sessionUpdate":"tool_call_update","toolCallId":"call-big","status":"completed",
                "content":[{"type":"content","content":{"type":"text","text":"네"}}]}),
            json!({"promptId":"p-5"}),
        );
        let answered = native.read().unwrap();
        assert_eq!(user_turn(&answered), None);

        // A record the scan cannot follow, nested past its depth, is
        // skipped: the read goes on, and the wait is not known until a
        // prompt starts the next turn.
        native.resume(&answered);
        let mut nested = json!("q".repeat(300 * 1024));
        for _ in 0..70 {
            nested = json!([nested]);
        }
        native.append(
            json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"x"},"data":nested}),
            json!({"promptId":"p-5"}),
        );
        let unknown = native.read().unwrap();
        assert_eq!(
            unknown.skipped_reasons.get("conversation_capacity"),
            Some(&1)
        );
        assert_eq!(unknown.turns.as_ref().unwrap().waiting(), None);
        native.resume(&unknown);
        native.append(block(6, json!({"type":"text","text":"다음"})), json!({}));
        let next = native.read().unwrap();
        assert_eq!(
            next.turns.as_ref().unwrap().waiting(),
            Some(Waiting::Nothing)
        );
    }

    #[test]
    fn plan_approval_follows_plan_mode_state_and_carries_the_plan_text() {
        let mut native = Native::new();
        let folder = native.folder().to_path_buf();
        let plan_mode = |awaiting: bool, state: &str| {
            fs::write(
                folder.join("plan_mode.json"),
                serde_json::to_vec_pretty(&json!({"state":state,"was_previously_active":true,
                    "reminder_count":0,"pending_exit_reminder":false,"awaiting_plan_approval":awaiting}))
                .unwrap(),
            )
            .unwrap();
        };
        plan_mode(false, "Active");
        fs::write(
            native.folder().join("plan.md"),
            "## 계획\n요청 보기를 나눈다\n",
        )
        .unwrap();
        let planning = native.read().unwrap();
        assert_eq!(
            user_turn(&planning),
            None,
            "a plan being written is no wait"
        );
        native.resume(&planning);

        plan_mode(true, "Active");
        let waiting = native.read().unwrap();
        assert!(
            waiting.events.is_empty(),
            "the state is read, no record replayed"
        );
        let fact = user_turn(&waiting).unwrap();
        assert_eq!(fact.kind, UserTurnKind::PlanApproval);
        assert_eq!(fact.content.unwrap().text(), "## 계획\n요청 보기를 나눈다");
        native.resume(&waiting);

        let long = "가".repeat(4_000);
        fs::write(native.folder().join("plan.md"), &long).unwrap();
        let bounded = user_turn(&native.read().unwrap()).unwrap().content.unwrap();
        assert!(bounded.truncated());
        assert!(long.starts_with(bounded.text()));

        plan_mode(false, "Inactive");
        assert_eq!(user_turn(&native.read().unwrap()), None);
        fs::write(
            native.folder().join("plan_mode.json"),
            "{\"awaiting_plan_approval\":\"yes\"",
        )
        .unwrap();
        assert!(
            native.read().is_err(),
            "an unreadable state is refused, never guessed"
        );
    }

    #[test]
    fn current_titles_come_from_the_summary_and_a_cleared_rename_revokes_them() {
        let mut native = Native::new();
        let first = native.read().unwrap();
        assert_eq!(
            (first.title.as_deref(), first.custom_title.as_deref()),
            (Some("요청 보기 만들기"), Some(""))
        );
        native.resume(&first);
        native.summary(|summary| {
            summary["generated_title"] = json!("새 이름");
            summary["title_is_manual"] = json!(true);
        });
        let renamed = native.read().unwrap();
        assert!(renamed.events.is_empty());
        assert_eq!(
            (renamed.title.as_deref(), renamed.custom_title.as_deref()),
            (Some(""), Some("새 이름"))
        );
        native.resume(&renamed);
        native.summary(|summary| {
            summary.as_object_mut().unwrap().remove("generated_title");
        });
        let cleared = native.read().unwrap();
        assert_eq!(
            (cleared.title.as_deref(), cleared.custom_title.as_deref()),
            (Some(""), Some(""))
        );
        native.resume(&cleared);
        native.summary(|summary| {
            summary["generated_title"] = json!("자동 이름");
            summary["title_is_manual"] = json!(false);
        });
        let automatic = native.read().unwrap();
        assert!(automatic.events.is_empty());
        assert_eq!(
            (
                automatic.title.as_deref(),
                automatic.custom_title.as_deref()
            ),
            (Some("자동 이름"), Some(""))
        );
    }

    #[test]
    fn id_and_path_prove_one_owner_and_a_wrong_checkout_folder_or_kind_is_refused() {
        let mut native = Native::new();
        let by_id = native.read().unwrap();
        native.request.reference_kind = "path".into();
        native.request.reference_value = native.path.display().to_string();
        let by_path = native.read().unwrap();
        assert_eq!(by_path.confirmed.owner, by_id.confirmed.owner);
        let other = native.home.path().join("other");
        fs::create_dir(&other).unwrap();
        native.request.cwd = Some(other.display().to_string());
        assert!(native.read().is_err());
        assert_eq!(
            hide_session::confirm_session_file(
                native.home.path(),
                Agent::Grok,
                &native.path,
                None,
                other.to_str()
            )
            .unwrap_err()
            .to_string(),
            "label_session_cwd_mismatch"
        );
        native.request.cwd = Some(native.cwd.display().to_string());

        let confirm = |path: &Path| {
            hide_session::confirm_session_file(
                native.home.path(),
                Agent::Grok,
                path,
                None,
                native.cwd.to_str(),
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
        };
        // The same files in another group or under another id's folder.
        let moved = native
            .home
            .path()
            .join(".grok/sessions/%2Felsewhere")
            .join(GROK_ID);
        fs::create_dir_all(&moved).unwrap();
        for name in ["updates.jsonl", "summary.json"] {
            fs::copy(native.folder().join(name), moved.join(name)).unwrap();
        }
        assert_eq!(
            confirm(&moved.join("updates.jsonl")).unwrap_err(),
            "label_session_default_directory_required"
        );
        let renamed = native
            .folder()
            .with_file_name("0199b000-0000-7000-8000-00000000000f");
        fs::create_dir(&renamed).unwrap();
        for name in ["updates.jsonl", "summary.json"] {
            fs::copy(native.folder().join(name), renamed.join(name)).unwrap();
        }
        assert_eq!(
            confirm(&renamed.join("updates.jsonl")).unwrap_err(),
            "label_session_id_mismatch"
        );
        let outside = native.home.path().join("updates.jsonl");
        fs::copy(&native.path, &outside).unwrap();
        assert!(confirm(&outside).is_err());

        for kind in ["subagent", "subagent_fork", "headless"] {
            native.summary(|summary| summary["session_kind"] = json!(kind));
            assert_eq!(confirm(&native.path).unwrap_err(), "label_session_not_root");
        }
        native.summary(|_| {});
        #[cfg(unix)]
        {
            let summary = native.folder().join("summary.json");
            let target = native.home.path().join("planted.json");
            fs::rename(&summary, &target).unwrap();
            std::os::unix::fs::symlink(&target, &summary).unwrap();
            assert_eq!(confirm(&native.path).unwrap_err(), "label_session_linked");
        }
    }

    #[test]
    fn resume_is_routed_only_from_the_sessions_own_checkout_group() {
        let native = Native::new();
        assert_eq!(native.route(&native.cwd), Ok(()));
        let other = native.home.path().join("other");
        fs::create_dir(&other).unwrap();
        assert!(native.route(&other).is_err());
        // A fork or wake names the proven file, as the label overlay hands it.
        let file = native.path.to_str().unwrap();
        assert_eq!(
            native.route_by("path", file, &native.cwd, Some(GROK_ID)),
            Ok(())
        );
        assert!(
            native
                .route_by("path", file, &other, Some(GROK_ID))
                .is_err()
        );
        assert!(
            native
                .route_by(
                    "path",
                    file,
                    &native.cwd,
                    Some("0199b000-0000-7000-8000-0000000000aa")
                )
                .is_err()
        );
        let located = SessionLocator::new(native.home.path())
            .locate(
                "pane",
                Agent::Grok,
                Some(&SessionIdentity::id(GROK_ID)),
                native.cwd.to_str(),
            )
            .unwrap();
        assert_eq!(located, native.path);
        assert!(
            SessionLocator::new(native.home.path())
                .locate(
                    "pane",
                    Agent::Grok,
                    Some(&SessionIdentity::id("0199b000-0000-7000-8000-0000000000aa")),
                    native.cwd.to_str()
                )
                .is_err()
        );
    }

    #[test]
    fn catalog_and_link_reads_list_only_root_conversations() {
        let native = Native::new();
        let child = native
            .folder()
            .with_file_name("0199b000-0000-7000-8000-0000000000c1");
        fs::create_dir(&child).unwrap();
        fs::copy(&native.path, child.join("updates.jsonl")).unwrap();
        let mut summary: serde_json::Value =
            serde_json::from_slice(&fs::read(native.folder().join("summary.json")).unwrap())
                .unwrap();
        summary["info"]["id"] = json!("0199b000-0000-7000-8000-0000000000c1");
        summary["session_kind"] = json!("subagent");
        fs::write(child.join("summary.json"), summary.to_string()).unwrap();
        let project = hide_project::resolve(&native.cwd, "local").unwrap();
        let sessions = SessionCatalog::new(native.home.path(), "local")
            .project_sessions(&project)
            .unwrap()
            .sessions;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, GROK_ID);
        assert_eq!(sessions[0].agent, Agent::Grok);
        assert_eq!(sessions[0].title.as_deref(), Some("요청 보기 만들기"));
        assert_eq!(
            sessions[0].first_human_request.as_deref(),
            Some("요청 보기를 만들어줘 긴 요청의 둘째 줄")
        );
        let answer = hide_session::links::read(
            native.home.path(),
            &[hide_session::links::ReadRequest {
                agent: Agent::Grok,
                path: native.path.display().to_string(),
                checkpoint: None,
            }],
        )
        .remove(0);
        assert!(answer.error.is_none());
        assert_eq!(answer.facts.session_id.as_deref(), Some(GROK_ID));
        assert_eq!(answer.facts.prs.len(), 1);
        let candidates = hide_session::links::candidates(native.home.path(), 0, None).unwrap();
        let grok: Vec<_> = candidates
            .iter()
            .filter(|candidate| candidate.agent == Agent::Grok)
            .collect();
        let root = fs::canonicalize(&native.path).unwrap();
        assert!(grok.iter().any(|candidate| candidate.path == root));
        assert!(
            grok.iter()
                .all(|candidate| candidate.path.ends_with("updates.jsonl"))
        );
    }

    #[test]
    fn a_torn_record_waits_and_replacement_or_truncation_starts_over_without_duplicates() {
        let mut native = Native::new();
        let first = native.read().unwrap();
        native.resume(&first);
        let torn = r#"{"timestamp":1790989400,"method":"session/update","params":{"sessionId":"x","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"반"#;
        fs::OpenOptions::new()
            .append(true)
            .open(&native.path)
            .unwrap()
            .write_all(torn.as_bytes())
            .unwrap();
        let waiting = native.read().unwrap();
        assert!(waiting.events.is_empty());
        native.resume(&waiting);
        fs::OpenOptions::new()
            .append(true)
            .open(&native.path)
            .unwrap()
            .write_all(
                "쯤\"}},\"_meta\":{\"promptId\":\"p-5\",\"agentTimestampMs\":1790989400000}}}\n"
                    .as_bytes(),
            )
            .unwrap();
        let completed = native.read().unwrap();
        assert_eq!(completed.events.len(), 1);
        assert_eq!(completed.events[0].text, "반쯤");
        native.resume(&completed);

        let body = fs::read(&native.path).unwrap();
        let temporary = native.folder().join("updates.tmp");
        fs::write(&temporary, &body).unwrap();
        fs::rename(&temporary, &native.path).unwrap();
        let replaced = native.read().unwrap();
        assert_eq!(replaced.rescanned.as_deref(), Some("replaced"));
        assert_eq!(
            replaced
                .events
                .iter()
                .filter(|event| event.kind == LabelEventKind::Human)
                .count(),
            3
        );
    }
}

mod pi {
    use super::*;
    use hide_session::{SessionCatalog, SessionIdentity, SessionLocator};
    use serde_json::json;
    use std::fs;
    use std::io::Write;

    struct Native {
        home: tempfile::TempDir,
        cwd: PathBuf,
        path: PathBuf,
        request: LabelTranscriptRequest,
    }

    impl Native {
        fn new() -> Self {
            let home = tempfile::tempdir().unwrap();
            let cwd = home.path().join("checkout");
            fs::create_dir(&cwd).unwrap();
            let cwd = fs::canonicalize(cwd).unwrap();
            // The default folder routes native --session <id>; the header
            // still proves ownership. The filename never supplies the id.
            let encoded = cwd
                .to_string_lossy()
                .trim_start_matches(['/', '\\'])
                .replace(['/', '\\', ':'], "-");
            let path = home
                .path()
                .join(".pi/agent/sessions")
                .join(format!("--{encoded}--"))
                .join("timestamp_not-the-id.jsonl");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let fixture = fs::read_to_string(fixtures().join("pi-1.0.4/session.jsonl")).unwrap();
            let mut lines = fixture.lines();
            let mut header: serde_json::Value =
                serde_json::from_str(lines.next().unwrap()).unwrap();
            header["cwd"] = json!(cwd);
            fs::write(
                &path,
                format!("{header}\n{}\n", lines.collect::<Vec<_>>().join("\n")),
            )
            .unwrap();
            let mut request = request(Agent::Pi);
            request.cwd = Some(cwd.display().to_string());
            Self {
                home,
                cwd,
                path,
                request,
            }
        }

        fn read(&self) -> Result<LabelTranscript, hide_session::ReadRefusal> {
            read(self.home.path(), &self.request)
        }

        fn append(&self, record: serde_json::Value) {
            writeln!(
                fs::OpenOptions::new()
                    .append(true)
                    .open(&self.path)
                    .unwrap(),
                "{record}"
            )
            .unwrap();
        }

        fn resume(&mut self, transcript: &LabelTranscript) {
            self.request.checkpoint = Some(transcript.checkpoint.clone());
        }
    }

    #[test]
    fn native_history_has_the_shared_conversation_facts_and_only_recorded_capabilities() {
        let native = Native::new();
        let transcript = native.read().unwrap();
        assert_eq!(transcript.title.as_deref(), Some("요청 보기 만들기"));
        assert!(
            transcript.custom_title.is_none(),
            "Pi records no title source"
        );
        let people: Vec<_> = transcript
            .events
            .iter()
            .filter(|event| event.kind == LabelEventKind::Human)
            .collect();
        assert_eq!(people.len(), 3);
        assert_eq!(people[0].text, "요청 보기를 만들어줘\n긴 요청의 둘째 줄");
        assert_eq!(people[0].at_unix_ms, START);
        assert_eq!(people[1].sender.as_deref(), Some("ci-lead"));
        assert_eq!((people[2].text.as_str(), people[2].images), ("", 1));
        assert_eq!(
            transcript.events.len(),
            4,
            "tool/thinking/custom/compaction records are no human turn"
        );
        assert_eq!(sighted(&transcript, 12).unwrap().at_unix_ms, START + 30_000);
        assert!(sighted(&transcript, 99).is_none());
        assert_eq!(
            transcript.confirmed.native_session_id.as_deref(),
            Some("pi-native-a")
        );
        assert!(transcript.turns.is_none());
        assert!(transcript.subagents.is_empty());
        let parsed =
            hide_session::parse_events(Agent::Pi, &fs::read_to_string(&native.path).unwrap());
        assert!(parsed.turn_marks.is_empty());
        assert_eq!(parsed.links.cwd.as_deref(), native.cwd.to_str());
        assert_eq!(parsed.links.interactive, None);
        assert_eq!(parsed.links.forked_from, None);
        assert!(!parsed.links.subagent);
    }

    #[test]
    fn exact_native_id_and_path_prove_the_same_owner_but_wrong_cwd_or_id_never_falls_back() {
        let mut native = Native::new();
        let by_id = native.read().unwrap();
        assert_eq!(
            by_id.confirmed.source_path.as_deref(),
            native.path.canonicalize().unwrap().to_str()
        );
        native.request.reference_kind = "path".to_owned();
        native.request.reference_value = native.path.display().to_string();
        let by_path = native.read().unwrap();
        assert_eq!(by_path.confirmed.owner, by_id.confirmed.owner);
        assert_eq!(by_path.confirmed.source_path, by_id.confirmed.source_path);
        let other = native.home.path().join("other-checkout");
        fs::create_dir(&other).unwrap();
        native.request.cwd = Some(other.display().to_string());
        assert!(native.read().is_err());
        native.request.cwd = Some(native.cwd.display().to_string());
        native.request.reference_kind = "id".to_owned();
        native.request.reference_value = "not-the-id".to_owned();
        assert_eq!(
            native.read().unwrap_err(),
            hide_session::ReadRefusal::Refused("session_file_missing".into())
        );
        assert!(
            SessionLocator::new(native.home.path())
                .locate(
                    "pane",
                    Agent::Pi,
                    Some(&SessionIdentity::id("missing")),
                    native.cwd.to_str()
                )
                .is_err()
        );
        assert!(
            hide_session::confirm_session_file(
                native.home.path(),
                Agent::Pi,
                &native.path,
                None,
                None
            )
            .is_err()
        );
        let outside = native.home.path().join("outside.jsonl");
        fs::copy(&native.path, &outside).unwrap();
        assert!(
            hide_session::confirm_session_file(
                native.home.path(),
                Agent::Pi,
                &outside,
                None,
                native.cwd.to_str()
            )
            .is_err()
        );
    }

    #[test]
    fn incremental_titles_clear_without_replaying_history_and_fork_paths_are_not_followed() {
        let mut native = Native::new();
        let first = native.read().unwrap();
        native.resume(&first);
        native.append(json!({"type":"session_info","id":"renamed","parentId":"a000000b","timestamp":"2026-10-03T01:03:00Z","name":"새 이름"}));
        let renamed = native.read().unwrap();
        assert_eq!(renamed.title.as_deref(), Some("새 이름"));
        assert!(renamed.custom_title.is_none());
        assert!(renamed.events.is_empty());
        native.resume(&renamed);
        native.append(json!({"type":"session_info","id":"cleared","parentId":"renamed","timestamp":"2026-10-03T01:03:01Z","name":""}));
        let cleared = native.read().unwrap();
        assert_eq!(cleared.title.as_deref(), Some(""));
        native.resume(&cleared);
        assert!(native.read().unwrap().events.is_empty());
        let body = fs::read_to_string(&native.path).unwrap();
        let mut lines = body.lines();
        let mut header: serde_json::Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        header["parentSession"] = json!(native.home.path().join("private-parent.jsonl"));
        fs::write(
            native.home.path().join("private-parent.jsonl"),
            "must never read this",
        )
        .unwrap();
        fs::write(
            &native.path,
            format!("{header}\n{}\n", lines.collect::<Vec<_>>().join("\n")),
        )
        .unwrap();
        native.request.checkpoint = None;
        let fork = native.read().unwrap();
        assert_eq!(fork.events.len(), 4);
        assert!(fork.subagents.is_empty());
    }

    /// Herdr reports a Pi session as its file. The proof that names the
    /// pane's session from that file and the link reader name the same
    /// session, by the id in the header, so the core can key the link
    /// record's lineage by either.
    #[test]
    fn the_file_herdr_reports_and_the_link_reader_name_one_session() {
        let mut native = Native::new();
        native.request.reference_kind = "path".to_owned();
        native.request.reference_value = native.path.display().to_string();
        let pane = native.read().unwrap();
        let linked = hide_session::links::read(
            native.home.path(),
            &[hide_session::links::ReadRequest {
                agent: Agent::Pi,
                path: native.path.display().to_string(),
                checkpoint: None,
            }],
        )
        .remove(0);
        assert_eq!(linked.error, None);
        assert_eq!(
            pane.confirmed.native_session_id.as_deref(),
            Some("pi-native-a")
        );
        assert_eq!(linked.facts.session_id.as_deref(), Some("pi-native-a"));
    }

    #[test]
    fn catalog_and_link_reads_share_pi_identity_and_tool_provenance() {
        let native = Native::new();
        let project = hide_project::resolve(&native.cwd, "local").unwrap();
        let sessions = SessionCatalog::new(native.home.path(), "local")
            .project_sessions(&project)
            .unwrap()
            .sessions;
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "pi-native-a");
        assert_eq!(sessions[0].agent, Agent::Pi);
        assert_eq!(sessions[0].title.as_deref(), Some("요청 보기 만들기"));
        let answer = hide_session::links::read(
            native.home.path(),
            &[hide_session::links::ReadRequest {
                agent: Agent::Pi,
                path: native.path.display().to_string(),
                checkpoint: None,
            }],
        )
        .remove(0);
        assert!(answer.error.is_none());
        assert_eq!(answer.facts.session_id.as_deref(), Some("pi-native-a"));
        assert_eq!(answer.facts.prs.len(), 1);
        assert_eq!(answer.facts.prs[0].number, 12);
        assert!(
            hide_session::links::candidates(native.home.path(), 0, None)
                .unwrap()
                .iter()
                .any(|candidate| candidate.agent == Agent::Pi)
        );
    }

    #[test]
    fn native_resume_routing_rejects_other_folders_and_reserved_path_ids() {
        let mut native = Native::new();
        let wrong = native
            .home
            .path()
            .join(".pi/agent/sessions/--other-checkout--/copied.jsonl");
        fs::create_dir_all(wrong.parent().unwrap()).unwrap();
        fs::copy(&native.path, &wrong).unwrap();
        native.request.reference_kind = "path".to_owned();
        native.request.reference_value = wrong.display().to_string();
        assert!(native.read().is_err());
        assert_eq!(
            hide_session::confirm_session_file(
                native.home.path(),
                Agent::Pi,
                &wrong,
                None,
                native.cwd.to_str()
            )
            .unwrap_err()
            .to_string(),
            "label_session_default_directory_required"
        );
        native.request.reference_value = native.path.display().to_string();
        let body = fs::read_to_string(&native.path)
            .unwrap()
            .replace("pi-native-a", "native.jsonl");
        fs::write(&native.path, body).unwrap();
        assert!(native.read().is_err());
        assert_eq!(
            hide_session::confirm_session_file(
                native.home.path(),
                Agent::Pi,
                &native.path,
                None,
                native.cwd.to_str()
            )
            .unwrap_err()
            .to_string(),
            "label_session_id_unresumable"
        );
    }

    fn route(native: &Native) -> Result<hide_session::session_activity::SessionActivity, String> {
        hide_session::session_activity::read(
            native.home.path(),
            &hide_session::session_activity::SessionActivityRequest {
                agent: Agent::Pi,
                reference_kind: "id".into(),
                reference_value: "pi-native-a".into(),
                cwd: native.cwd.to_str().map(str::to_owned),
                exact_route: true,
                expected_id: None,
            },
        )
        .map_err(String::from)
    }

    #[test]
    fn native_route_refuses_duplicate_histories_and_missing_exact_ids_before_prefix_fallback() {
        let native = Native::new();
        assert!(route(&native).is_ok());
        let duplicate = native.path.with_file_name("zz-duplicate.jsonl");
        fs::copy(&native.path, &duplicate).unwrap();
        fs::OpenOptions::new()
            .append(true)
            .open(&duplicate)
            .unwrap()
            .write_all(b"different history\n")
            .unwrap();
        assert_eq!(route(&native).unwrap_err(), "session_route_ambiguous");
        let by_path = hide_session::session_activity::SessionActivityRequest {
            agent: Agent::Pi,
            reference_kind: "path".into(),
            reference_value: duplicate.display().to_string(),
            cwd: native.cwd.to_str().map(str::to_owned),
            exact_route: true,
            expected_id: None,
        };
        assert_eq!(
            hide_session::session_activity::read(native.home.path(), &by_path).unwrap_err(),
            hide_session::ReadRefusal::Refused("session_route_ambiguous".into())
        );
        // A reported path alone remains readable, but does not authorize CLI routing.
        assert!(
            hide_session::confirm_session_file(
                native.home.path(),
                Agent::Pi,
                &duplicate,
                Some("pi-native-a"),
                native.cwd.to_str()
            )
            .is_ok()
        );
        fs::remove_file(duplicate).unwrap();
        let contents = fs::read_to_string(&native.path).unwrap();
        fs::write(
            &native.path,
            contents.replace("pi-native-a", "pi-native-ab"),
        )
        .unwrap();
        assert!(
            route(&native).is_err(),
            "missing exact ID cannot borrow its native prefix match"
        );
    }

    #[test]
    fn native_route_refuses_uninspectable_candidates_and_aggregate_metadata_capacity() {
        let native = Native::new();
        let sibling = native.path.with_file_name("candidate.jsonl");
        for content in [
            "not-json\n",
            "\n{\"type\":\"session\",\"id\":\"pi-native-a\"}\n",
            "{}",
        ] {
            fs::write(&sibling, content).unwrap();
            assert!(route(&native).is_err());
        }
        fs::remove_file(&sibling).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&native.path, &sibling).unwrap();
            assert!(route(&native).is_err());
            fs::remove_file(&sibling).unwrap();
        }
        for index in 0..6 {
            let header = json!({"type":"session", "version":3, "id":format!("other-{index}"),
                "cwd":native.cwd, "padding":"x".repeat(200_000)});
            fs::write(
                native.path.with_file_name(format!("budget-{index}.jsonl")),
                format!("{header}\n"),
            )
            .unwrap();
        }
        assert_eq!(
            hide_session::session_activity::read(
                native.home.path(),
                &hide_session::session_activity::SessionActivityRequest {
                    agent: Agent::Pi,
                    reference_kind: "path".into(),
                    reference_value: native.path.display().to_string(),
                    cwd: native.cwd.to_str().map(str::to_owned),
                    exact_route: true,
                    expected_id: None,
                }
            )
            .unwrap_err(),
            hide_session::ReadRefusal::Refused("session_route_capacity".into()),
            "a proven path still requires a bounded whole-candidate routing audit"
        );
    }

    #[test]
    fn queued_reads_refuse_replacement_owner_before_the_body_reader_runs() {
        let native = Native::new();
        let expected = hide_session::SessionReadScope {
            id: "pi-native-a".into(),
            cwd: native.cwd.display().to_string(),
        };
        let old = fs::read_to_string(&native.path).unwrap();
        fs::write(&native.path, old.replace("pi-native-a", "pi-native-b")).unwrap();
        let called = std::cell::Cell::new(false);
        let result = hide_session::read_session_file(
            native.home.path(),
            Agent::Pi,
            &native.path,
            Some(&expected),
            || {
                called.set(true);
                Ok(())
            },
        );
        assert_eq!(result.unwrap_err(), "label_session_id_mismatch");
        assert!(!called.get());
        assert!(
            hide_session::read_session_file(
                native.home.path(),
                Agent::Pi,
                &native.path,
                None,
                || Ok(())
            )
            .is_err()
        );
    }

    #[test]
    fn incremental_link_facts_keep_the_proven_first_header_despite_later_session_records() {
        let native = Native::new();
        let mut request = hide_session::links::ReadRequest {
            agent: Agent::Pi,
            path: native.path.display().to_string(),
            checkpoint: None,
        };
        let first = hide_session::links::read(native.home.path(), &[request.clone()]).remove(0);
        assert!(first.error.is_none());
        request.checkpoint = first.checkpoint;
        native.append(json!({"type":"session", "version":0, "id":"other-owner", "cwd":"/other"}));
        native.append(json!({"type":"message", "timestamp":"2026-10-03T01:02:00Z", "message":{"role":"toolResult", "content":[{"type":"text", "text":"https://github.com/acme/project/pull/44"}]}}));
        let next = hide_session::links::read(native.home.path(), &[request]).remove(0);
        assert!(next.error.is_none());
        assert_eq!(next.facts.session_id.as_deref(), Some("pi-native-a"));
        assert_eq!(next.facts.cwd.as_deref(), native.cwd.to_str());
        assert!(next.facts.prs.iter().any(|pr| pr.number == 44));
    }

    #[test]
    fn native_route_and_queued_reads_do_not_trust_colliding_encoded_checkouts() {
        let mut native = Native::new();
        let a = native.home.path().join("a-b");
        let b = native.home.path().join("a/b");
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        let a = hide_platform::fs::identity::canonical(&a).unwrap();
        let b = hide_platform::fs::identity::canonical(&b).unwrap();
        let folder = native.home.path().join(".pi/agent/sessions").join(format!(
            "--{}--",
            a.to_string_lossy()
                .trim_start_matches(['/', '\\'])
                .replace(['/', '\\', ':'], "-")
        ));
        fs::create_dir_all(&folder).unwrap();
        native.path = folder.join("a.jsonl");
        native.cwd = a.clone();
        fs::write(
            &native.path,
            format!(
                "{}\n",
                json!({"type":"session", "version":3, "id":"pi-native-a", "cwd":a})
            ),
        )
        .unwrap();
        assert!(route(&native).is_ok());
        fs::write(
            folder.join("b.jsonl"),
            format!(
                "{}\n",
                json!({"type":"session", "version":3, "id":"pi-native-a", "cwd":b})
            ),
        )
        .unwrap();
        assert_eq!(route(&native).unwrap_err(), "session_route_ambiguous");
        let expected = hide_session::SessionReadScope {
            id: "pi-native-a".into(),
            cwd: a.display().to_string(),
        };
        fs::write(
            &native.path,
            format!(
                "{}\n",
                json!({"type":"session", "version":3, "id":"pi-native-a", "cwd":b})
            ),
        )
        .unwrap();
        let result = hide_session::read_session_file(
            native.home.path(),
            Agent::Pi,
            &native.path,
            Some(&expected),
            || Ok(()),
        );
        assert_eq!(result.unwrap_err(), "label_session_cwd_mismatch");
    }

    #[test]
    fn native_id_discovery_has_one_shared_metadata_read_budget() {
        let native = Native::new();
        fs::remove_file(&native.path).unwrap();
        for index in 0..6 {
            let record = json!({"type":"session","version":3,"id":format!("other-{index}"),"cwd":native.cwd,"extra":"x".repeat(200_000)});
            fs::write(
                native.path.with_file_name(format!("{index}.jsonl")),
                format!("{record}\n"),
            )
            .unwrap();
        }
        let error = native.read().unwrap_err();
        assert_eq!(
            error,
            hide_session::ReadRefusal::Refused("session_capacity".into())
        );
    }

    #[test]
    fn torn_lines_and_replacement_or_truncation_are_observable_without_cross_owner_reads() {
        let mut native = Native::new();
        let first = native.read().unwrap();
        native.resume(&first);
        let line = json!({"type":"message","id":"a000000c","parentId":"a000000b","timestamp":"2026-10-03T01:04:00Z","message":{"role":"user","content":"다음 요청"}}).to_string();
        fs::OpenOptions::new()
            .append(true)
            .open(&native.path)
            .unwrap()
            .write_all(line.as_bytes())
            .unwrap();
        let torn = native.read().unwrap();
        assert!(torn.events.is_empty());
        native.resume(&torn);
        fs::OpenOptions::new()
            .append(true)
            .open(&native.path)
            .unwrap()
            .write_all(b"\n")
            .unwrap();
        let completed = native.read().unwrap();
        assert_eq!(completed.events.len(), 1);
        assert_eq!(completed.events[0].text, "다음 요청");
        native.resume(&completed);
        let replacement = native.path.with_extension("replacement");
        fs::copy(&native.path, &replacement).unwrap();
        fs::rename(replacement, &native.path).unwrap();
        let replaced = native.read().unwrap();
        assert_eq!(replaced.rescanned.as_deref(), Some("replaced"));
        assert_ne!(first.confirmed.incarnation, replaced.confirmed.incarnation);
        native.resume(&replaced);
        let header = fs::read_to_string(&native.path)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_owned();
        fs::write(&native.path, format!("{header}\n")).unwrap();
        let truncated = native.read().unwrap();
        assert_eq!(truncated.rescanned.as_deref(), Some("truncated"));
        assert!(truncated.events.is_empty());
        fs::write(
            &native.path,
            format!("{{\"type\":\"message\"}}\n{header}\n"),
        )
        .unwrap();
        assert!(
            native.read().is_err(),
            "only the first native header proves identity"
        );
    }

    #[cfg(unix)]
    #[test]
    fn internal_file_and_directory_links_cannot_grant_native_read_authority() {
        let mut native = Native::new();
        let hard_link = native.path.with_file_name("hard-linked.jsonl");
        fs::hard_link(&native.path, &hard_link).unwrap();
        assert!(native.read().is_err());
        assert_eq!(
            hide_session::confirm_session_file(
                native.home.path(),
                Agent::Pi,
                &native.path,
                None,
                native.cwd.to_str()
            )
            .unwrap_err()
            .to_string(),
            "label_session_linked"
        );
        fs::remove_file(hard_link).unwrap();
        let file_link = native.path.with_file_name("linked.jsonl");
        std::os::unix::fs::symlink(&native.path, &file_link).unwrap();
        native.request.reference_kind = "path".to_owned();
        native.request.reference_value = file_link.display().to_string();
        assert!(native.read().is_err());
        let root = native.home.path().join(".pi/agent/sessions");
        let directory_link = root.join("linked-directory");
        std::os::unix::fs::symlink(native.path.parent().unwrap(), &directory_link).unwrap();
        native.request.reference_value = directory_link
            .join(native.path.file_name().unwrap())
            .display()
            .to_string();
        assert!(native.read().is_err());
        let real_root = native.home.path().join("moved-root");
        fs::rename(&root, &real_root).unwrap();
        std::os::unix::fs::symlink(&real_root, &root).unwrap();
        native.request.reference_value = native.path.display().to_string();
        assert!(native.read().is_err());
        assert!(
            hide_session::links::candidates(native.home.path(), 0, None)
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn what_only_one_agent_records_is_read_from_it() {
    // Claude Code: the operator's rename, and a subagent's tools.
    let claude_home = home(Agent::Claude);
    let claude = read_whole(claude_home.path(), Agent::Claude);
    assert_eq!(claude.custom_title.as_deref(), Some("운영자 이름"));
    assert_eq!(sighted(&claude, 13).unwrap().at_unix_ms, START + 90_000);
    assert!(claude.subagents.contains_key("agent-a1.jsonl"));

    // Codex: a custom tool's output.
    let codex_home = home(Agent::Codex);
    let codex = read_whole(codex_home.path(), Agent::Codex);
    assert_eq!(sighted(&codex, 14).unwrap().at_unix_ms, START + 40_000);
}

/// Task identity: only a native record with manual provenance gets step one.
#[test]
fn claude_titles_keep_the_manual_rename_separate_from_the_automatic_name() {
    let native = home(Agent::Claude);
    let transcript = read_whole(native.path(), Agent::Claude);
    assert_eq!(transcript.title.as_deref(), Some("요청 보기 만들기"));
    assert_eq!(transcript.custom_title.as_deref(), Some("운영자 이름"));
    let id = request(Agent::Claude).reference_value;
    let file = native
        .path()
        .join(".claude/projects/-work-app")
        .join(format!("{id}.jsonl"));
    append(
        &file,
        &[serde_json::json!({"type":"custom-title", "customTitle":"", "sessionId":id})],
    );
    let cleared = read_whole(native.path(), Agent::Claude);
    assert_eq!(cleared.title.as_deref(), Some("요청 보기 만들기"));
    assert_eq!(cleared.custom_title.as_deref(), Some(""));
    append(
        &file,
        &[serde_json::json!({"type":"ai-title", "aiTitle":"", "sessionId":id})],
    );
    assert_eq!(
        read_whole(native.path(), Agent::Claude).title.as_deref(),
        Some("")
    );
}

#[test]
fn codex_thread_names_without_source_are_automatic_even_after_a_rename() {
    let native = home(Agent::Codex);
    let transcript = read_whole(native.path(), Agent::Codex);
    assert_eq!(transcript.title.as_deref(), Some("요청 보기 만들기"));
    assert!(transcript.custom_title.is_none());
    use std::io::Write;
    writeln!(
        std::fs::OpenOptions::new()
            .append(true)
            .open(native.path().join(".codex/session_index.jsonl"))
            .unwrap(),
        "{}",
        serde_json::json!({"id":request(Agent::Codex).reference_value,
        "thread_name":"운영자 이름", "updated_at":"2026-10-03T01:03:00.000Z"})
    )
    .unwrap();
    let renamed = read_whole(native.path(), Agent::Codex);
    assert_eq!(renamed.title.as_deref(), Some("운영자 이름"));
    assert!(renamed.custom_title.is_none());
}

#[test]
fn opencode_root_titles_without_source_are_automatic_even_after_a_rename() {
    let native = home(Agent::OpenCode);
    let first = read_whole(native.path(), Agent::OpenCode);
    assert_eq!(first.title.as_deref(), Some("요청 보기 만들기"));
    assert!(first.custom_title.is_none());
    let db = rusqlite::Connection::open(native.path().join(".local/share/opencode/opencode.db"))
        .unwrap();
    db.execute(
        "UPDATE session SET title = ?1 WHERE id = ?2",
        rusqlite::params!["운영자 이름", request(Agent::OpenCode).reference_value],
    )
    .unwrap();
    drop(db);
    let renamed = read_whole(native.path(), Agent::OpenCode);
    assert_eq!(renamed.title.as_deref(), Some("운영자 이름"));
    assert!(renamed.custom_title.is_none());
}

#[test]
fn a_locked_opencode_database_is_a_skipped_read_that_never_blocks_opencode() {
    let home = home(Agent::OpenCode);
    let path = home.path().join(".local/share/opencode/opencode.db");
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer.execute_batch("BEGIN EXCLUSIVE;").unwrap();
    let started = std::time::Instant::now();
    assert_eq!(
        read(home.path(), &request(Agent::OpenCode)).unwrap_err(),
        hide_session::ReadRefusal::InMotion("opencode_db_busy")
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    // OpenCode's write goes on and the next read takes it.
    writer.execute_batch("COMMIT;").unwrap();
    assert!(read(home.path(), &request(Agent::OpenCode)).is_ok());
}

#[test]
fn an_opencode_message_still_being_written_waits_for_the_next_read() {
    let home = home(Agent::OpenCode);
    let path = home.path().join(".local/share/opencode/opencode.db");
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer
        .execute(
            "INSERT INTO message VALUES ('msg_07', ?1, ?2, ?2, ?3)",
            rusqlite::params![
                "ses_0a1b2c3d4e5f60718293a4b5c6",
                START + 200_000,
                r#"{"role":"assistant","time":{"created":1790989400000}}"#
            ],
        )
        .unwrap();
    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    assert!(!first.has_more);
    let mut again = request(Agent::OpenCode);
    again.checkpoint = Some(first.checkpoint.clone());
    assert!(read(home.path(), &again).unwrap().events.is_empty());
}

/// Adds an operator message with one text part of `bytes` bytes.
fn add_opencode_request(home: &Path, index: u64, bytes: usize) {
    let path = home.join(".local/share/opencode/opencode.db");
    let writer = rusqlite::Connection::open(path).unwrap();
    let at = START + 300_000 + index;
    let message = format!("msg_big_{index:03}");
    writer
        .execute(
            "INSERT INTO message VALUES (?1, ?2, ?3, ?3, ?4)",
            rusqlite::params![
                message,
                "ses_0a1b2c3d4e5f60718293a4b5c6",
                at,
                format!(r#"{{"role":"user","time":{{"created":{at}}}}}"#)
            ],
        )
        .unwrap();
    writer
        .execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
            rusqlite::params![
                format!("prt_big_{index:03}"),
                message,
                "ses_0a1b2c3d4e5f60718293a4b5c6",
                at,
                format!(
                    r#"{{"type":"text","text":"{index} {}"}}"#,
                    "x".repeat(bytes)
                )
            ],
        )
        .unwrap();
}

#[test]
fn an_opencode_part_over_the_row_limit_is_skipped_without_being_read() {
    let home = home(Agent::OpenCode);
    add_opencode_request(home.path(), 0, hide_session::SESSION_LINE_LIMIT_BYTES);
    let transcript = read_whole(home.path(), Agent::OpenCode);
    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    assert_eq!(first.skipped_reasons.get("part_capacity"), Some(&1));
    assert!(
        transcript
            .events
            .iter()
            .all(|event| !event.text.starts_with("0 x")),
        "the oversized request is no event"
    );
}

#[test]
fn a_long_opencode_session_is_read_a_budget_at_a_time_and_whole_in_the_end() {
    let home = home(Agent::OpenCode);
    for index in 0..8 {
        add_opencode_request(home.path(), index, 200 * 1024);
    }
    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    assert!(
        first.has_more,
        "eight 200 KiB requests are more than one read"
    );
    let loaded: usize = first.events.iter().map(|event| event.text.len()).sum();
    assert!(loaded <= 1024 * 1024 + 4096, "{loaded}");

    let whole = read_whole(home.path(), Agent::OpenCode);
    let big: Vec<_> = whole
        .events
        .iter()
        .filter(|event| event.text.ends_with("xxxx"))
        .map(|event| event.text.split(' ').next().unwrap().to_owned())
        .collect();
    assert_eq!(big, ["0", "1", "2", "3", "4", "5", "6", "7"]);
}

#[test]
fn opencode_metadata_uses_the_read_budget_even_when_it_cannot_be_parsed() {
    let home = home(Agent::OpenCode);
    let path = home.path().join(".local/share/opencode/opencode.db");
    let writer = rusqlite::Connection::open(path).unwrap();
    writer
        .execute_batch("DELETE FROM part; DELETE FROM message;")
        .unwrap();
    // Each record fits the metadata row limit, but together they exceed
    // the read budget by more than ten times. Invalid JSON is still input
    // that the reader loaded, and must consume the same byte budget.
    let metadata = "x".repeat(63 * 1024);
    for index in 0..200 {
        writer
            .execute(
                "INSERT INTO message VALUES (?1, ?2, ?3, ?3, ?4)",
                rusqlite::params![
                    format!("msg_metadata_{index:03}"),
                    request(Agent::OpenCode).reference_value,
                    START + index,
                    metadata
                ],
            )
            .unwrap();
    }
    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    assert!(first.events.is_empty());
    assert_eq!(first.checkpoint.offset(), 16);
    assert_eq!(first.skipped_reasons.get("malformed_json"), Some(&16));
    assert!(first.has_more, "remaining metadata belongs to a later read");
    let mut again = request(Agent::OpenCode);
    again.checkpoint = Some(first.checkpoint);
    let next = read(home.path(), &again).unwrap();
    assert_eq!(next.checkpoint.offset(), 32);
    assert_eq!(next.skipped_reasons.get("malformed_json"), Some(&16));
}

#[test]
fn opencode_empty_and_skipped_parts_have_a_work_cap_and_advance() {
    for data in [
        rusqlite::types::Value::Text(String::new()),
        rusqlite::types::Value::Blob(vec![0]),
    ] {
        let home = home(Agent::OpenCode);
        let writer =
            rusqlite::Connection::open(home.path().join(".local/share/opencode/opencode.db"))
                .unwrap();
        writer
            .execute_batch("DELETE FROM part; DELETE FROM message;")
            .unwrap();
        add_opencode_request(home.path(), 0, 10);
        writer
            .execute("DELETE FROM part WHERE message_id = 'msg_big_000'", [])
            .unwrap();
        for index in 0..2_000 {
            writer
                .execute(
                    "INSERT INTO part VALUES (?1, 'msg_big_000', ?2, ?3, ?3, ?4)",
                    rusqlite::params![
                        format!("prt_empty_{index:04}"),
                        request(Agent::OpenCode).reference_value,
                        START + index,
                        data
                    ],
                )
                .unwrap();
        }
        add_opencode_request(home.path(), 1, 10);
        let transcript = read(home.path(), &request(Agent::OpenCode)).unwrap();
        let reason = if matches!(data, rusqlite::types::Value::Text(_)) {
            "malformed_json"
        } else {
            "not_text"
        };
        assert_eq!(transcript.skipped_reasons.get(reason), Some(&256));
        assert_eq!(
            transcript.skipped_reasons.get("part_work_capacity"),
            Some(&1)
        );
        assert_eq!(transcript.checkpoint.offset(), 2);
        assert!(
            transcript
                .events
                .iter()
                .any(|event| event.text.starts_with("1 x"))
        );
        assert!(!transcript.has_more);
    }
}

#[test]
fn opencode_part_work_is_bounded_across_messages_and_resumes() {
    let home = home(Agent::OpenCode);
    let writer =
        rusqlite::Connection::open(home.path().join(".local/share/opencode/opencode.db")).unwrap();
    writer
        .execute_batch("DELETE FROM part; DELETE FROM message;")
        .unwrap();
    for message in 0..3 {
        add_opencode_request(home.path(), message, 10);
        if message == 2 {
            continue;
        }
        writer
            .execute(
                "DELETE FROM part WHERE message_id = ?1",
                [format!("msg_big_{message:03}")],
            )
            .unwrap();
        for index in 0..300 {
            writer
                .execute(
                    "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?4, '')",
                    rusqlite::params![
                        format!("prt_work_{message}_{index:03}"),
                        format!("msg_big_{message:03}"),
                        request(Agent::OpenCode).reference_value,
                        START + index
                    ],
                )
                .unwrap();
        }
    }
    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    assert!(first.events.is_empty());
    assert_eq!(first.skipped_reasons.get("malformed_json"), Some(&512));
    assert_eq!(first.skipped_reasons.get("part_work_capacity"), Some(&2));
    assert_eq!(first.checkpoint.offset(), 2);
    assert!(first.has_more);
    let mut again = request(Agent::OpenCode);
    again.checkpoint = Some(first.checkpoint);
    let last = read(home.path(), &again).unwrap();
    assert!(
        last.events
            .iter()
            .any(|event| event.text.starts_with("2 x"))
    );
    assert_eq!(last.checkpoint.offset(), 3);
    assert!(!last.has_more);
}

#[test]
fn opencode_bounds_sql_work_when_a_part_sort_cannot_use_an_index() {
    let home = home(Agent::OpenCode);
    let writer =
        rusqlite::Connection::open(home.path().join(".local/share/opencode/opencode.db")).unwrap();
    writer
        .execute_batch("DELETE FROM part; DELETE FROM message;")
        .unwrap();
    add_opencode_request(home.path(), 0, 10);
    writer.execute_batch("WITH RECURSIVE rows(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM rows WHERE n<100000) INSERT INTO part SELECT printf('prt_many_%06d', n), 'msg_big_000', 'ses_0a1b2c3d4e5f60718293a4b5c6', n, n, '' FROM rows;").unwrap();
    assert_eq!(
        read(home.path(), &request(Agent::OpenCode)).unwrap_err(),
        hide_session::ReadRefusal::Refused("opencode_db_work_capacity".into())
    );
    let parts: i64 = writer
        .query_row("SELECT count(*) FROM part", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        parts, 100001,
        "the bounded read must not change the provider database"
    );
}

#[test]
fn an_oversized_first_opencode_message_has_bounded_output_and_allows_progress() {
    let home = home(Agent::OpenCode);
    let path = home.path().join(".local/share/opencode/opencode.db");
    let writer = rusqlite::Connection::open(path).unwrap();
    writer
        .execute_batch("DELETE FROM part; DELETE FROM message;")
        .unwrap();
    add_opencode_request(home.path(), 0, 200 * 1024);
    for index in 1..12 {
        writer
            .execute(
                "INSERT INTO part VALUES (?1, 'msg_big_000', ?2, ?3, ?3, ?4)",
                rusqlite::params![
                    format!("prt_extra_{index:03}"),
                    request(Agent::OpenCode).reference_value,
                    START + 300_000 + index,
                    serde_json::json!({ "type": "text", "text": "x".repeat(200 * 1024) })
                        .to_string()
                ],
            )
            .unwrap();
    }
    add_opencode_request(home.path(), 1, 200 * 1024);
    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    let output_bytes: usize = first.events.iter().map(|event| event.text.len()).sum();
    assert!(output_bytes <= hide_session::SESSION_INCREMENT_READ_LIMIT_BYTES as usize);
    assert_eq!(first.checkpoint.offset(), 1);
    assert_eq!(first.skipped_reasons.get("read_budget"), Some(&7));
    assert!(first.has_more);
    let mut again = request(Agent::OpenCode);
    again.checkpoint = Some(first.checkpoint);
    let next = read(home.path(), &again).unwrap();
    assert!(
        next.events
            .iter()
            .any(|event| event.text.starts_with("1 x"))
    );
    assert_eq!(next.checkpoint.offset(), 2);
    assert!(!next.has_more);
}

#[test]
fn an_opencode_row_that_holds_no_text_or_too_much_is_skipped_and_the_read_goes_on() {
    let home = home(Agent::OpenCode);
    let path = home.path().join(".local/share/opencode/opencode.db");
    let writer = rusqlite::Connection::open(&path).unwrap();
    writer
        .execute(
            "UPDATE session SET title = ?1 WHERE id = ?2",
            rusqlite::params!["제".repeat(100_000), "ses_0a1b2c3d4e5f60718293a4b5c6"],
        )
        .unwrap();
    add_opencode_request(home.path(), 0, 10);
    writer
        .execute(
            "UPDATE part SET data = CAST(data AS BLOB) WHERE id = 'prt_big_000'",
            [],
        )
        .unwrap();
    add_opencode_request(home.path(), 1, 10);
    writer
        .execute(
            "UPDATE message SET data = json_set(data, '$.padding', ?1) WHERE id = 'msg_big_001'",
            rusqlite::params!["x".repeat(100 * 1024)],
        )
        .unwrap();
    add_opencode_request(home.path(), 2, 10);

    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    assert_eq!(first.title, None, "a title past the cut is no title");
    assert_eq!(first.skipped_reasons.get("not_text"), Some(&1));
    assert_eq!(first.skipped_reasons.get("message_capacity"), Some(&1));
    assert!(
        first
            .events
            .iter()
            .any(|event| event.text.starts_with("2 x")),
        "the request after them is read"
    );
}

/// A Codex 0.160.1 session whose last turn ran in plan mode and proposed a
/// plan (PRD codex-plan-approval-hold D-03, B12): the records Codex writes
/// before it shows "Implement this plan?".
const PLAN_SESSION: &str = "0199a000-0000-7000-8000-0000000000a1";

fn plan_home() -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    copy_tree(
        &fixtures().join("codex-0.160.1-plan"),
        &home.path().join(".codex"),
    );
    let rollout = home.path().join(format!(
        ".codex/sessions/2026/10/07/rollout-2026-10-07T01-00-00-{PLAN_SESSION}.jsonl"
    ));
    (home, rollout)
}

fn plan_request() -> LabelTranscriptRequest {
    LabelTranscriptRequest {
        reference_value: PLAN_SESSION.to_owned(),
        ..request(Agent::Codex)
    }
}

/// The request that continues where `answer` stopped, as the label worker
/// keeps it across reads and restarts (through its persisted form).
fn continued(answer: &LabelTranscript) -> LabelTranscriptRequest {
    fn persisted<T: serde::Serialize + serde::de::DeserializeOwned>(value: &T) -> T {
        serde_json::from_slice(&serde_json::to_vec(value).unwrap()).unwrap()
    }
    LabelTranscriptRequest {
        checkpoint: Some(persisted(&answer.checkpoint)),
        turns: answer.turns.as_ref().map(persisted),
        ..plan_request()
    }
}

fn append(path: &Path, records: &[serde_json::Value]) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    for record in records {
        writeln!(file, "{record}").unwrap();
    }
}

fn event(kind: &str, turn: &str, extra: serde_json::Value) -> serde_json::Value {
    let mut payload = serde_json::json!({"type": kind, "turn_id": turn});
    payload
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::json!({"timestamp":"2026-10-07T01:02:00.000Z","type":"event_msg","payload":payload})
}

fn person(text: &str) -> serde_json::Value {
    serde_json::json!({"timestamp":"2026-10-07T01:02:00.100Z","type":"response_item",
        "payload":{"type":"message","role":"user","content":[{"type":"input_text","text":text}]}})
}

fn waiting(answer: &LabelTranscript) -> Option<Waiting> {
    answer
        .turns
        .as_ref()
        .expect("a Codex read reports its turns")
        .waiting()
}

#[test]
fn a_codex_plan_turn_waits_for_approval_until_the_next_turn_starts() {
    let (home, rollout) = plan_home();
    let proposed = read(home.path(), &plan_request()).unwrap();
    assert_eq!(waiting(&proposed), Some(Waiting::PlanApproval));

    // Approving starts the next turn in default mode.
    append(
        &rollout,
        &[
            event(
                "task_started",
                "turn-3",
                serde_json::json!({"collaboration_mode_kind":"default"}),
            ),
            person("Implement the plan."),
        ],
    );
    let approved = read(home.path(), &continued(&proposed)).unwrap();
    assert_eq!(waiting(&approved), Some(Waiting::Nothing));
}

#[test]
fn a_persons_message_after_the_plan_ends_the_wait() {
    let (home, rollout) = plan_home();
    let proposed = read(home.path(), &plan_request()).unwrap();
    append(&rollout, &[person("계획을 조금 바꿔줘")]);
    let answered = read(home.path(), &continued(&proposed)).unwrap();
    assert_eq!(waiting(&answered), Some(Waiting::Nothing));
}

#[test]
fn a_plan_mode_turn_that_ends_in_a_question_or_is_interrupted_does_not_wait() {
    for ending in ["question", "interrupted"] {
        let (home, rollout) = plan_home();
        let first = read(home.path(), &plan_request()).unwrap();
        let mut records = vec![
            event(
                "task_started",
                "turn-3",
                serde_json::json!({"collaboration_mode_kind":"plan"}),
            ),
            person("범위를 정해줘"),
        ];
        records.push(match ending {
            "question" => serde_json::json!({"timestamp":"2026-10-07T01:02:01.000Z",
                "type":"response_item","payload":{"type":"message","role":"assistant",
                "content":[{"type":"output_text","text":"어느 화면부터 볼까요?"}]}}),
            _ => event(
                "turn_aborted",
                "turn-3",
                serde_json::json!({"reason":"interrupted"}),
            ),
        });
        if ending == "question" {
            records.push(event(
                "task_complete",
                "turn-3",
                serde_json::json!({"last_agent_message":"어느 화면부터 볼까요?"}),
            ));
        }
        append(&rollout, &records);
        let after = read(home.path(), &continued(&first)).unwrap();
        assert_eq!(waiting(&after), Some(Waiting::Nothing), "{ending}");
    }
}

#[test]
fn a_plan_turn_still_running_is_not_known_and_a_default_turn_is_not_waiting() {
    let (plan, rollout) = plan_home();
    let first = read(plan.path(), &plan_request()).unwrap();
    append(
        &rollout,
        &[
            event(
                "task_started",
                "turn-3",
                serde_json::json!({"collaboration_mode_kind":"plan"}),
            ),
            person("다른 계획도 세워줘"),
        ],
    );
    let running = read(plan.path(), &continued(&first)).unwrap();
    assert_eq!(waiting(&running), None);

    // The fixture's other agent session finishes a turn with no mode.
    let codex = home(Agent::Codex);
    assert_eq!(
        waiting(&read_whole(codex.path(), Agent::Codex)),
        Some(Waiting::Nothing)
    );
    let claude = home(Agent::Claude);
    assert_eq!(
        waiting(&read_whole(claude.path(), Agent::Claude)),
        Some(Waiting::Nothing)
    );
}

/// D-07: records this reader does not recognise never read as "nothing
/// waits": a plan turn whose mode value changed, and a session whose turn
/// records were renamed, are not known.
#[test]
fn a_changed_codex_record_format_is_not_known_rather_than_not_waiting() {
    let (home, rollout) = plan_home();
    let original = std::fs::read_to_string(&rollout).unwrap();
    let mode_changed = original.replace(
        r#""collaboration_mode_kind":"plan""#,
        r#""collaboration_mode_kind":"Plan""#,
    );
    assert_ne!(mode_changed, original);
    std::fs::write(&rollout, &mode_changed).unwrap();
    assert_eq!(waiting(&read(home.path(), &plan_request()).unwrap()), None);

    let renamed = original
        .replace(r#""type":"task_started""#, r#""type":"turn_started""#)
        .replace(r#""type":"item_completed""#, r#""type":"item_done""#)
        .replace(r#""type":"task_complete""#, r#""type":"turn_complete""#)
        .replace("<proposed_plan>", "<plan>");
    std::fs::write(&rollout, renamed).unwrap();
    assert_eq!(waiting(&read(home.path(), &plan_request()).unwrap()), None);
}

#[test]
fn a_read_split_inside_the_plan_turn_and_resumed_at_its_anchor_answers_the_same() {
    let (home, rollout) = plan_home();
    let whole = std::fs::read_to_string(&rollout).unwrap();
    let lines: Vec<&str> = whole.split_inclusive('\n').collect();
    // Stop after the plan turn's person message, before its plan.
    std::fs::write(&rollout, lines[..11].concat()).unwrap();
    let first = read(home.path(), &plan_request()).unwrap();
    assert_eq!(waiting(&first), None, "a plan turn still running");
    std::fs::write(&rollout, whole).unwrap();
    let second = read(home.path(), &continued(&first)).unwrap();
    assert_eq!(waiting(&second), Some(Waiting::PlanApproval));

    // A restarted reader resumes at the anchor, the turn's person message,
    // with the tracker it kept: that message does not answer the plan.
    let anchor = first.anchor.clone().expect("the read returned a person");
    let resumed = LabelTranscriptRequest {
        checkpoint: Some(anchor),
        ..continued(&second)
    };
    assert_eq!(
        waiting(&read(home.path(), &resumed).unwrap()),
        Some(Waiting::PlanApproval)
    );
}

#[test]
fn a_replaced_session_file_folds_its_turns_again() {
    let (home, rollout) = plan_home();
    let proposed = read(home.path(), &plan_request()).unwrap();
    // The file is replaced by one shorter than what was read: a rescan, so
    // the previous turn's wait does not carry over.
    let whole = std::fs::read_to_string(&rollout).unwrap();
    let lines: Vec<&str> = whole.split_inclusive('\n').collect();
    std::fs::write(&rollout, lines[..8].concat()).unwrap();
    let rescanned = read(home.path(), &continued(&proposed)).unwrap();
    assert!(rescanned.rescanned.is_some());
    assert_eq!(waiting(&rescanned), Some(Waiting::Nothing));
}

/// Hide's OpenCode plugin adds one synthetic text part to a root prompt; its
/// Memory receipt is reported for the core to check, and the part stays out
/// of the conversation the labels and titles read (PRD opencode-plugin D-12).
#[test]
fn hides_receipt_in_an_opencode_synthetic_part_is_reported_and_kept_out_of_the_conversation() {
    let home = home(Agent::OpenCode);
    add_opencode_request(home.path(), 0, 10);
    let path = home.path().join(".local/share/opencode/opencode.db");
    let writer = rusqlite::Connection::open(path).unwrap();
    let receipt =
        r#"<hide-memory-receipt event="UserPromptSubmit" count="1" items="mem-1@2" auth="aaaa" />"#;
    let text = format!(
        "<system-reminder>\nProject Memory:\n- Keep the parser pure.\n{receipt}\n</system-reminder>"
    );
    let at = START + 300_000;
    for (id, synthetic, body) in [
        ("prt_big_000_hide", true, text.as_str()),
        // An operator's own text quoting a receipt is no receipt.
        ("prt_big_000_typed", false, receipt),
    ] {
        writer
            .execute(
                "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
                rusqlite::params![
                    id,
                    "msg_big_000",
                    "ses_0a1b2c3d4e5f60718293a4b5c6",
                    at + 1,
                    serde_json::json!({"type": "text", "text": body, "synthetic": synthetic})
                        .to_string()
                ],
            )
            .unwrap();
    }

    let transcript = read_whole(home.path(), Agent::OpenCode);

    let request = transcript
        .events
        .iter()
        .find(|event| event.text.starts_with("0 x"))
        .expect("the operator's request is an event");
    assert!(!request.text.contains("Project Memory"), "{}", request.text);
    assert_eq!(
        transcript.memory_receipts,
        [hide_session::label_transcript::MemoryReceiptPart {
            context_reset: false,
            offset: request.offset,
            text: receipt.to_owned(),
        }]
    );
}

/// A hook's context Claude Code stored as an attachment carries Hide's Memory
/// receipt to the label read, at the attachment record's offset, the turn a
/// later read of the same file names it by; an operator's message quoting a
/// receipt is no receipt (PRD project-memory-v2 D-16).
#[test]
fn a_claude_hook_attachment_receipt_is_reported_and_a_quoted_one_is_not() {
    let home = home(Agent::Claude);
    let path = home
        .path()
        .join(".claude/projects/-work-app/a1b2c3d4-0000-4000-8000-000000000001.jsonl");
    let receipt =
        r#"<hide-memory-receipt event="UserPromptSubmit" count="1" items="m3@1" auth="aaaa" />"#;
    let offset = std::fs::metadata(&path).unwrap().len();
    let records = [
        serde_json::json!({
            "type": "attachment",
            "timestamp": "2026-10-11T00:00:00.000Z",
            "sessionId": "a1b2c3d4-0000-4000-8000-000000000001",
            "cwd": "/work/app",
            "attachment": {
                "type": "hook_additional_context",
                "hookName": "UserPromptSubmit",
                "hookEvent": "UserPromptSubmit",
                "content": [format!("<hide-memory>\nKeep the parser pure.\n</hide-memory>\n{receipt}")],
            },
        }),
        serde_json::json!({
            "type": "user",
            "timestamp": "2026-10-11T00:00:01.000Z",
            "sessionId": "a1b2c3d4-0000-4000-8000-000000000001",
            "cwd": "/work/app",
            "message": {"role": "user", "content": receipt},
        }),
    ];
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    for record in records {
        use std::io::Write;
        writeln!(file, "{record}").unwrap();
    }
    drop(file);

    let transcript = read_whole(home.path(), Agent::Claude);

    assert_eq!(
        transcript.memory_receipts,
        [hide_session::label_transcript::MemoryReceiptPart {
            context_reset: false,
            offset,
            text: receipt.to_owned(),
        }]
    );
}

pub(crate) const OPENCODE_ROOT: &str = "ses_0a1b2c3d4e5f60718293a4b5c6";

fn opencode_writer(home: &Path) -> rusqlite::Connection {
    rusqlite::Connection::open(home.join(".local/share/opencode/opencode.db")).unwrap()
}

/// OpenCode's assistant message as 1.18.30 writes it: `completed` absent
/// while it is still being written.
fn opencode_assistant(writer: &rusqlite::Connection, id: &str, at: u64, completed: bool) {
    let time = if completed {
        format!(r#"{{"created":{at},"completed":{}}}"#, at + 1)
    } else {
        format!(r#"{{"created":{at}}}"#)
    };
    writer
        .execute(
            // OpenCode updates a message in place; a replacement would
            // drop its parts with it.
            "INSERT INTO message VALUES (?1, ?2, ?3, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            rusqlite::params![
                id,
                OPENCODE_ROOT,
                at,
                format!(r#"{{"role":"assistant","mode":"build","time":{time}}}"#)
            ],
        )
        .unwrap();
}

/// OpenCode's `question` tool part in `status`, as its question tool writes it.
fn opencode_question(writer: &rusqlite::Connection, message: &str, at: u64, status: &str) {
    let input = serde_json::json!({"questions": [{
        "question": "어느 브랜치에 올릴까요?",
        "header": "Branch",
        "options": [
            {"label": "main", "description": "기본 브랜치"},
            {"label": "release", "description": "배포 브랜치"}
        ]
    }]});
    let state = match status {
        // OpenCode streams the call's input before it runs it.
        "pending" => serde_json::json!({"status": "pending", "input": {}, "raw": ""}),
        "running" => {
            serde_json::json!({"status": "running", "input": input, "time": {"start": at}})
        }
        "completed" => serde_json::json!({"status": "completed", "input": input,
            "output": "User has answered your questions: main", "metadata": {}, "title": "Asked 1 question",
            "time": {"start": at, "end": at + 5}}),
        _ => serde_json::json!({"status": "error", "input": input, "error": "dismissed",
            "time": {"start": at, "end": at + 5}}),
    };
    let part = serde_json::json!({"type": "tool", "tool": "question", "callID": "call_q1", "state": state});
    writer
        .execute(
            "INSERT INTO part VALUES ('prt_q1', ?1, ?2, ?3, ?3, ?4) \
             ON CONFLICT(id) DO UPDATE SET data = excluded.data",
            rusqlite::params![message, OPENCODE_ROOT, at, part.to_string()],
        )
        .unwrap();
}

fn opencode_continued(answer: &LabelTranscript) -> LabelTranscriptRequest {
    let mut next = request(Agent::OpenCode);
    next.checkpoint = Some(answer.checkpoint.clone());
    next.turns = answer.turns.clone();
    next
}

#[test]
fn an_opencode_question_waits_with_its_text_and_choices_until_it_is_answered() {
    use hide_session::turns::UserTurnKind;
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    assert_eq!(
        first.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );

    // The question waits while its message is unfinished.
    opencode_assistant(&writer, "msg_07", START + 400_000, false);
    opencode_question(&writer, "msg_07", START + 400_100, "running");
    let asking = read(home.path(), &opencode_continued(&first)).unwrap();
    assert!(
        asking.events.is_empty(),
        "the unfinished message is no event yet"
    );
    let fact = asking.turns.as_ref().unwrap().user_turn().unwrap();
    assert_eq!(fact.kind, UserTurnKind::Question);
    let content = serde_json::to_value(fact.content.unwrap())
        .unwrap()
        .to_string();
    assert!(content.contains("어느 브랜치에 올릴까요?"), "{content}");
    assert!(content.contains("release"), "{content}");

    // An idle reread of the same unfinished message keeps the same wait.
    let again = read(home.path(), &opencode_continued(&asking)).unwrap();
    assert_eq!(
        again.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );

    // The operator answers: the part completes and OpenCode finishes the message.
    opencode_question(&writer, "msg_07", START + 400_100, "completed");
    opencode_assistant(&writer, "msg_07", START + 400_000, true);
    let answered = read(home.path(), &opencode_continued(&again)).unwrap();
    assert_eq!(
        answered.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
    assert!(answered.turns.as_ref().unwrap().user_turn().is_none());
}

#[test]
fn a_pending_opencode_question_is_asked_with_its_choices_once_it_runs() {
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    opencode_assistant(&writer, "msg_07", START + 400_000, false);
    opencode_question(&writer, "msg_07", START + 400_100, "pending");
    let streaming = read(home.path(), &opencode_continued(&first)).unwrap();
    assert_eq!(
        streaming.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
    opencode_question(&writer, "msg_07", START + 400_100, "running");
    let asking = read(home.path(), &opencode_continued(&streaming)).unwrap();
    let fact = asking.turns.as_ref().unwrap().user_turn().unwrap();
    let content = serde_json::to_value(fact.content.unwrap())
        .unwrap()
        .to_string();
    assert!(content.contains("release"), "{content}");
}

#[test]
fn more_opencode_questions_than_the_cap_refuse_the_read_rather_than_miss_one() {
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    opencode_assistant(&writer, "msg_07", START + 400_000, false);
    let part = serde_json::json!({"type": "tool", "tool": "question", "callID": "call_q",
        "state": {"status": "running", "input": {"questions": []}, "time": {"start": START}}});
    for index in 0..=hide_session::turns::QUESTION_CALL_LIMIT {
        let mut part = part.clone();
        part["callID"] = serde_json::json!(format!("call_q{index}"));
        writer
            .execute(
                "INSERT INTO part VALUES (?1, 'msg_07', ?2, ?3, ?3, ?4)",
                rusqlite::params![
                    format!("prt_q{index:02}"),
                    OPENCODE_ROOT,
                    START + 400_100 + index as u64,
                    part.to_string()
                ],
            )
            .unwrap();
    }
    assert_eq!(
        read(home.path(), &request(Agent::OpenCode)).unwrap_err(),
        hide_session::ReadRefusal::Refused("user_turn_capacity".into())
    );
}

/// A `part` row of `message` holding `data` padded by `padding` bytes.
fn opencode_padded_part(
    writer: &rusqlite::Connection,
    id: &str,
    message: &str,
    at: u64,
    mut data: serde_json::Value,
    padding: usize,
) {
    data["padding"] = serde_json::json!("x".repeat(padding));
    writer
        .execute(
            "INSERT INTO part VALUES (?1, ?2, ?3, ?4, ?4, ?5)",
            rusqlite::params![id, message, OPENCODE_ROOT, at, data.to_string()],
        )
        .unwrap();
}

fn opencode_running_question() -> serde_json::Value {
    serde_json::json!({"type": "tool", "tool": "question", "callID": "call_big",
        "state": {"status": "running", "time": {"start": START},
            "input": {"questions": [{"question": "어느 브랜치에 올릴까요?",
                "options": [{"label": "main"}, {"label": "release"}]}]}}})
}

#[test]
fn an_oversized_part_beside_an_opencode_question_is_skipped_and_the_question_still_asks() {
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    opencode_assistant(&writer, "msg_07", START + 400_000, false);
    opencode_padded_part(
        &writer,
        "prt_big",
        "msg_07",
        START + 400_050,
        serde_json::json!({"type": "tool", "tool": "bash", "callID": "call_bash"}),
        hide_session::SESSION_LINE_LIMIT_BYTES,
    );
    opencode_question(&writer, "msg_07", START + 400_100, "running");
    let asking = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(
        asking.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );
    assert_eq!(asking.skipped_reasons.get("part_capacity"), Some(&1));
}

#[test]
fn an_opencode_question_over_the_row_limit_is_skipped_as_part_capacity_not_loaded() {
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    opencode_assistant(&writer, "msg_07", START + 400_000, false);
    opencode_padded_part(
        &writer,
        "prt_big",
        "msg_07",
        START + 400_100,
        opencode_running_question(),
        hide_session::SESSION_LINE_LIMIT_BYTES,
    );
    let read = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(read.skipped_reasons.get("part_capacity"), Some(&1));
    assert_eq!(
        read.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
}

#[test]
fn an_opencode_question_past_the_read_budget_waits_for_the_next_read_and_then_asks() {
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    // Four finished messages spend nearly the whole read budget, each part
    // under the row limit, so the unfinished question no longer fits.
    let part = hide_session::SESSION_LINE_LIMIT_BYTES - 6 * 1024;
    for index in 0..4u64 {
        let message = format!("msg_07{index}");
        let at = START + 400_000 + index * 10;
        opencode_assistant(&writer, &message, at, true);
        opencode_padded_part(
            &writer,
            &format!("prt_fill{index}"),
            &message,
            at + 1,
            serde_json::json!({"type": "text", "text": "filler"}),
            part,
        );
    }
    opencode_assistant(&writer, "msg_08", START + 400_100, false);
    opencode_padded_part(
        &writer,
        "prt_big",
        "msg_08",
        START + 400_110,
        opencode_running_question(),
        64 * 1024,
    );

    let first = read(home.path(), &request(Agent::OpenCode)).unwrap();
    assert!(first.has_more);
    assert_eq!(
        first.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
    let next = read(home.path(), &opencode_continued(&first)).unwrap();
    assert_eq!(
        next.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );
}

#[test]
fn a_dismissed_opencode_question_ends_the_wait_and_a_read_from_the_start_agrees() {
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    opencode_assistant(&writer, "msg_07", START + 400_000, false);
    opencode_question(&writer, "msg_07", START + 400_100, "running");
    let asking = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(
        asking.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Question)
    );
    opencode_question(&writer, "msg_07", START + 400_100, "error");
    opencode_assistant(&writer, "msg_07", START + 400_000, true);
    // A reader that lost its state folds the whole session to the same answer.
    let fresh = read_whole(home.path(), Agent::OpenCode);
    assert_eq!(
        fresh.turns.as_ref().unwrap().waiting(),
        Some(Waiting::Nothing)
    );
}

#[test]
fn an_unknown_opencode_question_state_refuses_the_read_rather_than_guessing() {
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    opencode_assistant(&writer, "msg_07", START + 400_000, true);
    opencode_question(&writer, "msg_07", START + 400_100, "running");
    writer
        .execute(
            "UPDATE part SET data = json_set(data, '$.state.status', 'future') WHERE id = 'prt_q1'",
            [],
        )
        .unwrap();
    assert_eq!(
        read(home.path(), &request(Agent::OpenCode)).unwrap_err(),
        hide_session::ReadRefusal::Refused("user_turn_invalid".into())
    );
}

#[test]
fn an_opencode_child_session_is_never_read_into_or_as_the_root_conversation() {
    let home = home(Agent::OpenCode);
    let root = read_whole(home.path(), Agent::OpenCode);
    assert!(
        root.events
            .iter()
            .all(|event| !event.text.contains("subagent")),
        "the child's records stay out of the root"
    );
    assert!(sighted(&root, 77).is_none());
    assert_eq!(root.title.as_deref(), Some("요청 보기 만들기"));
    let mut child = request(Agent::OpenCode);
    child.reference_value = "ses_0a1b2c3d4e5f60718293child".into();
    assert_eq!(
        read(home.path(), &child).unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_not_root".into())
    );
}

#[test]
fn an_opencode_session_proves_its_checkout_and_refuses_another_one() {
    let home = home(Agent::OpenCode);
    let mut other = request(Agent::OpenCode);
    other.cwd = Some("/work/other".into());
    assert_eq!(
        read(home.path(), &other).unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_cwd_mismatch".into())
    );
    other.cwd = None;
    assert_eq!(
        read(home.path(), &other).unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_cwd_unconfirmed".into())
    );
    other.cwd = Some("/work/app/".into());
    let confirmed = read(home.path(), &other).unwrap().confirmed;
    assert_eq!(confirmed.native_session_id.as_deref(), Some(OPENCODE_ROOT));
    assert_eq!(confirmed.incarnation, "opencode:1790989200000");
    let mut path = request(Agent::OpenCode);
    path.reference_kind = "path".into();
    path.reference_value = "/work/app/.opencode/session".into();
    assert_eq!(
        read(home.path(), &path).unwrap_err(),
        hide_session::ReadRefusal::Refused("session_kind_unsupported".into())
    );
}

#[cfg(unix)]
#[test]
fn a_linked_opencode_database_is_refused() {
    let home = home(Agent::OpenCode);
    let folder = home.path().join(".local/share/opencode");
    std::fs::rename(folder.join("opencode.db"), home.path().join("elsewhere.db")).unwrap();
    std::os::unix::fs::symlink(home.path().join("elsewhere.db"), folder.join("opencode.db"))
        .unwrap();
    assert_eq!(
        read(home.path(), &request(Agent::OpenCode)).unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_linked".into())
    );
}

#[test]
fn a_skipped_opencode_message_still_witnesses_where_a_read_ended() {
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    // A message too large to load ends the session: its id still witnesses it.
    writer
        .execute(
            "INSERT INTO message VALUES ('msg_07', ?1, ?2, ?2, ?3)",
            rusqlite::params![
                OPENCODE_ROOT,
                START + 140_000,
                format!(
                    r#"{{"role":"user","time":{{"created":{}}},"pad":"{}"}}"#,
                    START + 140_000,
                    "x".repeat(70 * 1024)
                )
            ],
        )
        .unwrap();
    let whole = read_whole(home.path(), Agent::OpenCode);
    assert!(whole.skipped_reasons.contains_key("message_capacity"));
    let held =
        hide_session::opencode::holds(home.path(), OPENCODE_ROOT, "/work/app", &whole.checkpoint);
    assert!(held.is_ok(), "{held:?}");
    // Once that message is gone the same checkpoint no longer holds.
    writer
        .execute("DELETE FROM message WHERE id = 'msg_07'", [])
        .unwrap();
    assert!(
        hide_session::opencode::holds(home.path(), OPENCODE_ROOT, "/work/app", &whole.checkpoint)
            .is_err()
    );
}

#[test]
fn an_opencode_session_rewound_and_grown_back_is_read_again_not_continued() {
    let home = home(Agent::OpenCode);
    let first = read_whole(home.path(), Agent::OpenCode);
    let writer = opencode_writer(home.path());
    // An OpenCode revert drops the last message and a new one takes its place.
    writer
        .execute_batch("DELETE FROM part WHERE message_id = 'msg_06'; DELETE FROM message WHERE id = 'msg_06';")
        .unwrap();
    writer
        .execute(
            "INSERT INTO message VALUES ('msg_06b', ?1, ?2, ?2, ?3)",
            rusqlite::params![
                OPENCODE_ROOT,
                START + 130_000,
                r#"{"role":"user","time":{"created":1790989330000}}"#
            ],
        )
        .unwrap();
    writer
        .execute(
            "INSERT INTO part VALUES ('prt_06b', 'msg_06b', ?1, ?2, ?2, ?3)",
            rusqlite::params![
                OPENCODE_ROOT,
                START + 130_000,
                r#"{"type":"text","text":"다른 방향으로 다시 해줘"}"#
            ],
        )
        .unwrap();
    let again = read(home.path(), &opencode_continued(&first)).unwrap();
    assert_eq!(again.rescanned.as_deref(), Some("replaced"));
    assert_eq!(
        again.events.first().map(|event| event.text.as_str()),
        Some("요청 보기를 만들어줘\n긴 요청의 둘째 줄")
    );
    assert!(
        again
            .events
            .iter()
            .any(|event| event.text == "다른 방향으로 다시 해줘")
    );
}

#[test]
fn opencode_conversation_page_is_its_label_read_by_message_with_receipts_as_injected_context() {
    use hide_session::{EventKind, SessionReadScope};
    let home = home(Agent::OpenCode);
    let writer = opencode_writer(home.path());
    // Hide's plugin stores a receipt in a synthetic part of the person's
    // message; another tool's context arrives as a person's message of its
    // own.
    writer
        .execute(
            "INSERT INTO part VALUES ('prt_hide', 'msg_01', ?1, 1790989200001, 1790989200001, ?2)",
            rusqlite::params![
                OPENCODE_ROOT,
                serde_json::json!({"type": "text", "synthetic": true,
                    "text": "<system-reminder>\n<hide-memory-receipt event=\"UserPromptSubmit\" count=\"1\" items=\"m1@1\" auth=\"a\" />\n</system-reminder>"})
                .to_string()
            ],
        )
        .unwrap();
    writer
        .execute(
            "INSERT INTO message VALUES ('msg_07', ?1, 1790989400000, 1790989400000, ?2)",
            rusqlite::params![
                OPENCODE_ROOT,
                r#"{"role":"user","time":{"created":1790989400000}}"#
            ],
        )
        .unwrap();
    writer
        .execute(
            "INSERT INTO part VALUES ('prt_ctx', 'msg_07', ?1, 1790989400000, 1790989400000, ?2)",
            rusqlite::params![
                OPENCODE_ROOT,
                serde_json::json!({"type": "text", "text": "<system-reminder>Background task finished</system-reminder>"})
                    .to_string()
            ],
        )
        .unwrap();
    let scope = |cwd: &str| SessionReadScope {
        id: OPENCODE_ROOT.to_owned(),
        cwd: cwd.to_owned(),
    };

    let page =
        hide_session::opencode::read_conversation(home.path(), &scope("/work/app"), None).unwrap();
    let events = page
        .events
        .iter()
        .zip(&page.event_offsets)
        .map(|(event, offset)| (*offset, event.kind, event.is_provider_injected()))
        .collect::<Vec<_>>();
    assert_eq!(
        events,
        [
            (0, EventKind::Human, false),
            (0, EventKind::Injected, true),
            (3, EventKind::Human, false),
            (4, EventKind::Human, false),
            (5, EventKind::Assistant, false),
            (6, EventKind::Injected, false),
        ]
    );
    assert!(page.events[1].text.starts_with("<hide-memory-receipt "));
    assert_eq!(page.events[1].at_unix_ms, 1_790_989_200_000);
    assert_eq!(page.events[3].images, 1);
    assert_eq!((page.checkpoint.offset(), page.has_more), (7, false));
    assert!(page.read_bytes > 0 && !page.rescanned);

    // The same messages, at the same indexes, as the label read reports them.
    let label = read(home.path(), &request(Agent::OpenCode)).unwrap();
    assert_eq!(
        label
            .events
            .iter()
            .map(|event| event.offset)
            .collect::<Vec<_>>(),
        [0, 3, 4, 5, 6]
    );
    assert_eq!(label.memory_receipts[0].offset, 0);

    // Read on from its checkpoint, nothing new is there.
    let next = hide_session::opencode::read_conversation(
        home.path(),
        &scope("/work/app"),
        Some(page.checkpoint),
    )
    .unwrap();
    assert!(next.events.is_empty());
    assert_eq!(next.checkpoint.offset(), 7);

    assert_eq!(
        hide_session::opencode::read_conversation(home.path(), &scope("/work/other"), None)
            .unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_cwd_mismatch".into())
    );
}

#[test]
fn opencode_activity_answers_its_newest_write_and_count_only_for_its_proven_owner() {
    use hide_session::session_activity::{SessionActivityRequest, read as activity};
    let home = home(Agent::OpenCode);
    let request = |cwd: &str| SessionActivityRequest {
        agent: Agent::OpenCode,
        reference_kind: "id".into(),
        reference_value: OPENCODE_ROOT.into(),
        cwd: Some(cwd.into()),
        exact_route: true,
        expected_id: Some(OPENCODE_ROOT.into()),
    };
    let answer = activity(home.path(), &request("/work/app")).unwrap();
    assert_eq!(answer.bytes, 6);
    assert_eq!(answer.modified_at_unix_ms, 1_790_989_331_000);
    assert_eq!(
        activity(home.path(), &request("/work/other")).unwrap_err(),
        hide_session::ReadRefusal::Refused("label_session_cwd_mismatch".into())
    );
    let mut changed = request("/work/app");
    changed.expected_id = Some("ses_other".into());
    assert_eq!(
        activity(home.path(), &changed).unwrap_err(),
        hide_session::ReadRefusal::Refused("session_route_owner_changed".into())
    );
    let value = serde_json::to_string(&answer).unwrap();
    assert!(!value.contains(OPENCODE_ROOT) && !value.contains("/work/app"));
}

#[test]
fn opencode_catalog_holds_only_its_root_sessions_in_the_project() {
    use hide_session::SessionCatalog;
    let home = home(Agent::OpenCode);
    let checkout = home.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    let checkout = std::fs::canonicalize(checkout).unwrap();
    let writer = opencode_writer(home.path());
    writer
        .execute(
            "UPDATE session SET directory = ?1",
            [checkout.display().to_string()],
        )
        .unwrap();
    writer
        .execute(
            "INSERT INTO session (id, project_id, slug, directory, title, version, \
             time_created, time_updated) VALUES ('ses_elsewhere', 'prj_1', 'far', \
             '/elsewhere/app', 'elsewhere', '1.18.30', 1, 1)",
            [],
        )
        .unwrap();
    let project = hide_project::resolve(&checkout, "local").unwrap();

    let sessions = SessionCatalog::new(home.path(), "local")
        .project_sessions(&project)
        .unwrap();
    assert!(sessions.refusals.is_empty(), "{:?}", sessions.refusals);
    let sessions = sessions.sessions;
    assert_eq!(sessions.len(), 1, "{sessions:?}");
    let session = &sessions[0];
    assert_eq!(session.id, OPENCODE_ROOT);
    assert_eq!(session.agent, Agent::OpenCode);
    assert_eq!(
        session.locator,
        PathBuf::from(format!("opencode/{OPENCODE_ROOT}"))
    );
    assert_eq!(session.title.as_deref(), Some("요청 보기 만들기"));
    assert!(
        session
            .first_human_request
            .as_deref()
            .is_some_and(|text| text.starts_with("요청 보기를 만들어줘")),
        "{session:?}"
    );
}

#[test]
fn original_issue_turn_is_scoped_and_bound_to_the_end_witness_for_native_adapters() {
    use hide_session::label_transcript::{WorkTurnRequest, read_work_turn, work_turn};
    for agent in [Agent::Claude, Agent::Codex, Agent::OpenCode] {
        let home = home(agent);
        let transcript = read_whole(home.path(), agent);
        let first = transcript
            .events
            .iter()
            .find(|event| event.kind == LabelEventKind::Human)
            .unwrap();
        let original = work_turn(&transcript.events, first.offset).unwrap();
        let request = WorkTurnRequest {
            agent,
            session_id: request(agent).reference_value,
            path: (agent == Agent::OpenCode).then(|| "opencode/catalog-locator".into()),
            cwd: "/work/app".into(),
            offset: first.offset,
            witness: original.witness(),
        };
        assert_eq!(
            read_work_turn(home.path(), &request).unwrap(),
            original,
            "{agent:?}"
        );
        let mut changed = request;
        changed.witness = hide_session::label_transcript::WorkTurn {
            request: "another turn".into(),
            assistant: original.assistant,
        }
        .witness();
        assert_eq!(
            read_work_turn(home.path(), &changed).unwrap_err(),
            "work_turn_changed",
            "{agent:?}"
        );
    }
}

#[test]
fn original_turn_rejects_a_same_id_file_replacement_before_preview_and_accepts_append() {
    use hide_session::label_transcript::{WorkTurnRequest, read_work_turn, work_turn};
    let home = home(Agent::Codex);
    let transcript = read_whole(home.path(), Agent::Codex);
    let first = transcript
        .events
        .iter()
        .find(|event| event.kind == LabelEventKind::Human)
        .unwrap();
    let original = work_turn(&transcript.events, first.offset).unwrap();
    let path = home.path().join(".codex/sessions/2026/10/03/rollout-2026-10-03T01-00-00-0199a000-0000-7000-8000-000000000002.jsonl");
    let request = WorkTurnRequest {
        agent: Agent::Codex,
        session_id: request(Agent::Codex).reference_value,
        path: Some(path.to_string_lossy().into_owned()),
        cwd: "/work/app".into(),
        offset: first.offset,
        witness: original.witness(),
    };
    use std::io::Write;
    std::fs::OpenOptions::new().append(true).open(&path).unwrap().write_all(b"\n{\"type\":\"event_msg\",\"timestamp\":\"2026-10-03T01:01:00Z\",\"payload\":{\"type\":\"user_message\",\"message\":\"Later turn\"}}\n").unwrap();
    assert_eq!(read_work_turn(home.path(), &request).unwrap(), original);
    let content = std::fs::read_to_string(&path).unwrap();
    let changed = content.replacen("요청 보기를", "요청 요기를", 1);
    assert_ne!(
        changed, content,
        "fixture must replace the original request at unchanged byte boundaries"
    );
    let replacement = path.with_extension("replacement");
    std::fs::write(&replacement, changed).unwrap();
    std::fs::rename(replacement, &path).unwrap();
    assert_eq!(
        read_work_turn(home.path(), &request).unwrap_err(),
        "work_turn_changed"
    );
}

/// A Codex 0.163.0 session started with `/goal` (PRD
/// label-goal-command-context D-04-D-06): Codex records the operator's
/// objective in a `user_goal` message, adds its own `goal` reminder to
/// every turn while the goal is active, and reports a cleared goal in another
/// `user_goal` message that carries no words of the operator's.
#[test]
fn a_codex_goal_is_the_operators_request_and_its_turn_reminder_is_not() {
    let home = tempfile::tempdir().unwrap();
    copy_tree(
        &fixtures().join("codex-0.163.0-goal"),
        &home.path().join(".codex"),
    );
    let request = LabelTranscriptRequest {
        reference_value: "0199a000-0000-7000-8000-0000000000b1".to_owned(),
        ..request(Agent::Codex)
    };
    let transcript = read(home.path(), &request).unwrap();
    let humans = transcript
        .events
        .iter()
        .filter(|event| event.kind == LabelEventKind::Human)
        .map(|event| event.text.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        humans,
        ["/goal 출시 전 검수 리포트를 \"확실하게\" 정리해줘\n\n에이전트별로 나눠서"],
        "the objective, unwrapped and decoded, is the one request, marked as the goal command"
    );
    assert!(
        transcript
            .events
            .iter()
            .all(|event| !event.text.contains("codex_internal_context")),
        "neither Codex wrapper reaches the conversation as words"
    );
}

const AGY_ID: &str = "a9c0e2f4-0000-4000-8000-000000000008";

mod antigravity {
    use super::*;
    use hide_session::{SessionIdentity, SessionLocator};
    use std::fs;

    /// A subagent agy started from the fixture's conversation.
    const CHILD: &str = "a9c0e2f4-0000-4000-8000-000000000009";

    struct Native {
        home: tempfile::TempDir,
        cwd: PathBuf,
        request: LabelTranscriptRequest,
    }

    impl Native {
        fn new() -> Self {
            let home = tempfile::tempdir().unwrap();
            let cwd = home.path().join("checkout 1");
            fs::create_dir(&cwd).unwrap();
            let cwd = fs::canonicalize(cwd).unwrap();
            let folder = home.path().join(".gemini/antigravity-cli");
            fs::create_dir_all(&folder).unwrap();
            let database =
                rusqlite::Connection::open(folder.join("conversation_summaries.db")).unwrap();
            database
                .execute_batch(
                    &fs::read_to_string(fixtures().join("antigravity-1.3.6/summaries.sql"))
                        .unwrap(),
                )
                .unwrap();
            drop(database);
            let native = Self {
                request: LabelTranscriptRequest {
                    cwd: Some(cwd.display().to_string()),
                    ..request(Agent::Antigravity)
                },
                home,
                cwd,
            };
            native.conversation(AGY_ID, "", "2026-10-03 01:02:09+00:00");
            native
        }

        /// agy writes a conversation's transcript, its store and its index row.
        fn conversation(&self, id: &str, parent: &str, modified: &str) {
            let transcript = hide_session::antigravity::transcript(self.home.path(), id);
            fs::create_dir_all(transcript.parent().unwrap()).unwrap();
            fs::copy(
                fixtures().join("antigravity-1.3.6/transcript_full.jsonl"),
                &transcript,
            )
            .unwrap();
            let folder = self.home.path().join(".gemini/antigravity-cli");
            fs::create_dir_all(folder.join("conversations")).unwrap();
            fs::write(folder.join("conversations").join(format!("{id}.db")), b"").unwrap();
            // agy's spelling: `file:///work/repo`, or on Windows
            // `file:///C:/work/repo`.
            let wire = hide_platform::path::to_wire(&self.cwd).unwrap();
            let slash = if wire.starts_with('/') { "" } else { "/" };
            let uri = format!("file://{slash}{}", wire.replace(' ', "%20"));
            rusqlite::Connection::open(folder.join("conversation_summaries.db"))
                .unwrap()
                .execute(
                    "INSERT INTO conversation_summaries (conversation_id, title, last_modified_time, \
                     workspace_uris, parent_conversation_id, last_user_input_time, app_data_dir) \
                     VALUES (?1, '요청 보기 만들기', ?3, ?2, ?4, ?3, 'antigravity-cli')",
                    rusqlite::params![id, serde_json::json!([uri]).to_string(), modified, parent],
                )
                .unwrap();
        }

        fn route(&self, id: &str, cwd: &Path) -> Result<(), hide_session::ReadRefusal> {
            hide_session::session_activity::read(
                self.home.path(),
                &hide_session::session_activity::SessionActivityRequest {
                    agent: Agent::Antigravity,
                    reference_kind: "id".into(),
                    reference_value: id.into(),
                    cwd: cwd.to_str().map(str::to_owned),
                    exact_route: true,
                    expected_id: None,
                },
            )
            .map(|_| ())
        }
    }

    #[test]
    fn native_history_has_the_shared_conversation_facts() {
        let native = Native::new();
        let transcript = read(native.home.path(), &native.request).unwrap();
        assert_eq!(transcript.title.as_deref(), Some("요청 보기 만들기"));
        let people: Vec<_> = transcript
            .events
            .iter()
            .filter(|event| event.kind == LabelEventKind::Human)
            .collect();
        assert_eq!(people.len(), 1, "what a hook gave agy is no one's request");
        assert_eq!(people[0].text, "요청 보기를 만들어줘\n긴 요청의 둘째 줄");
        assert_eq!(people[0].at_unix_ms, START);
        let replies: Vec<_> = transcript
            .events
            .iter()
            .filter(|event| event.kind == LabelEventKind::Assistant)
            .collect();
        assert_eq!(replies.len(), 1, "a tool call step is no answer");
        assert!(replies[0].text.starts_with("PR을 열었습니다"));
        assert_eq!(replies[0].at_unix_ms, START + 129_000);
        assert_eq!(sighted(&transcript, 12).unwrap().at_unix_ms, START + 30_000);
        assert!(sighted(&transcript, 99).is_none());
        assert_eq!(
            transcript.confirmed.native_session_id.as_deref(),
            Some(AGY_ID)
        );
    }

    #[test]
    fn a_subagents_conversation_is_never_an_operators_session() {
        let native = Native::new();
        native.conversation(CHILD, AGY_ID, "2026-10-03 01:03:00+00:00");
        let child = LabelTranscriptRequest {
            reference_value: CHILD.into(),
            ..native.request.clone()
        };
        assert!(read(native.home.path(), &child).is_err());
        // The newest conversation in the checkout is the subagent's; a pane
        // with no reported session still reads its operator's.
        let located = SessionLocator::new(native.home.path())
            .locate("pane", Agent::Antigravity, None, native.cwd.to_str())
            .unwrap();
        assert_eq!(
            located,
            hide_session::antigravity::transcript(native.home.path(), AGY_ID)
        );
    }

    #[test]
    fn a_conversation_routes_only_from_its_own_checkout_while_agy_keeps_it() {
        let native = Native::new();
        native.route(AGY_ID, &native.cwd).unwrap();
        let other = native.home.path().join("other");
        fs::create_dir(&other).unwrap();
        assert!(native.route(AGY_ID, &other).is_err());
        fs::remove_file(
            native
                .home
                .path()
                .join(".gemini/antigravity-cli/conversations")
                .join(format!("{AGY_ID}.db")),
        )
        .unwrap();
        assert_eq!(
            native.route(AGY_ID, &native.cwd).unwrap_err(),
            hide_session::ReadRefusal::Refused("session_route_missing".into())
        );
    }

    /// A route proof that meets agy writing its index says so, and a read a
    /// moment later proves the route: Fork, Sleep and resume read it again
    /// rather than refuse a conversation agy still keeps.
    #[test]
    fn a_route_proof_that_meets_agy_writing_its_index_answers_once_agy_is_done() {
        let native = Native::new();
        let journal = native
            .home
            .path()
            .join(".gemini/antigravity-cli/conversation_summaries.db-journal");
        fs::write(&journal, b"agy writing its index").unwrap();
        assert_eq!(
            native.route(AGY_ID, &native.cwd),
            Err(hide_session::ReadRefusal::InMotion("antigravity_db_busy"))
        );
        fs::remove_file(&journal).unwrap();
        native.route(AGY_ID, &native.cwd).unwrap();
    }

    #[test]
    fn a_fork_is_the_one_new_root_conversation_at_the_source_step_in_its_checkout() {
        let native = Native::new();
        let cwd = hide_platform::fs::identity::canonical(&native.cwd).unwrap();
        let index = rusqlite::Connection::open(
            native
                .home
                .path()
                .join(".gemini/antigravity-cli/conversation_summaries.db"),
        )
        .unwrap();
        let steps = |id: &str, count: i64| {
            index
                .execute(
                    "UPDATE conversation_summaries SET step_count = ?2 WHERE conversation_id = ?1",
                    rusqlite::params![id, count],
                )
                .unwrap();
        };
        steps(AGY_ID, 7);
        let older = "a9c0e2f4-0000-4000-8000-00000000000b";
        native.conversation(older, "", "2026-10-03 01:02:30+00:00");
        steps(older, 7);
        let mark = hide_session::antigravity::fork_mark(native.home.path(), AGY_ID, &cwd).unwrap();
        assert_eq!(mark.steps, 7);
        let forked = || {
            hide_session::antigravity::forked(native.home.path(), AGY_ID, &cwd, &mark)
                .map_err(|error| error.to_string())
        };
        assert_eq!(
            forked().unwrap(),
            None,
            "neither the source nor a conversation already there is the fork"
        );
        let other_step = "a9c0e2f4-0000-4000-8000-00000000000c";
        native.conversation(other_step, "", "2026-10-03 01:04:00+00:00");
        steps(other_step, 3);
        // A fork of a longer conversation, or a new one that grew past the
        // source, lands in the same checkout first.
        let longer = "a9c0e2f4-0000-4000-8000-000000000010";
        native.conversation(longer, "", "2026-10-03 01:04:30+00:00");
        steps(longer, 50);
        assert_eq!(
            forked().unwrap(),
            None,
            "a row past the source's steps is not its fork"
        );
        // agy stamps the fork with the node's clock, which may be behind the
        // core's, and the source may take a step while agy writes it.
        let fork = "a9c0e2f4-0000-4000-8000-00000000000d";
        native.conversation(fork, "", "2026-10-01 00:00:00+00:00");
        steps(fork, 7);
        steps(AGY_ID, 8);
        assert_eq!(forked().unwrap().as_deref(), Some(fork));
        let second = "a9c0e2f4-0000-4000-8000-00000000000e";
        native.conversation(second, "", "2026-10-03 01:06:00+00:00");
        steps(second, 7);
        assert_eq!(forked().unwrap_err(), "antigravity_fork_ambiguous");
        let oversized = hide_session::antigravity::ForkMark {
            known: vec![AGY_ID.to_owned(); 257],
            steps: 7,
        };
        assert_eq!(
            hide_session::antigravity::forked(native.home.path(), AGY_ID, &cwd, &oversized)
                .unwrap_err()
                .to_string(),
            "antigravity_fork_mark_invalid"
        );
        assert_eq!(
            hide_session::antigravity::fork_mark(
                native.home.path(),
                "a9c0e2f4-0000-4000-8000-00000000000f",
                &cwd
            )
            .unwrap_err()
            .to_string(),
            "session_route_missing"
        );
    }

    #[test]
    fn a_reported_conversation_the_index_does_not_hold_is_refused() {
        let native = Native::new();
        let missing = "a9c0e2f4-0000-4000-8000-00000000000a";
        let transcript = hide_session::antigravity::transcript(native.home.path(), missing);
        fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        fs::copy(
            fixtures().join("antigravity-1.3.6/transcript_full.jsonl"),
            &transcript,
        )
        .unwrap();
        let located = SessionLocator::new(native.home.path()).locate(
            "pane",
            Agent::Antigravity,
            Some(&SessionIdentity::id(missing)),
            native.cwd.to_str(),
        );
        assert!(located.is_err());
    }
}
