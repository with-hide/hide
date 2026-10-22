use hide_session::turns::{ToolTurnMark, TurnMark, TurnMode, TurnTracker, WakeLoss, WakeMark};
use hide_session::{
    Agent, ConversationCursor, EventKind, RescanReason, SESSION_LINE_LIMIT_BYTES, SkipReason,
};
use serde_json::json;
use std::fs;
use std::io::Write;

fn human(text: &str) -> String {
    format!(
        "{}\n",
        json!({
            "timestamp": "2026-09-30T00:00:00Z", "type": "response_item",
            "payload": {"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]}
        })
    )
}

#[test]
fn oversized_tool_record_does_not_hide_human_turns_or_their_offsets() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let first = human("작업 라벨을 고쳐줘");
    let tool = format!(
        "{}\n",
        json!({
            "type": "event_msg", "payload": {"type": "item_completed", "output": "x".repeat(SESSION_LINE_LIMIT_BYTES * 5)}
        })
    );
    let last = human("도구 실행 결과 이후에도 계속해줘");
    fs::write(&path, format!("{first}{tool}{last}")).unwrap();
    let mut reader = ConversationCursor::new();
    let mut events = Vec::new();
    let mut offsets = Vec::new();
    let mut oversized = 0;
    for poll in 0..3 {
        let parsed = reader.read(Agent::Codex, &path).unwrap();
        assert_eq!(reader.has_more(), poll == 0);
        events.extend(parsed.events);
        offsets.extend(parsed.event_offsets);
        oversized += parsed
            .skipped_reasons
            .get(&SkipReason::NonConversationCapacity)
            .copied()
            .unwrap_or(0);
    }
    assert_eq!(
        events
            .iter()
            .map(|event| event.text.as_str())
            .collect::<Vec<_>>(),
        ["작업 라벨을 고쳐줘", "도구 실행 결과 이후에도 계속해줘"]
    );
    assert!(events.iter().all(|event| event.kind == EventKind::Human));
    assert_eq!(offsets, [0, (first.len() + tool.len()) as u64]);
    assert_eq!(oversized, 1);
}

#[test]
fn split_oversized_tool_result_resumes_only_at_the_next_record() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let prefix =
        "{\"type\":\"response_item\",\"payload\":{\"type\":\"function_call_output\",\"output\":\"";
    fs::write(
        &path,
        format!("{prefix}{}", "x".repeat(SESSION_LINE_LIMIT_BYTES + 1)),
    )
    .unwrap();
    let mut reader = ConversationCursor::new();
    assert!(reader.read(Agent::Codex, &path).unwrap().events.is_empty());
    assert!(
        !reader.has_more(),
        "a torn record at EOF waits for an append"
    );
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap()
        .write_all(format!("\"}}}}\n{}", human("완료 결과를 확인해줘")).as_bytes())
        .unwrap();
    let parsed = reader.read(Agent::Codex, &path).unwrap();
    assert_eq!(parsed.events.len(), 1);
    assert_eq!(parsed.events[0].text, "완료 결과를 확인해줘");
}

