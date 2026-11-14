//! A tool call as an agent's hook input and its session file both name it:
//! its tool and a digest of its input ([`key`]). Claude Code's permission
//! request carries no call id, so the call a wait is on is known by this key
//! in the hook, in the core and in the session read alike.

use std::collections::HashSet;
use std::path::Path;

use serde_json::Value;
use sha2::Digest;

/// The longest tool name a key keeps; a longer one is not a call this
/// module names.
pub const NAME_LIMIT: usize = 128;

/// How much of a session file's end [`answered_in_claude_file`] reads: the
/// waited call and what answered it are the file's last records while the
/// turn waits.
pub const TRANSCRIPT_TAIL_BYTES: u64 = 256 * 1024;

/// `name:digest`: the first eight bytes of SHA-256 over the input with each
/// object's keys in order, so the hook's payload and the session file, which
/// may write the same input with its keys in another order, give one key.
pub fn key(name: &str, input: &Value) -> Option<String> {
    if name.is_empty() || name.len() > NAME_LIMIT || name.contains(':') {
        return None;
    }
    Some(format!("{name}:{}", digest(input)))
}

/// `name:`, the key of a call whose input was not read whole.
pub fn key_by_name(name: &str) -> Option<String> {
    (!name.is_empty() && name.len() <= NAME_LIMIT && !name.contains(':'))
        .then(|| format!("{name}:"))
}

/// Claude Code's question tool. Its dialog asks the operator a question,
/// yet Claude Code opens it with the same permission request as any call
/// that needs approval.
pub const CLAUDE_QUESTION_TOOL: &str = "AskUserQuestion";

/// Whether the call a key names ([`key`], [`key_by_name`]) asks the
/// operator a question rather than for a permission.
pub fn asks_question(waited: &str) -> bool {
    waited
        .split_once(':')
        .is_some_and(|(name, _)| name == CLAUDE_QUESTION_TOOL)
}

/// Whether a call known by `name` and, when read whole, the digest `input`
/// is the call `waited` names: the same tool, with the same input when both
/// sides have one.
pub fn ends(waited: &str, name: &str, input: Option<&str>) -> bool {
    let Some((waited_name, waited_input)) = waited.split_once(':') else {
        return false;
    };
    waited_name == name
        && (waited_input.is_empty() || input.is_none_or(|input| input == waited_input))
}

/// The digest half of [`key`].
pub fn digest(input: &Value) -> String {
    let mut hasher = sha2::Sha256::new();
    canonical(input, &mut hasher);
    hasher.finalize()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn canonical(value: &Value, hasher: &mut sha2::Sha256) {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            hasher.update(b"{");
            for key in keys {
                hasher.update(Value::String(key.clone()).to_string().as_bytes());
                hasher.update(b":");
                canonical(&map[key], hasher);
                hasher.update(b",");
            }
            hasher.update(b"}");
        }
        Value::Array(items) => {
            hasher.update(b"[");
            for item in items {
                canonical(item, hasher);
                hasher.update(b",");
            }
            hasher.update(b"]");
        }
        other => hasher.update(other.to_string().as_bytes()),
    }
}

/// Whether Claude Code's session file at `transcript` records a result for
/// the call `waited` names: the last call with its input, or, for a key
/// known by its tool alone, the tool's last call. A refusal in the permission
/// dialog writes that result and sends no hook event (Claude Code 2.1.312).
/// A call the file does not hold, as a background subagent's, which asks
/// under the main session and writes its own file, a call beyond the file's
/// last [`TRANSCRIPT_TAIL_BYTES`], or a file that cannot be read, is not
/// answered.
pub fn answered_in_claude_file(transcript: &Path, waited: &str) -> bool {
    let Some((name, input)) = waited.split_once(':') else {
        return false;
    };
    let Ok(tail) = crate::read_tail(transcript, TRANSCRIPT_TAIL_BYTES) else {
        return false;
    };
    let (mut same, mut last) = (None, None);
    let mut results = HashSet::new();
    for (_, line) in tail.records() {
        let Ok(record) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(blocks) = record.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        for block in blocks {
            let id = block["id"].as_str();
            match block["type"].as_str() {
                Some("tool_use") if block["name"].as_str() == Some(name) => {
                    if !input.is_empty() && digest(&block["input"]) == input {
                        same = id.map(str::to_owned);
                    }
                    last = id.map(str::to_owned);
                }
                Some("tool_result") => {
                    if let Some(answered) = block["tool_use_id"].as_str() {
                        results.insert(answered.to_owned());
                    }
                }
                _ => {}
            }
        }
    }
    let waited_call = if input.is_empty() { last } else { same };
    waited_call.is_some_and(|call| results.contains(&call))
}

