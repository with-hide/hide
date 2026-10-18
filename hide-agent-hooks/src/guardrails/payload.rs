//! What each runtime's pre-tool payload says a call does, for the
//! guardrails: a shell command, the files a tool writes, or the files it
//! reads (PRD agent-guardrails D-05, B23).
//!
//! The tool names and input keys are each runtime's own, observed on the
//! versions Hide installs into: Claude Code 2.1.300 (`Edit`, `Write`,
//! `NotebookEdit`, `Read` with `file_path` or `notebook_path`), Codex 0.162.1
//! (`apply_patch` with the patch text in `command`), Cursor 2026.10.14
//! (`Read` and `Write`, an edit included, with `file_path`), Grok 1.0.52 (`read_file`
//! with `path`, `search_replace` and `write` with `file_path`), OpenCode
//! 1.18.30 (`read`, `edit`, `write` with `filePath`, `apply_patch` with
//! `patchText`), Pi 1.0.4 and omp 18.7.0 (`read`, `edit`, `write` with
//! `path`), agy 1.3.6 (`run_command` with `CommandLine` and `Cwd`,
//! `view_file` with `AbsolutePath`, `write_to_file`, `replace_file_content`
//! and `multi_replace_file_content` with `TargetFile`). A key another version
//! renames reads as no path, and the call runs.

use std::path::PathBuf;

use hide_agent_adapter::HookDialect;
use serde_json::Value;

use super::Action;

/// A tool call: what it does, and the folder it runs in when the payload
/// names one.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCall {
    pub action: Action,
    pub cwd: Option<PathBuf>,
}

/// The shell tools by name, across runtimes.
const SHELL_TOOLS: [&str; 6] = [
    "Bash",
    "run_terminal_command",
    "Shell",
    "bash",
    "run_command",
    "shell",
];

/// The keys agy's file writes carry their new text under, ahead of the
/// call's name in agy's sorted payload.
const AGY_WRITE_CONTENT_KEYS: [&str; 3] =
    ["CodeContent", "ReplacementContent", "ReplacementChunks"];

/// The keys a shell tool's input carries its command under.
const COMMAND_KEYS: [&str; 2] = ["command", "CommandLine"];

/// The tools that write a file by name, across runtimes.
const WRITE_TOOLS: [&str; 16] = [
    "Edit",
    "Write",
    "MultiEdit",
    "NotebookEdit",
    "apply_patch",
    "search_replace",
    "write",
    "edit",
    "StrReplace",
    "Delete",
    "EditNotebook",
    "multiedit",
    "write_to_file",
    "replace_file_content",
    "multi_replace_file_content",
    "patch",
];

const READ_TOOLS: [&str; 4] = ["Read", "read_file", "read", "view_file"];

/// The keys a tool's input names its file under, across runtimes.
const PATH_KEYS: [&str; 7] = [
    "file_path",
    "path",
    "filePath",
    "notebook_path",
    "target_file",
    "TargetFile",
    "AbsolutePath",
];

/// The keys a patch tool's input carries its text under.
const PATCH_KEYS: [&str; 3] = ["command", "patchText", "patch"];

/// The pre-tool payload of a command hook (Claude Code, Codex, Cursor,
/// Grok, agy). A payload cut at the read limit is
/// read for its tool and path from the bytes that arrived.
pub fn read_hook(payload: &[u8], truncated: bool, dialect: HookDialect) -> Option<ToolCall> {
    if truncated {
        return read_cut(payload, dialect);
    }
    let value: Value = serde_json::from_slice(payload).ok()?;
    let (tool, input, cwd) = match dialect {
        HookDialect::Grok => (
            value.get("toolName").and_then(Value::as_str),
            value.get("toolInput"),
            value.get("cwd").and_then(Value::as_str),
        ),
        // agy names no cwd beside the call: a shell call carries its own
        // `Cwd`, and the conversation's workspace is the first of
        // `workspacePaths`.
        HookDialect::Antigravity => (
            value.pointer("/toolCall/name").and_then(Value::as_str),
            value.pointer("/toolCall/args"),
            value.pointer("/workspacePaths/0").and_then(Value::as_str),
        ),
        _ => (
            value.get("tool_name").and_then(Value::as_str),
            value.get("tool_input"),
            value.get("cwd").and_then(Value::as_str),
        ),
    };
    let input_cwd = input
        .and_then(|input| {
            ["working_directory", "cwd", "Cwd"]
                .into_iter()
                .find_map(|key| input.get(key))
        })
        .and_then(Value::as_str);
    let workspace = value
        .get("workspace_roots")
        .and_then(Value::as_array)
        .and_then(|roots| roots.first())
        .and_then(Value::as_str);
    let cwd = [input_cwd, cwd, workspace]
        .into_iter()
        .flatten()
        .find(|cwd| !cwd.is_empty())
        .map(PathBuf::from);
    let action = action(tool?, input?)?;
    Some(ToolCall { action, cwd })
}