#[test]
fn oversized_conversation_and_unreadable_records_lose_their_text_and_the_read_continues() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let huge = "x".repeat(SESSION_LINE_LIMIT_BYTES + 1);
    let claude_after = format!(
        "{}\n",
        json!({"type":"user","timestamp":"2026-10-09T00:00:00Z",
            "message":{"role":"user","content":"after the record"}})
    );
    let codex_after = human("after the record");
    let asked = TurnMark::Tools(vec![ToolTurnMark::Asked {
        call: "question-1".into(),
        content: None,
    }]);
    // (agent, record, what a lost text keeps of its turn)
    let records = [
        (Agent::Codex, human(&huge), None),
        // A person's message with a pasted screenshot.
        (
            Agent::Claude,
            format!(
                "{}\n",
                json!({"type":"user","message":{"role":"user","content":[
                    {"type":"text","text":"이 화면 봐줘"},
                    {"type":"image","source":{"type":"base64","media_type":"image/png","data":huge}}
                ]}})
            ),
            None,
        ),
        // Text beside a question keeps the question.
        (
            Agent::Claude,
            format!(
                "{}\n",
                json!({"type":"assistant","message":{"content":[
                    {"type":"text","text":huge},
                    {"type":"tool_use","id":"question-1","name":"AskUserQuestion","input":{}}
                ]}})
            ),
            Some(asked),
        ),
        // Nothing certifies these: what the turn waits for is not known.
        (
            Agent::Codex,
            format!("{huge}\n"),
            Some(TurnMark::Unreadable),
        ),
        (
            Agent::Claude,
            format!("{}\n", json!({"kind":"unknown","data":huge})),
            Some(TurnMark::Unreadable),
        ),
    ];
    for (agent, record, mark) in records {
        let after = match agent {
            Agent::Claude => &claude_after,
            _ => &codex_after,
        };
        fs::write(&path, format!("{record}{after}")).unwrap();
        let mut reader = ConversationCursor::new();
        let (mut marks, mut events, mut lost) = (Vec::new(), Vec::new(), 0);
        while {
            let parsed = reader.read(agent, &path).unwrap();
            marks.extend(parsed.turn_marks);
            events.extend(parsed.events);
            lost += parsed
                .skipped_reasons
                .get(&SkipReason::ConversationCapacity)
                .copied()
                .unwrap_or(0);
            reader.has_more()
        } {}
        let context = format!("{agent:?} {record:.80}");
        assert_eq!(lost, 1, "{context}");
        assert_eq!(
            marks
                .iter()
                .find(|(offset, _)| *offset == 0)
                .map(|(_, mark)| mark),
            mark.as_ref(),
            "{context}"
        );
        assert_eq!(
            events
                .iter()
                .map(|event| event.text.as_str())
                .collect::<Vec<_>>(),
            ["after the record"],
            "{context}"
        );
    }
}

#[test]
fn oversized_native_records_keep_their_turn_mark_without_their_body() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let huge = "x".repeat(SESSION_LINE_LIMIT_BYTES + 1);
    let asked = TurnMark::Tools(vec![ToolTurnMark::Asked {
        call: "question-1".into(),
        content: None,
    }]);
    let answered = TurnMark::Tools(vec![ToolTurnMark::Answered {
        call: "question-1".into(),
    }]);
    let turn = Some("turn-1".to_owned());
    let records = [
        (
            Agent::Claude,
            json!({"type":"assistant","message":{"content":[{
                "type":"tool_use","id":"question-1","name":"AskUserQuestion",
                "input":{"questions":[{"question":huge}]}
            }]}}),
            asked.clone(),
        ),
        (
            Agent::Claude,
            json!({"type":"user","message":{"content":[{
                "type":"tool_result","tool_use_id":"question-1","content":huge
            }]}}),
            answered.clone(),
        ),
        (
            Agent::Codex,
            json!({"type":"response_item","payload":{
                "type":"function_call","call_id":"question-1","name":"request_user_input",
                "arguments":json!({"questions":[{"question":huge}]}).to_string()
            }}),
            asked,
        ),
        (
            Agent::Codex,
            json!({"type":"response_item","payload":{
                "type":"function_call_output","call_id":"question-1","output":huge
            }}),
            answered,
        ),
        // How many questions an async call this large asked is not known.
        (
            Agent::Codex,
            json!({"type":"response_item","payload":{
                "type":"function_call","call_id":"question-1","name":"request_user_input_async",
                "arguments":json!({"questions":[{"title":huge}]}).to_string()
            }}),
            TurnMark::Unreadable,
        ),
        (
            Agent::Codex,
            json!({"type":"event_msg","payload":{
                "type":"item_completed","turn_id":"turn-1","item":{"type":"Plan","text":huge}
            }}),
            TurnMark::Plan { turn: turn.clone() },
        ),
        (
            Agent::Codex,
            json!({"type":"event_msg","payload":{
                "type":"task_started","turn_id":"turn-1","collaboration_mode_kind":"plan","extra":huge
            }}),
            TurnMark::Started {
                turn: turn.clone(),
                mode: TurnMode::Plan,
            },
        ),
        (
            Agent::Codex,
            json!({"type":"event_msg","payload":{
                "type":"task_complete","turn_id":"turn-1","extra":huge
            }}),
            TurnMark::Completed { turn: turn.clone() },
        ),
        (
            Agent::Codex,
            json!({"type":"event_msg","payload":{
                "type":"turn_aborted","turn_id":"turn-1","extra":huge
            }}),
            TurnMark::Aborted { turn },
        ),
    ];
    for (agent, record, mark) in records {
        let after = human("after native record");
        fs::write(&path, format!("{record}\n{after}")).unwrap();
        let mut reader = ConversationCursor::new();
        let mut marks = Vec::new();
        let mut oversized = 0;
        while {
            let parsed = reader.read(agent, &path).unwrap();
            marks.extend(parsed.turn_marks);
            oversized += parsed
                .skipped_reasons
                .get(&SkipReason::NonConversationCapacity)
                .copied()
                .unwrap_or(0);
            reader.has_more()
        } {}
        assert_eq!(marks.first(), Some(&(0, mark)), "{agent:?} {record:.120}");
        assert_eq!(oversized, 1, "{agent:?}");
        assert_eq!(
            reader.checkpoint().offset(),
            (record.to_string().len() + 1 + after.len()) as u64
        );
    }
}