/// The subagents the main agent stopped during its turn `turn`, as its
/// Codex session file records them: the ids, as Codex's hooks send them as
/// `agent_id`, whose latest word in the turn is a stop, in the order written.
///
/// A subagent the main agent stops itself (`interrupt_agent`, or the older
/// tools' `close_agent`) sends no `SubagentStop`, and the main agent's session
/// file is the one place that says so (Codex 0.167.2, measured): the current
/// tools write a `SubAgentActivity` item of kind `interrupted`, the older ones
/// a `CollabAgentToolCall` of `close_agent`. Any later word on the same
/// subagent (a follow-up task, a message) takes the stop back. A start or a
/// completion here says nothing the hooks have not: the file records a
/// completion only after the subagent's `SubagentStop`, so a turn end read
/// between the two would see a finished subagent as started.
///
/// The records read are those after the turn's `task_started` within the
/// file's last [`TRANSCRIPT_TAIL_BYTES`], whichever start is nearer; a stop
/// before that is not read. A file that cannot be read is an error.
pub fn codex_subagents_stopped_in_turn(
    transcript: &Path,
    turn: &str,
) -> crate::Result<Vec<String>> {
    let tail = crate::read_tail(transcript, TRANSCRIPT_TAIL_BYTES)?;
    let records: Vec<Value> = tail
        .records()
        .filter_map(|(_, line)| serde_json::from_str::<Value>(line).ok())
        .collect();
    let from = records
        .iter()
        .rposition(|record| {
            record["type"] == "event_msg"
                && record["payload"]["type"] == "task_started"
                && record["payload"]["turn_id"] == turn
        })
        .unwrap_or(0);
    let mut latest: Vec<(&str, bool)> = Vec::new();
    for record in &records[from..] {
        for (id, stopped) in subagent_words(record) {
            latest.retain(|(known, _)| *known != id);
            latest.push((id, stopped));
        }
    }
    Ok(latest
        .into_iter()
        .filter(|(_, stopped)| *stopped)
        .map(|(id, _)| id.to_owned())
        .collect())
}

