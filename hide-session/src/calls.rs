//! A tool call as an agent's hook input and its session file both name it:
//! its tool and a digest of its input ([`key`]). Claude Code's permission
//! request carries no call id, so the call a wait is on is known by this key
//! in the hook, in the core and in the session read alike.

use std::collections::HashSet;
use std::io::{Read, Seek, SeekFrom};
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
    let Ok(mut file) = std::fs::File::open(transcript) else {
        return false;
    };
    let Ok(length) = file.seek(SeekFrom::End(0)) else {
        return false;
    };
    let start = length.saturating_sub(TRANSCRIPT_TAIL_BYTES);
    let mut tail = Vec::new();
    if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut tail).is_err() {
        return false;
    }
    // A read that starts inside a record skips to the next one.
    let whole = if start == 0 {
        &tail[..]
    } else {
        match tail.iter().position(|&byte| byte == b'\n') {
            Some(at) => &tail[at + 1..],
            None => return false,
        }
    };
    let (mut same, mut last) = (None, None);
    let mut results = HashSet::new();
    for line in whole.split(|&byte| byte == b'\n') {
        let Ok(record) = serde_json::from_slice::<Value>(line) else {
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
}