#[test]
fn a_screenshot_tool_result_answers_its_call_and_the_read_continues() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    // Claude Code writes a screenshot inline: the call id comes first, the
    // image is base64 inside the block, and `toolUseResult` repeats it.
    let image = "A".repeat(SESSION_LINE_LIMIT_BYTES * 2);
    let blocks = json!([
        {"type":"text","text":"Computer Use state"},
        {"type":"image","source":{"type":"base64","media_type":"image/png","data":image}},
        {"type":"text","text":"Key window"}
    ]);
    let record = json!({
        "parentUuid":"p", "type":"user",
        "message":{"role":"user","content":[
            {"tool_use_id":"toolu_01Screenshot","type":"tool_result","content":blocks}
        ]},
        "toolUseResult":blocks
    });
    let after = json!({"type":"user","message":{"role":"user","content":"다음 단계로 가자"},
        "timestamp":"2026-10-09T00:00:00Z"});
    fs::write(&path, format!("{record}\n{after}\n")).unwrap();
    let mut reader = ConversationCursor::new();
    let (mut marks, mut events) = (Vec::new(), Vec::new());
    while {
        let parsed = reader.read(Agent::Claude, &path).unwrap();
        marks.extend(parsed.turn_marks);
        events.extend(parsed.events);
        reader.has_more()
    } {}
    assert_eq!(
        marks.first(),
        Some(&(
            0,
            TurnMark::Tools(vec![ToolTurnMark::Answered {
                call: "toolu_01Screenshot".into()
            }])
        ))
    );
    assert_eq!(events.last().unwrap().text, "다음 단계로 가자");
}

#[test]
fn a_split_native_question_keeps_its_call_across_a_checkpoint_restore() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    // Both native discriminators arrive after the discarded body, so the
    // scanner must finish the same bounded block after a durable restore.
    let complete = format!(
        "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"input\":{{\"questions\":[{{\"question\":\"{}\"}}]}},\"name\":\"AskUserQuestion\",\"type\":\"tool_use\",\"id\":\"question-1\"}}]}}}}\n",
        "x".repeat(SESSION_LINE_LIMIT_BYTES + 1)
    );
    let split = complete.find("\"name\"").unwrap();
    fs::write(&path, &complete[..split]).unwrap();
    let mut reader = ConversationCursor::new();
    assert!(
        reader
            .read(Agent::Claude, &path)
            .unwrap()
            .turn_marks
            .is_empty()
    );
    assert!(!reader.has_more(), "a torn record waits for its append");
    let checkpoint = serde_json::to_value(reader.checkpoint()).unwrap();
    fs::write(&path, &complete).unwrap();
    let asked = TurnMark::Tools(vec![ToolTurnMark::Asked {
        call: "question-1".into(),
        content: None,
    }]);
    // An older build's scan kept no ids: the record is reread from its start.
    for older_scan in [false, true] {
        let mut checkpoint = checkpoint.clone();
        if older_scan {
            let classifier = checkpoint["classifier"].as_object_mut().unwrap();
            classifier.remove("native_ids");
            classifier.insert("user_turn_scanned".into(), json!(true));
        }
        let mut restored = ConversationCursor::restore(serde_json::from_value(checkpoint).unwrap());
        let mut marks = Vec::new();
        while {
            marks.extend(restored.read(Agent::Claude, &path).unwrap().turn_marks);
            restored.has_more()
        } {}
        assert_eq!(marks, [(0, asked.clone())], "older scan: {older_scan}");
        assert_eq!(restored.checkpoint().offset(), complete.len() as u64);
    }
}