/// What one Codex session record says of subagents: each id it names, and
/// whether it says the main agent stopped it.
fn subagent_words(record: &Value) -> Vec<(&str, bool)> {
    let payload = &record["payload"];
    if record["type"] != "event_msg" || payload["type"] != "item_completed" {
        return Vec::new();
    }
    let item = &payload["item"];
    match item["type"].as_str() {
        Some("SubAgentActivity") => item["agent_thread_id"]
            .as_str()
            .map(|id| vec![(id, item["kind"] == "interrupted")])
            .unwrap_or_default(),
        Some("CollabAgentToolCall") => {
            let closed = item["tool"] == "close_agent" && item["status"] == "completed";
            item["receiver_thread_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|id| (id, closed))
                .collect()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_key_names_the_input_whatever_the_order_of_its_keys() {
        let one = key("Bash", &json!({"command": "ls", "description": "list"})).unwrap();
        let other = key("Bash", &json!({"description": "list", "command": "ls"})).unwrap();
        assert_eq!(one, other);
        assert!(one.starts_with("Bash:") && one.len() == "Bash:".len() + 16);
        assert_ne!(one, key("Bash", &json!({"command": "ls -a"})).unwrap());
        assert_eq!(key("a:b", &json!({})), None);
        assert!(ends(&one, "Bash", None));
        assert!(ends(&one, "Bash", Some(&one["Bash:".len()..])));
        assert!(!ends(&one, "Bash", Some("0000000000000000")));
        assert!(ends("Bash:", "Bash", Some("ab")));
        assert!(!ends(&one, "Read", None));
    }

    /// Claude Code 2.1.312 records a refusal in the permission dialog as
    /// the call's result, and sends no hook event for it.
    #[test]
    fn a_refused_call_is_answered_in_the_session_file_and_a_pending_one_is_not() {
        let folder = tempfile::tempdir().unwrap();
        let transcript = folder.path().join("session.jsonl");
        let input = json!({"command": "rm -rf build", "description": "Clean"});
        let waited = key("Bash", &input).unwrap();
        let call = json!({"type": "assistant", "message": {"role": "assistant", "content": [
            {"type": "tool_use", "id": "toolu_read", "name": "Read", "input": {"file_path": "/x"}},
            {"type": "tool_use", "id": "toolu_bash", "name": "Bash", "input": input},
        ]}});
        let read_result = json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_read", "content": "x"}]}});
        let refusal = json!({"type": "user", "message": {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "toolu_bash",
             "content": "User refused permission to run tool", "is_error": true}]},
            "permissionDecision": {"decision": "reject", "source": "user_reject"}});
        // A long record before the call leaves the read starting inside it.
        let filler = json!({"type": "user", "message": {"role": "user",
            "content": "x".repeat(TRANSCRIPT_TAIL_BYTES as usize)}});
        std::fs::write(&transcript, format!("{filler}\n{call}\n{read_result}\n")).unwrap();
        assert!(
            !answered_in_claude_file(&transcript, &waited),
            "the dialog is still open"
        );
        std::fs::write(
            &transcript,
            format!("{filler}\n{call}\n{read_result}\n{refusal}\n"),
        )
        .unwrap();
        assert!(answered_in_claude_file(&transcript, &waited));
        // A call known by its tool alone is the tool's last call.
        assert!(answered_in_claude_file(&transcript, "Bash:"));
        assert!(!answered_in_claude_file(&transcript, "Edit:"));
        assert!(!answered_in_claude_file(
            &folder.path().join("missing.jsonl"),
            &waited
        ));
        // A background subagent's call, which this file does not hold, is
        // not answered by the main session's earlier call of the same tool.
        let subagents = key("Bash", &json!({"command": "cargo test"})).unwrap();
        assert!(!answered_in_claude_file(&transcript, &subagents));
    }

    fn activity(id: &str, kind: &str) -> Value {
        json!({"type": "event_msg", "payload": {"type": "item_completed",
            "thread_id": "main", "turn_id": "t",
            "item": {"type": "SubAgentActivity", "kind": kind, "agent_thread_id": id,
                     "agent_path": "/root/x"}}})
    }

    fn collab(tool: &str, id: &str) -> Value {
        json!({"type": "event_msg", "payload": {"type": "item_completed",
            "thread_id": "main", "turn_id": "t",
            "item": {"type": "CollabAgentToolCall", "tool": tool, "status": "completed",
                     "receiver_thread_ids": [id], "agents_states": {id: "running"}}}})
    }

    fn task_started(turn: &str) -> Value {
        json!({"type": "event_msg", "payload": {"type": "task_started", "turn_id": turn}})
    }

    fn codex_file(records: &[Value]) -> (tempfile::TempDir, std::path::PathBuf) {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().join("rollout.jsonl");
        let text: String = records.iter().map(|record| format!("{record}\n")).collect();
        std::fs::write(&path, text).unwrap();
        (folder, path)
    }

    /// The records Codex 0.167.2 wrote with its current subagent tools: a
    /// subagent whose latest word in the turn is an interrupt was stopped, and
    /// an earlier turn's words are not read.
    #[test]
    fn a_codex_subagent_stopped_in_the_turn_is_one_whose_latest_word_is_a_stop() {
        let (_folder, path) = codex_file(&[
            task_started("t0"),
            activity("old", "interrupted"),
            task_started("t1"),
            activity("a", "started"),
            activity("b", "started"),
            activity("a", "completed"),
            activity("c", "started"),
            activity("c", "interrupted"),
            activity("a", "interacted"),
            activity("d", "started"),
            activity("d", "interrupted"),
            activity("d", "interacted"),
            // A record that only mentions a subagent kind in its text.
            json!({"type": "response_item", "payload": {"type": "message",
                "content": [{"type": "input_text", "text": "SubAgentActivity interrupted b"}]}}),
        ]);
        assert_eq!(codex_subagents_stopped_in_turn(&path, "t1").unwrap(), ["c"]);
    }

    /// The older tools record only the main agent's calls: a close is a
    /// stop, and a later message to the same subagent takes it back.
    #[test]
    fn of_the_older_codex_tools_only_a_close_is_a_stop() {
        let (_folder, path) = codex_file(&[
            task_started("t1"),
            collab("spawn_agent", "a"),
            collab("spawn_agent", "b"),
            collab("close_agent", "b"),
            collab("close_agent", "a"),
            collab("send_input", "a"),
        ]);
        assert_eq!(codex_subagents_stopped_in_turn(&path, "t1").unwrap(), ["b"]);
    }

    /// A turn that wrote more than the tail after a word leaves it unread,
    /// and a missing file is an error.
    #[test]
    fn a_codex_word_before_the_tail_is_not_read() {
        let filler = json!({"type": "response_item", "payload": "x".repeat(4096)});
        let mut records = vec![task_started("t1"), activity("far", "interrupted")];
        let fill = TRANSCRIPT_TAIL_BYTES as usize / 4096 + 1;
        records.extend(std::iter::repeat_n(filler, fill));
        records.push(activity("near", "interrupted"));
        let (folder, path) = codex_file(&records);
        assert_eq!(
            codex_subagents_stopped_in_turn(&path, "t1").unwrap(),
            ["near"]
        );
        assert!(codex_subagents_stopped_in_turn(&folder.path().join("gone.jsonl"), "t1").is_err());
    }
}