/// What `tool` with `input` does, for a hook payload and for the scripts of
/// OpenCode, Pi and omp alike.
pub fn action(tool: &str, input: &Value) -> Option<Action> {
    if SHELL_TOOLS.contains(&tool) {
        return COMMAND_KEYS
            .iter()
            .find_map(|key| input.get(*key))
            .and_then(Value::as_str)
            .map(|command| Action::Shell(command.to_owned()));
    }
    let paths = paths(input);
    if WRITE_TOOLS.contains(&tool) {
        return (!paths.is_empty()).then_some(Action::Write(paths));
    }
    if READ_TOOLS.contains(&tool) {
        return (!paths.is_empty()).then_some(Action::Read(paths));
    }
    None
}

fn paths(input: &Value) -> Vec<String> {
    let mut paths: Vec<String> = PATH_KEYS
        .iter()
        .filter_map(|key| input.get(*key).and_then(Value::as_str))
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect();
    for key in PATCH_KEYS {
        if let Some(patch) = input.get(key).and_then(Value::as_str) {
            paths.extend(patch_paths(patch));
        }
    }
    paths
}

/// The files a patch in the `*** Begin Patch` form names (Codex's
/// `apply_patch`, OpenCode 1.x's `apply_patch` and 2.x's `patch`): added,
/// updated, deleted and moved to.
pub fn patch_paths(patch: &str) -> Vec<String> {
    patch
        .lines()
        .filter_map(|line| {
            [
                "*** Add File: ",
                "*** Update File: ",
                "*** Delete File: ",
                "*** Move to: ",
            ]
            .iter()
            .find_map(|marker| line.strip_prefix(marker))
        })
        .map(|path| path.trim().to_owned())
        .filter(|path| !path.is_empty())
        .collect()
}

/// A payload cut at the read limit: its tool name and its file path, read
/// from the bytes that arrived. A shell command cut short is not decided on.
fn read_cut(payload: &[u8], dialect: HookDialect) -> Option<ToolCall> {
    let tool_key = match dialect {
        HookDialect::Grok => "toolName",
        // agy sorts its keys, so the call's `name` follows its arguments: a
        // payload cut inside a long `CodeContent` has lost the name and the
        // `TargetFile` after it, and is judged as a write Hide cannot read.
        HookDialect::Antigravity => "name",
        _ => "tool_name",
    };
    let Some(tool) = string_after(payload, tool_key) else {
        let writes = dialect == HookDialect::Antigravity
            && AGY_WRITE_CONTENT_KEYS.iter().any(|key| {
                let needle = format!("\"{key}\"");
                payload
                    .windows(needle.len())
                    .any(|window| window == needle.as_bytes())
            });
        return writes.then_some(ToolCall {
            action: Action::UnreadWrite,
            cwd: None,
        });
    };
    if SHELL_TOOLS.contains(&tool.as_str()) {
        return None;
    }
    let mut paths: Vec<String> = PATH_KEYS
        .iter()
        .filter_map(|key| string_after(payload, key))
        .filter(|path| !path.is_empty())
        .collect();
    // A patch is cut inside its own text, so its file lines are read from
    // the escaped bytes, each one that ended before the cut.
    paths.extend(cut_patch_paths(payload));
    let action = if WRITE_TOOLS.contains(&tool.as_str()) && !paths.is_empty() {
        Action::Write(paths)
    } else if READ_TOOLS.contains(&tool.as_str()) && !paths.is_empty() {
        Action::Read(paths)
    } else {
        return None;
    };
    Some(ToolCall {
        action,
        cwd: string_after(payload, "cwd")
            .or_else(|| string_after(payload, "Cwd"))
            .filter(|cwd| !cwd.is_empty())
            .map(PathBuf::from),
    })
}

/// The file lines of a patch inside a JSON string that may be cut short:
/// each marker's path up to the escaped newline (`\n`) or the string's end.
fn cut_patch_paths(payload: &[u8]) -> Vec<String> {
    let mut paths = Vec::new();
    for marker in [
        "*** Add File: ",
        "*** Update File: ",
        "*** Delete File: ",
        "*** Move to: ",
    ] {
        let marker = marker.as_bytes();
        let mut from = 0;
        while let Some(at) = payload[from..]
            .windows(marker.len())
            .position(|window| window == marker)
        {
            let start = from + at + marker.len();
            let rest = &payload[start..];
            let Some(end) = rest
                .windows(2)
                .position(|pair| pair == b"\\n" || pair[0] == b'"')
            else {
                break;
            };
            if let Ok(path) = std::str::from_utf8(&rest[..end]) {
                let path = path.trim();
                if !path.is_empty() {
                    paths.push(path.to_owned());
                }
            }
            from = start + end;
        }
    }
    paths
}

/// The whole JSON string value of the first `"key":` in `bytes`.
fn string_after(bytes: &[u8], key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let at = bytes
        .windows(needle.len())
        .position(|window| window == needle.as_bytes())?;
    let rest = &bytes[at + needle.len()..];
    let rest = &rest[rest.iter().position(|byte| !byte.is_ascii_whitespace())?..];
    let rest = rest.strip_prefix(b":")?;
    let start = rest.iter().position(|byte| !byte.is_ascii_whitespace())?;
    serde_json::Deserializer::from_slice(&rest[start..])
        .into_iter::<String>()
        .next()?
        .ok()
}