#[test]
fn replacement_during_discard_keeps_the_new_session_human_turn() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    fs::write(
        &path,
        format!(
            "{{\"type\":\"event_msg\",\"body\":\"{}",
            "x".repeat(SESSION_LINE_LIMIT_BYTES + 1)
        ),
    )
    .unwrap();
    let mut reader = ConversationCursor::new();
    reader.read(Agent::Codex, &path).unwrap();
    let replacement = directory.path().join("replacement.jsonl");
    fs::write(&replacement, human("새 세션 요청")).unwrap();
    fs::rename(replacement, &path).unwrap();
    let parsed = reader.read(Agent::Codex, &path).unwrap();
    assert_eq!(parsed.rescan_reason, Some(RescanReason::Replaced));
    assert_eq!(parsed.events[0].text, "새 세션 요청");
    assert_eq!(parsed.event_offsets, [0]);
}

#[test]
fn claude_progress_does_not_hide_its_title_or_human_turn() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let ignored = format!(
        "{{\"type\":\"progress\",\"data\":\"{}\"}}\n",
        "x".repeat(SESSION_LINE_LIMIT_BYTES + 1)
    );
    let title = "{\"type\":\"ai-title\",\"aiTitle\":\"설치 문제 확인\"}\n";
    let human = "{\"type\":\"user\",\"origin\":{\"kind\":\"human\"},\"timestamp\":\"2026-09-30T00:00:00Z\",\"message\":{\"content\":\"계속 확인해줘\"}}\n";
    fs::write(&path, format!("{ignored}{title}{human}")).unwrap();
    let parsed = ConversationCursor::new()
        .read(Agent::Claude, &path)
        .unwrap();
    assert_eq!(parsed.title.as_deref(), Some("설치 문제 확인"));
    assert_eq!(parsed.events[0].text, "계속 확인해줘");
    assert_eq!(parsed.event_offsets, [(ignored.len() + title.len()) as u64]);
}

/// A Claude Code `tool_use` block for `name` with `padding` after the name and
/// id, so a record is over the line cap while those two arrive last.
fn claude_call(name: &str, id: &str, padding: usize) -> String {
    format!(
        "{{\"type\":\"assistant\",\"message\":{{\"content\":[{{\"input\":{{\"command\":\"{}\"}},\"name\":\"{name}\",\"type\":\"tool_use\",\"id\":\"{id}\"}}]}}}}\n",
        "x".repeat(padding)
    )
}

/// Reads `path` to its end, saving the checkpoint as JSON and restoring it
/// between polls the way the label store does, and returns every mark in order
/// with the number of polls it took.
fn read_through_restores(path: &std::path::Path) -> (Vec<(u64, TurnMark)>, usize) {
    let mut reader = ConversationCursor::new();
    let mut marks = Vec::new();
    let mut polls = 0;
    loop {
        marks.extend(reader.read(Agent::Claude, path).unwrap().turn_marks);
        polls += 1;
        let saved = serde_json::to_value(reader.checkpoint()).unwrap();
        let more = reader.has_more();
        reader = ConversationCursor::restore(serde_json::from_value(saved).unwrap());
        if !more {
            return (marks, polls);
        }
    }
}

/// A checkpoint made by a build that did not keep what the wake reader needs
/// (`wake_aware`) cannot say which call a torn record named, so the record is
/// read again from its start, as one that kept no native ids is.
#[test]
fn a_scan_checkpointed_before_the_wake_reader_is_reread_from_its_record_start() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let complete = claude_call("Bash", "toolu_long", SESSION_LINE_LIMIT_BYTES + 1);
    // Both the tool's name and its id arrive after the discarded body.
    let split = complete.find("\"name\"").unwrap();
    fs::write(&path, &complete[..split]).unwrap();
    let mut reader = ConversationCursor::new();
    assert!(
        reader
            .read(Agent::Claude, &path)
            .unwrap()
            .turn_marks
            .is_empty()
    );
    let checkpoint = serde_json::to_value(reader.checkpoint()).unwrap();
    fs::write(&path, &complete).unwrap();
    let called = TurnMark::Wake(vec![WakeMark::Call {
        call: "toolu_long".into(),
    }]);
    for older_scan in [false, true] {
        let mut checkpoint = checkpoint.clone();
        if older_scan {
            let classifier = checkpoint["classifier"].as_object_mut().unwrap();
            assert_eq!(classifier.remove("wake_aware"), Some(json!(true)));
        }
        let mut restored = ConversationCursor::restore(serde_json::from_value(checkpoint).unwrap());
        let mut marks = Vec::new();
        while {
            marks.extend(restored.read(Agent::Claude, &path).unwrap().turn_marks);
            restored.has_more()
        } {}
        assert_eq!(marks, [(0, called.clone())], "older scan: {older_scan}");
        assert_eq!(restored.checkpoint().offset(), complete.len() as u64);
    }
}

/// The result of a command, longer than the poll budget, is read over several
/// polls and a restore between each; the call it answers is remembered by the
/// tracker, so the loss is the same as for a result read in one go.
#[test]
fn a_result_longer_than_a_poll_budget_still_loses_the_devices_of_its_open_call() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let record = |value: serde_json::Value| format!("{value}\n");
    let boot = record(
        json!({"type": "attachment", "attachment": {"type": "hook_success",
        "hookName": "SessionStart:startup"}}),
    );
    let started = record(
        json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "toolu_start",
            "content": "Command running in background with ID: bg1"}]}}),
    );
    let result = record(
        json!({"type": "user", "message": {"role": "user", "content": [
        {"type": "tool_result", "tool_use_id": "toolu_long",
            "content": "x".repeat(hide_session::SESSION_INCREMENT_READ_LIMIT_BYTES as usize * 5 / 2)}]}}),
    );
    fs::write(
        &path,
        format!(
            "{boot}{started}{}{result}",
            claude_call("Bash", "toolu_long", 0)
        ),
    )
    .unwrap();
    let (marks, polls) = read_through_restores(&path);
    assert!(polls >= 3, "the record spans polls: {polls}");
    let mut tracker = TurnTracker::default();
    for (offset, mark) in &marks {
        tracker.fold(*offset, mark);
    }
    assert_eq!(tracker.wake_loss(), Some(WakeLoss::Lost));
    assert!(tracker.wake_expiries().is_empty());

    // The same session with a short result of that call loses nothing, so the
    // loss above is the long result's.
    fs::write(
        &path,
        format!(
            "{boot}{started}{}{}",
            claude_call("Bash", "toolu_long", 0),
            record(
                json!({"type": "user", "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "toolu_long", "content": "done"}]}})
            )
        ),
    )
    .unwrap();
    let (marks, _) = read_through_restores(&path);
    let mut tracker = TurnTracker::default();
    for (offset, mark) in &marks {
        tracker.fold(*offset, mark);
    }
    assert_eq!(tracker.wake_loss(), None);
    assert_eq!(tracker.wake_expiries().len(), 1);
}

const CLAUDE_COMMANDS: &str = include_str!("../fixtures/claude-commands.jsonl");

/// What the operator asked, in order: a command answered by Claude Code itself
/// (`/model`, `/usage`) and its output are not requests; a command sent to the
/// model is one, whatever its name, and so is one Claude Code answers and then
/// hands to the model, marking its answer as the operator's turn (`/goal`).
const COMMAND_REQUESTS: [&str; 5] = [
    "설치하고 재측정해줘",
    "글 정리해줘",
    "/ship",
    "/goal 출시 전에 검수 리포트 정리해줘",
    "마지막 요청",
];

fn humans(events: &[hide_session::ConversationEvent]) -> Vec<&str> {
    events
        .iter()
        .filter(|event| event.kind == EventKind::Human)
        .map(|event| event.text.as_str())
        .collect()
}

#[test]
fn a_command_claude_code_answered_itself_is_not_a_request() {
    let parsed = hide_session::parse_claude_events(CLAUDE_COMMANDS);
    assert_eq!(humans(&parsed.events), COMMAND_REQUESTS);
}

#[test]
fn a_command_and_its_reply_in_different_polls_is_read_once_either_way() {
    let lines: Vec<&str> = CLAUDE_COMMANDS.split_inclusive('\n').collect();
    // Every poll boundary: the file is first written up to `cut` lines, read,
    // then completed and read again.
    for cut in 0..=lines.len() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("session.jsonl");
        fs::write(&path, lines[..cut].concat()).unwrap();
        let mut reader = ConversationCursor::new();
        let mut events = reader.read(Agent::Claude, &path).unwrap().events;
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(lines[cut..].concat().as_bytes())
            .unwrap();
        loop {
            events.extend(reader.read(Agent::Claude, &path).unwrap().events);
            if !reader.has_more() {
                break;
            }
        }
        assert_eq!(humans(&events), COMMAND_REQUESTS, "split after line {cut}");
    }
}

#[test]
fn a_command_at_the_end_of_the_file_waits_for_its_reply() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let lines: Vec<&str> = CLAUDE_COMMANDS.split_inclusive('\n').collect();
    // Up to and including the `/ship` command, without its expansion.
    fs::write(&path, lines[..9].concat()).unwrap();
    let mut reader = ConversationCursor::new();
    let first = reader.read(Agent::Claude, &path).unwrap();
    assert_eq!(
        humans(&first.events),
        ["설치하고 재측정해줘", "글 정리해줘"]
    );
    assert!(reader.read(Agent::Claude, &path).unwrap().events.is_empty());
}

#[test]
fn a_command_far_from_its_reply_never_stalls_the_reader() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let lines: Vec<&str> = CLAUDE_COMMANDS.split_inclusive('\n').collect();
    // The `/model` command, more than one poll window of records that are not
    // conversation, then its reply.
    let padding = format!(
        "{{\"type\":\"progress\",\"data\":\"{}\"}}\n",
        "x".repeat(100 * 1024)
    );
    let mut contents = lines[..4].concat();
    contents.push_str(&padding.repeat(12));
    contents.push_str(lines[4]);
    fs::write(&path, contents).unwrap();
    let mut reader = ConversationCursor::new();
    let mut polls = 0;
    loop {
        polls += 1;
        assert!(polls <= 5, "the reader made no progress");
        reader.read(Agent::Claude, &path).unwrap();
        if !reader.has_more() {
            break;
        }
    }
}

#[test]
fn a_command_whose_reply_is_past_the_window_edge_is_still_answered_by_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("session.jsonl");
    let lines: Vec<&str> = CLAUDE_COMMANDS.split_inclusive('\n').collect();
    // The `/model` command ends a few bytes short of the poll window's edge, so
    // its reply starts the next window.
    let line_of = |size: usize| {
        format!(
            "{{\"type\":\"progress\",\"data\":\"{}\"}}\n",
            "x".repeat(size)
        )
    };
    let head = lines[..3].concat();
    let edge = hide_session::SESSION_INCREMENT_READ_LIMIT_BYTES as usize - 16;
    let mut contents = head.clone();
    let mut room = edge - head.len() - lines[3].len();
    while room > 150 * 1024 {
        contents.push_str(&line_of(100 * 1024));
        room -= line_of(100 * 1024).len();
    }
    contents.push_str(&line_of(room - line_of(0).len()));
    contents.push_str(lines[3]);
    assert_eq!(contents.len(), edge);
    contents.push_str(lines[4]);
    contents.push_str(lines[lines.len() - 1]);
    fs::write(&path, contents).unwrap();
    let mut reader = ConversationCursor::new();
    let mut events = Vec::new();
    for _ in 0..5 {
        events.extend(reader.read(Agent::Claude, &path).unwrap().events);
        if !reader.has_more() {
            break;
        }
    }
    assert_eq!(humans(&events), ["설치하고 재측정해줘", "마지막 요청"]);
}
