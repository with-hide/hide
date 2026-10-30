//! Tool calls a turn left failed (PRD hide-ai-suggestions B4, D-45): a call
//! whose result says it failed, unless a later call of the same kind in the
//! same turn succeeded. Each format's records are read for their calls and
//! results by structure; the tracker folds them across reads, as
//! [`crate::turns::TurnTracker`] folds turns, and hands each failure on once
//! its turn is over.
//!
//! A failure is named by its kind only ([`head`]): a shell command's program
//! and subcommand, or the tool's name. Neither the command line nor the
//! result leaves this module.
//!
//! The tracker also counts what the current turn did, without a model (PRD
//! hide-ai-suggestions D-56): its calls, the distinct files it read, the
//! searches it ran more than once and the calls that failed, which the
//! turn-end analysis is handed as one line ([`ToolSummary`]). A read is kept
//! only as a hash of its path and a search as a hash of its pattern, so
//! neither a path nor a pattern (which may hold a token) leaves the reader.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The most calls waiting for their result, and the most failed kinds one
/// turn holds; past either the oldest is dropped, so a turn of a thousand
/// calls is read in bounded memory.
pub const TOOL_CALL_LIMIT: usize = 64;
pub const TOOL_FAILURE_LIMIT: usize = 32;
/// The longest kind kept, in bytes.
pub const HEAD_LIMIT: usize = 120;
/// The most distinct files and searches one turn's count remembers; past
/// the files the count says "at least", and a search past the searches is
/// not counted.
pub const FILE_LIMIT: usize = 128;
pub const SEARCH_LIMIT: usize = 16;
/// The most repeated searches a summary counts.
pub const SUMMARY_SEARCHES: usize = 3;

/// One record's word about a tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolStep {
    /// A call of the kind `head`, and the reads and searches it made.
    Call {
        call: String,
        head: String,
        uses: Vec<ToolUse>,
    },
    /// The result of `call`.
    Result { call: String, failed: bool },
    /// A call and its result in one record (OpenCode's tool part).
    Both {
        head: String,
        failed: bool,
        uses: Vec<ToolUse>,
    },
}

/// What a call did that the turn's summary counts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToolUse {
    /// A file read, by a hash of its path.
    Read(u32),
    /// A search, by a hash of its pattern.
    Search(u32),
}

pub(crate) type Parser = fn(&Value) -> Vec<ToolStep>;

/// A kind of call that failed and stayed failed until its turn ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedTool {
    pub head: String,
    pub at_unix_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct OpenCall {
    call: String,
    head: String,
}

/// The current turn's calls and failures, carried from read to read.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolTracker {
    /// The offset of the first record not folded yet, so a read that starts
    /// earlier folds nothing twice.
    #[serde(default)]
    through: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    calls: Vec<OpenCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    failed: Vec<FailedTool>,
    /// The person's message the counted turn began at; `None` until a read
    /// saw one, when the count may have begun mid-turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    turn: Option<u64>,
    #[serde(default, skip_serializing_if = "TurnUse::is_empty")]
    used: TurnUse,
}

/// What the current turn did, counted as it is folded; kept after the
/// agent stops until a person's message begins the next turn.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct TurnUse {
    #[serde(default)]
    calls: u32,
    #[serde(default)]
    failed: u32,
    /// The distinct files read, by path hash.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<u32>,
    /// A file not remembered was read once [`FILE_LIMIT`] were, so the
    /// count is a floor.
    #[serde(default)]
    files_more: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    searches: Vec<SearchSeen>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SearchSeen {
    hash: u32,
    times: u32,
}

impl TurnUse {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn count(&mut self, uses: Vec<ToolUse>) {
        self.calls = self.calls.saturating_add(1);
        for used in uses {
            match used {
                ToolUse::Read(path) => {
                    if self.files.contains(&path) {
                        continue;
                    }
                    if self.files.len() < FILE_LIMIT {
                        self.files.push(path);
                    } else {
                        self.files_more = true;
                    }
                }
                ToolUse::Search(hash) => {
                    if let Some(seen) = self.searches.iter_mut().find(|seen| seen.hash == hash) {
                        seen.times = seen.times.saturating_add(1);
                    } else if self.searches.len() < SEARCH_LIMIT {
                        self.searches.push(SearchSeen { hash, times: 1 });
                    }
                }
            }
        }
    }
}

/// One turn's tool use as the turn-end analysis is handed it (D-56).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolSummary {
    pub calls: u32,
    pub files_read: u32,
    /// More files were read than [`FILE_LIMIT`]: `files_read` is a floor.
    pub files_more: bool,
    pub failed: u32,
    /// How many times each search run more than once ran, the most first.
    pub repeated_searches: Vec<u32>,
}

impl ToolTracker {
    /// Folds one record's steps, written at `at_unix_ms`.
    pub(crate) fn fold(&mut self, offset: u64, at_unix_ms: u64, steps: Vec<ToolStep>) {
        if offset < self.through {
            return;
        }
        self.through = offset.saturating_add(1);
        for step in steps {
            match step {
                ToolStep::Call { call, head, uses } => {
                    self.used.count(uses);
                    if self.calls.len() == TOOL_CALL_LIMIT {
                        self.calls.remove(0);
                    }
                    self.calls.push(OpenCall { call, head });
                }
                ToolStep::Result { call, failed } => {
                    let Some(index) = self.calls.iter().position(|open| open.call == call) else {
                        continue;
                    };
                    let head = self.calls.remove(index).head;
                    self.settle(head, failed, at_unix_ms);
                }
                ToolStep::Both { head, failed, uses } => {
                    self.used.count(uses);
                    self.settle(head, failed, at_unix_ms);
                }
            }
        }
    }

    fn settle(&mut self, head: String, failed: bool, at_unix_ms: u64) {
        self.failed.retain(|failure| failure.head != head);
        if failed {
            self.used.failed = self.used.failed.saturating_add(1);
            if self.failed.len() == TOOL_FAILURE_LIMIT {
                self.failed.remove(0);
            }
            self.failed.push(FailedTool { head, at_unix_ms });
        }
    }

    /// A person's message at `offset` began a new turn: what the last one
    /// left failed is over.
    pub(crate) fn human(&mut self, offset: u64, ended: &mut Vec<FailedTool>) {
        if offset < self.through {
            return;
        }
        self.through = offset.saturating_add(1);
        ended.append(&mut self.end_turn());
        self.turn = Some(offset);
        self.used = TurnUse::default();
    }

    /// The turn is over (its agent stopped): what it left failed, each kind
    /// once, and nothing left open. What it did stays counted for
    /// [`Self::summary`] until the next turn begins.
    pub fn end_turn(&mut self) -> Vec<FailedTool> {
        self.calls.clear();
        std::mem::take(&mut self.failed)
    }

    /// What the turn begun by the person's message at `offset` did, or
    /// `None` when the count is of another turn or began mid-turn.
    pub fn summary(&self, offset: u64) -> Option<ToolSummary> {
        if self.turn != Some(offset) {
            return None;
        }
        let used = &self.used;
        let mut repeated = used
            .searches
            .iter()
            .filter(|seen| seen.times > 1)
            .map(|seen| seen.times)
            .collect::<Vec<_>>();
        repeated.sort_by_key(|times| std::cmp::Reverse(*times));
        repeated.truncate(SUMMARY_SEARCHES);
        Some(ToolSummary {
            calls: used.calls,
            files_read: used.files.len() as u32,
            files_more: used.files_more,
            failed: used.failed,
            repeated_searches: repeated,
        })
    }
}

/// The kind of a call: a shell command's program, with its subcommand when
/// it names one, or else the tool's own name.
pub fn head(tool: &str, command: Option<&str>) -> String {
    let named = command.and_then(command_head);
    let head = named.unwrap_or_else(|| tool.trim().to_owned());
    let mut end = head.len().min(HEAD_LIMIT);
    while !head.is_char_boundary(end) {
        end -= 1;
    }
    head[..end].to_owned()
}

/// What runs first in `line` that is not only a change of place or of
/// environment (`cd`, `export`, an assignment).
fn command_head(line: &str) -> Option<String> {
    hide_shell::simple_commands(line)
        .into_iter()
        .find_map(|words| {
            let mut words = words.into_iter().skip_while(|word| {
                hide_shell::is_assignment(word) || hide_shell::is_prefix(word, false)
            });
            let program = words.next()?;
            let program = program
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(&program)
                .to_owned();
            if matches!(
                program.as_str(),
                "cd" | "pushd" | "popd" | "export" | "set" | "source" | "." | "unset" | "true"
            ) || program.is_empty()
            {
                return None;
            }
            let sub = words.next().filter(|word| {
                word.len() <= 32
                    && word.starts_with(|c: char| c.is_ascii_alphabetic())
                    && word
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
            });
            Some(match sub {
                Some(sub) => format!("{program} {sub}"),
                None => program,
            })
        })
}

/// The reads and searches a call made: a shell command's by its words, or
/// a read or search tool's by its name and its path or pattern.
fn uses(tool: &str, input: Option<&Value>) -> Vec<ToolUse> {
    let Some(input) = input else {
        return Vec::new();
    };
    if let Some(line) = command_of(input) {
        return shell_uses(&line);
    }
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| input.get(*key).and_then(Value::as_str))
    };
    match tool.trim().to_ascii_lowercase().as_str() {
        "read" | "read_file" | "readfile" | "view" | "view_file" | "open_file" => {
            text(&["file_path", "filePath", "path", "target_file", "file"])
                .map(read)
                .into_iter()
                .collect()
        }
        "grep" | "glob" | "search" | "find" | "grep_search" | "file_search" | "search_files"
        | "codebase_search" => text(&["pattern", "query", "regex", "glob_pattern", "search_term"])
            .and_then(search)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

/// The files a shell line reads and the patterns it searches for, from the
/// programs that do only that: `cat`, `head`, `sed -n` and the like, and
/// `rg`, `grep`, `find` and the like.
fn shell_uses(line: &str) -> Vec<ToolUse> {
    let mut uses = Vec::new();
    for words in hide_shell::simple_commands(line) {
        let mut words = words.into_iter().skip_while(|word| {
            hide_shell::is_assignment(word) || hide_shell::is_prefix(word, false)
        });
        let Some(program) = words.next() else {
            continue;
        };
        let args = words.collect::<Vec<_>>();
        let program = program.rsplit(['/', '\\']).next().unwrap_or(&program);
        match program {
            "head" | "tail" => uses.extend(operands(&args, &["-n", "-c"]).map(read)),
            "cat" | "less" | "more" | "bat" | "nl" => uses.extend(operands(&args, &[]).map(read)),
            "sed"
                if args.iter().any(|arg| arg == "-n")
                    && !args.iter().any(|arg| arg.starts_with("-i")) =>
            {
                // The script is the first operand unless `-e` or `-f` gave it.
                let given = args.iter().any(|arg| arg == "-e" || arg == "-f");
                uses.extend(
                    operands(&args, &["-e", "-f"])
                        .skip(usize::from(!given))
                        .map(read),
                );
            }
            "rg" | "grep" | "egrep" | "fgrep" | "ag" | "ack" => {
                uses.extend(pattern(&args).and_then(search));
            }
            "git" if args.first().is_some_and(|sub| sub == "grep") => {
                uses.extend(pattern(&args[1..]).and_then(search));
            }
            "fd" => uses.extend(operands(&args, PATTERN_VALUE_FLAGS).next().and_then(search)),
            "find" => uses.extend(
                args.windows(2)
                    .find(|pair| {
                        matches!(pair[0].as_str(), "-name" | "-iname" | "-path" | "-regex")
                    })
                    .and_then(|pair| search(&pair[1])),
            ),
            _ => {}
        }
    }
    uses
}

/// The flags of a search program that take the next word as their value.
const PATTERN_VALUE_FLAGS: &[&str] = &[
    "-f",
    "-g",
    "--glob",
    "-t",
    "--type",
    "-T",
    "--type-not",
    "-A",
    "-B",
    "-C",
    "-m",
    "--max-count",
    "--context",
    "--after-context",
    "--before-context",
    "--include",
    "--exclude",
    "-M",
    "--max-columns",
    "-d",
    "--max-depth",
    "-j",
    "--threads",
    "-E",
    "--encoding",
    "-e",
    "--regexp",
];

/// The words of `args` that are not options, skipping the value of each
/// flag in `valued`; every word after `--` is one.
fn operands<'a>(args: &'a [String], valued: &'a [&str]) -> impl Iterator<Item = &'a str> + 'a {
    let mut skip = false;
    let mut ended = false;
    args.iter().filter_map(move |arg| {
        if skip {
            skip = false;
            return None;
        }
        if ended {
            return Some(arg.as_str());
        }
        if arg == "--" {
            ended = true;
            return None;
        }
        if arg.starts_with('-') && arg.len() > 1 {
            skip = valued.contains(&arg.as_str());
            return None;
        }
        Some(arg.as_str())
    })
}

/// A search program's pattern: the value of `-e`, or its first operand;
/// none for a listing of files (`rg --files`).
fn pattern(args: &[String]) -> Option<&str> {
    if args.iter().any(|arg| arg == "--files") {
        return None;
    }
    args.windows(2)
        .find(|pair| matches!(pair[0].as_str(), "-e" | "--regexp"))
        .map(|pair| pair[1].as_str())
        .or_else(|| operands(args, PATTERN_VALUE_FLAGS).next())
}

fn read(path: &str) -> ToolUse {
    let path = path.trim();
    ToolUse::Read(fnv1a(path.strip_prefix("./").unwrap_or(path)))
}

fn search(text: &str) -> Option<ToolUse> {
    let text = text.trim();
    (!text.is_empty()).then(|| ToolUse::Search(fnv1a(text)))
}

/// A path's hash that stays the same across builds and machines, so a
/// tracker read on a device and carried to the core compares alike.
fn fnv1a(text: &str) -> u32 {
    text.bytes().fold(0x811c_9dc5, |hash, byte| {
        (hash ^ u32::from(byte)).wrapping_mul(0x0100_0193)
    })
}

/// A shell tool's command line, wherever the format keeps it.
fn command_of(input: &Value) -> Option<String> {
    match input.get("command").or_else(|| input.get("cmd"))? {
        Value::String(line) => Some(line.clone()),
        // Codex's older `shell` tool: an argv, `bash -lc <line>` included.
        Value::Array(argv) => {
            let words = argv.iter().filter_map(Value::as_str).collect::<Vec<_>>();
            match words.as_slice() {
                [shell, flag, line] if flag.starts_with('-') && flag.ends_with('c') => {
                    let _ = shell;
                    Some((*line).to_owned())
                }
                _ => Some(words.join(" ")),
            }
        }
        _ => None,
    }
}

fn call_id(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && id.len() <= crate::turns::NATIVE_ID_LIMIT_BYTES)
        .map(str::to_owned)
}

/// Claude Code: an assistant's `tool_use` blocks and a user record's
/// `tool_result` blocks, `is_error` on a failed one.
pub(crate) fn claude(item: &Value) -> Vec<ToolStep> {
    let role = item.get("type").and_then(Value::as_str);
    let Some(blocks) = item.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(
            |block| match (role, block.get("type").and_then(Value::as_str)) {
                (Some("assistant"), Some("tool_use")) => {
                    let name = block.get("name").and_then(Value::as_str)?;
                    let input = block.get("input");
                    Some(ToolStep::Call {
                        call: call_id(block.get("id"))?,
                        head: head(name, input.and_then(command_of).as_deref()),
                        uses: uses(name, input),
                    })
                }
                (Some("user"), Some("tool_result")) => Some(ToolStep::Result {
                    call: call_id(block.get("tool_use_id"))?,
                    failed: block.get("is_error").and_then(Value::as_bool) == Some(true),
                }),
                _ => None,
            },
        )
        .collect()
}

/// Codex: `function_call`/`function_call_output` for its shell tools, whose
/// output carries the exit code, and `custom_tool_call`/`_output` for a
/// script that runs commands (`exec`), whose output carries each command's
/// `exit_code`.
pub(crate) fn codex(item: &Value) -> Vec<ToolStep> {
    if item.get("type").and_then(Value::as_str) != Some("response_item") {
        return Vec::new();
    }
    let Some(payload) = item.get("payload") else {
        return Vec::new();
    };
    let call = || call_id(payload.get("call_id"));
    let step = match payload.get("type").and_then(Value::as_str) {
        Some("function_call") => (|| {
            let name = payload.get("name").and_then(Value::as_str)?;
            let arguments = payload
                .get("arguments")
                .and_then(Value::as_str)
                .and_then(|arguments| serde_json::from_str::<Value>(arguments).ok());
            Some(ToolStep::Call {
                call: call()?,
                head: head(name, arguments.as_ref().and_then(command_of).as_deref()),
                uses: uses(name, arguments.as_ref()),
            })
        })(),
        Some("custom_tool_call") => (|| {
            let name = payload.get("name").and_then(Value::as_str)?;
            let script = payload.get("input").and_then(Value::as_str);
            Some(ToolStep::Call {
                call: call()?,
                head: head(name, script.and_then(script_command).as_deref()),
                uses: script
                    .into_iter()
                    .flat_map(script_commands)
                    .flat_map(|command| shell_uses(&command))
                    .collect(),
            })
        })(),
        Some("function_call_output" | "custom_tool_call_output") => {
            call().map(|call| ToolStep::Result {
                call,
                failed: codex_failed(payload.get("output")),
            })
        }
        _ => None,
    };
    step.into_iter().collect()
}

/// The first command a Codex script runs: the string after its first
/// `cmd:`, read as the script's string literal.
fn script_command(script: &str) -> Option<String> {
    script_commands(script).next()
}

/// Every command a Codex script runs, in order: the string after each
/// `cmd:`, read as the script's string literal.
fn script_commands(script: &str) -> impl Iterator<Item = String> + '_ {
    script
        .match_indices("cmd:")
        .filter_map(|(start, cmd)| literal(script[start + cmd.len()..].trim_start()))
}

/// The string literal `rest` begins with, unescaped.
fn literal(rest: &str) -> Option<String> {
    let quote = rest
        .chars()
        .next()
        .filter(|c| matches!(c, '"' | '\'' | '`'))?;
    let mut line = String::new();
    let mut chars = rest[quote.len_utf8()..].chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next()? {
                'n' => line.push('\n'),
                't' => line.push('\t'),
                other => line.push(other),
            },
            c if c == quote => return Some(line),
            c => line.push(c),
        }
    }
    None
}

/// Whether a Codex tool output says its command failed: a nonzero exit
/// code in any of the forms Codex writes it.
pub(crate) fn codex_failed(output: Option<&Value>) -> bool {
    let failed = |code: i64| code != 0;
    let texts: Vec<&str> = match output {
        Some(Value::String(text)) => vec![text],
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect(),
        Some(Value::Object(object)) => {
            return object
                .get("metadata")
                .and_then(|metadata| metadata.get("exit_code"))
                .and_then(Value::as_i64)
                .is_some_and(failed);
        }
        _ => return false,
    };
    texts.iter().any(|text| {
        let text = text.trim_start();
        if let Ok(value) = serde_json::from_str::<Value>(text)
            && let Some(code) = value.pointer("/metadata/exit_code").and_then(Value::as_i64)
        {
            return failed(code);
        }
        ["Exit code: ", "Process exited with code ", "\"exit_code\":"]
            .iter()
            .any(|marker| codes_after(text, marker).any(failed))
    })
}

fn codes_after<'a>(text: &'a str, marker: &'a str) -> impl Iterator<Item = i64> + 'a {
    text.match_indices(marker).filter_map(move |(at, _)| {
        let digits = text[at + marker.len()..]
            .trim_start()
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect::<String>();
        digits.parse().ok()
    })
}

/// Pi and omp: an assistant message's `toolCall` blocks and a `toolResult`
/// message, `isError` on a failed one.
pub(crate) fn pi(item: &Value) -> Vec<ToolStep> {
    if item["type"] != "message" {
        return Vec::new();
    }
    let message = &item["message"];
    match message["role"].as_str() {
        Some("toolResult") => call_id(message.get("toolCallId"))
            .map(|call| ToolStep::Result {
                call,
                failed: message["isError"] == true,
            })
            .into_iter()
            .collect(),
        Some("assistant") => message
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|block| block["type"] == "toolCall")
            .filter_map(|block| {
                let name = block.get("name").and_then(Value::as_str)?;
                let arguments = block.get("arguments");
                Some(ToolStep::Call {
                    call: call_id(block.get("id"))?,
                    head: head(name, arguments.and_then(command_of).as_deref()),
                    uses: uses(name, arguments),
                })
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Grok: a `tool_call` update names the call, and its terminal update says
/// `completed` or `failed`.
pub(crate) fn grok(item: &Value) -> Vec<ToolStep> {
    let Some(update) = item.pointer("/params/update") else {
        return Vec::new();
    };
    let Some(call) = call_id(update.get("toolCallId")) else {
        return Vec::new();
    };
    let kind = update.get("sessionUpdate").and_then(Value::as_str);
    let mut steps = Vec::new();
    if kind == Some("tool_call") {
        let name = update
            .get("title")
            .or_else(|| update.get("kind"))
            .and_then(Value::as_str)
            .unwrap_or("tool");
        let input = update.get("rawInput");
        steps.push(ToolStep::Call {
            call: call.clone(),
            head: head(name, input.and_then(command_of).as_deref()),
            uses: uses(name, input),
        });
    }
    if matches!(kind, Some("tool_call" | "tool_call_update")) {
        match update["status"].as_str() {
            Some("completed") => steps.push(ToolStep::Result {
                call,
                failed: false,
            }),
            Some("failed") => steps.push(ToolStep::Result { call, failed: true }),
            _ => {}
        }
    }
    steps
}

/// OpenCode: one `tool` part holds its call and, once settled, its state.
pub(crate) fn opencode_part(part: &Value) -> Option<ToolStep> {
    if part["type"] != "tool" {
        return None;
    }
    let failed = match part.pointer("/state/status").and_then(Value::as_str)? {
        "completed" => false,
        "error" => true,
        _ => return None,
    };
    let tool = part.get("tool").and_then(Value::as_str)?;
    let input = part.pointer("/state/input");
    Some(ToolStep::Both {
        head: head(tool, input.and_then(command_of).as_deref()),
        failed,
        uses: uses(tool, input),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_command_is_named_by_its_program_and_subcommand() {
        assert_eq!(head("Bash", Some("cargo test -p herdr-core")), "cargo test");
        assert_eq!(head("Bash", Some("cd web && pnpm build")), "pnpm build");
        assert_eq!(
            head("Bash", Some("RUST_LOG=1 /usr/bin/git push origin")),
            "git push"
        );
        assert_eq!(head("Bash", Some("python3 -m pytest")), "python3");
        assert_eq!(head("Edit", None), "Edit");
        assert_eq!(head("Bash", Some("cd somewhere")), "Bash");
    }

    fn tracker(steps: &[(u64, Vec<ToolStep>)]) -> ToolTracker {
        let mut tracker = ToolTracker::default();
        for (offset, steps) in steps {
            tracker.fold(*offset, *offset * 10, steps.clone());
        }
        tracker
    }

    fn call(id: &str, head: &str) -> ToolStep {
        ToolStep::Call {
            call: id.into(),
            head: head.into(),
            uses: Vec::new(),
        }
    }

    fn result(id: &str, failed: bool) -> ToolStep {
        ToolStep::Result {
            call: id.into(),
            failed,
        }
    }

    #[test]
    fn a_failure_the_same_turn_fixed_is_not_kept() {
        let mut fixed = tracker(&[
            (1, vec![call("a", "cargo test")]),
            (2, vec![result("a", true)]),
            (3, vec![call("b", "cargo test")]),
            (4, vec![result("b", false)]),
        ]);
        assert!(fixed.end_turn().is_empty());

        let mut left = tracker(&[
            (1, vec![call("a", "cargo test"), call("b", "npm run")]),
            (2, vec![result("a", true), result("b", true)]),
            (3, vec![call("c", "cargo test")]),
            (4, vec![result("c", true)]),
        ]);
        let ended = left.end_turn();
        assert_eq!(
            ended.iter().map(|f| f.head.as_str()).collect::<Vec<_>>(),
            ["npm run", "cargo test"],
            "each kind once, the last failure's time"
        );
        assert_eq!(ended[1].at_unix_ms, 40);
    }

    #[test]
    fn a_new_turn_ends_the_last_and_a_reread_folds_nothing_twice() {
        let mut tracker = tracker(&[
            (1, vec![call("a", "git push")]),
            (2, vec![result("a", true)]),
        ]);
        tracker.fold(2, 20, vec![result("a", true)]);
        let mut ended = Vec::new();
        tracker.human(5, &mut ended);
        assert_eq!(ended.len(), 1);
        tracker.human(5, &mut ended);
        assert_eq!(ended.len(), 1, "the same message read again");
        assert!(tracker.end_turn().is_empty());
    }

    #[test]
    fn each_format_says_which_call_failed() {
        let claude_call = json!({"type":"assistant","message":{"content":[
            {"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}}]}});
        let claude_result = json!({"type":"user","message":{"content":[
            {"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"boom"}]}});
        assert_eq!(claude(&claude_call), [call("t1", "cargo test")]);
        assert_eq!(claude(&claude_result), [result("t1", true)]);

        let codex_call = json!({"type":"response_item","payload":{"type":"function_call",
            "name":"shell","call_id":"c1","arguments":"{\"command\":[\"bash\",\"-lc\",\"npm test\"]}"}});
        let codex_out = json!({"type":"response_item","payload":{"type":"function_call_output",
            "call_id":"c1","output":"Exit code: 2\nWall time: 1s\nOutput:\nfail"}});
        assert_eq!(codex(&codex_call), [call("c1", "npm test")]);
        assert_eq!(codex(&codex_out), [result("c1", true)]);
        let script = json!({"type":"response_item","payload":{"type":"custom_tool_call",
            "name":"exec","call_id":"c2","input":"text(await tools.exec_command({cmd:\"docker version\"}))"}});
        let script_out = json!({"type":"response_item","payload":{"type":"custom_tool_call_output",
            "call_id":"c2","output":[{"type":"input_text","text":"Script completed\n"},
            {"type":"input_text","text":"{\"chunk_id\":\"a\",\"exit_code\":1,\"output\":\"x\"}"}]}});
        assert_eq!(codex(&script), [call("c2", "docker version")]);
        assert_eq!(codex(&script_out), [result("c2", true)]);
        let passed = json!({"type":"response_item","payload":{"type":"custom_tool_call_output",
            "call_id":"c2","output":[{"type":"input_text","text":"{\"exit_code\":0}"}]}});
        assert_eq!(codex(&passed), [result("c2", false)]);

        let pi_call = json!({"type":"message","message":{"role":"assistant","content":[
            {"type":"toolCall","id":"p1","name":"bash","arguments":{"command":"make check"}}]}});
        let pi_result = json!({"type":"message","message":{"role":"toolResult","toolCallId":"p1","isError":true}});
        assert_eq!(pi(&pi_call), [call("p1", "make check")]);
        assert_eq!(pi(&pi_result), [result("p1", true)]);

        let grok_failed = json!({"params":{"update":{"sessionUpdate":"tool_call_update",
            "toolCallId":"g1","status":"failed"}}});
        assert_eq!(grok(&grok_failed), [result("g1", true)]);

        let part = json!({"type":"tool","tool":"bash","callID":"o1",
            "state":{"status":"error","input":{"command":"go test ./..."}}});
        assert_eq!(
            opencode_part(&part),
            Some(ToolStep::Both {
                head: "go test".into(),
                failed: true,
                uses: Vec::new(),
            })
        );
    }

    fn calling(id: &str, uses: Vec<ToolUse>) -> ToolStep {
        ToolStep::Call {
            call: id.into(),
            head: "tool".into(),
            uses,
        }
    }

    #[test]
    fn a_turn_counts_its_files_repeated_searches_and_failures_until_the_next_turn() {
        let mut tracker = ToolTracker::default();
        let mut ended = Vec::new();
        tracker.human(1, &mut ended);
        let mut offset = 2;
        for path in ["vite.config.ts", "package.json", "playwright.config.ts"] {
            tracker.fold(
                offset,
                0,
                vec![calling(&format!("r{offset}"), vec![read(path)])],
            );
            offset += 1;
        }
        for _ in 0..2 {
            tracker.fold(
                offset,
                0,
                vec![calling(
                    &format!("s{offset}"),
                    shell_uses("rg -n \"5173\" web"),
                )],
            );
            offset += 1;
        }
        tracker.fold(
            offset,
            0,
            vec![calling("again", vec![read("./package.json")])],
        );
        tracker.fold(
            offset + 1,
            0,
            vec![calling("x", Vec::new()), result("x", true)],
        );
        tracker.end_turn();
        assert_eq!(
            tracker.summary(1),
            Some(ToolSummary {
                calls: 7,
                files_read: 3,
                files_more: false,
                failed: 1,
                repeated_searches: vec![2],
            }),
            "a file read twice is one file, and the count outlives the agent's stop"
        );
        assert_eq!(
            tracker.summary(0),
            None,
            "another turn's count is not this one's"
        );
        tracker.human(20, &mut ended);
        assert_eq!(tracker.summary(20), Some(ToolSummary::default()));
    }

    #[test]
    fn past_its_file_limit_a_turn_reads_at_least_that_many_and_keeps_no_pattern() {
        let mut tracker = ToolTracker::default();
        tracker.human(1, &mut Vec::new());
        let token = "access_token=ghp_abcdefghijklmnopqrstuvwxyz0123";
        for offset in 2..FILE_LIMIT as u64 + 12 {
            let mut uses = vec![read(&format!("src/{offset}.rs"))];
            uses.extend(shell_uses(&format!("rg '{token}'")));
            tracker.fold(offset, 0, vec![calling(&format!("r{offset}"), uses)]);
        }
        let summary = tracker.summary(1).unwrap();
        assert_eq!(
            (summary.files_read, summary.files_more),
            (FILE_LIMIT as u32, true),
            "reads past the limit say at least, not more files"
        );
        assert_eq!(summary.repeated_searches, [FILE_LIMIT as u32 + 10]);
        let kept = serde_json::to_string(&tracker).unwrap();
        assert!(!kept.contains("ghp_"), "{kept}");
        assert!(!kept.contains("access_token"), "{kept}");
    }

    #[test]
    fn a_count_that_began_mid_turn_is_no_summary() {
        let mut tracker = ToolTracker::default();
        tracker.fold(3, 0, vec![calling("a", vec![read("a.rs")])]);
        assert_eq!(tracker.summary(1), None);
    }

    #[test]
    fn reads_and_searches_are_recognized_in_each_format_and_in_shell_words() {
        let hash = |path| read(path);
        assert_eq!(
            shell_uses("cd web && sed -n '1,40p' src/a.ts | head -n 5"),
            [hash("src/a.ts")]
        );
        assert_eq!(shell_uses("cat a.rs b.rs"), [hash("a.rs"), hash("b.rs")]);
        assert_eq!(
            shell_uses("rg -g '*.rs' -n PORT"),
            [ToolUse::Search(fnv1a("PORT"))]
        );
        assert_eq!(
            shell_uses("grep -e 'x y' -r ."),
            [ToolUse::Search(fnv1a("x y"))]
        );
        assert_eq!(
            shell_uses("git grep -n TODO"),
            [ToolUse::Search(fnv1a("TODO"))]
        );
        assert_eq!(
            shell_uses("find . -name '*.md'"),
            [ToolUse::Search(fnv1a("*.md"))]
        );
        assert!(
            shell_uses("rg --files web").is_empty(),
            "a listing is no search"
        );
        assert!(
            shell_uses("sed -i 's/a/b/' x").is_empty(),
            "an edit is no read"
        );
        assert!(shell_uses("cargo test").is_empty());

        let claude_read = json!({"type":"assistant","message":{"content":[
            {"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"/r/AGENTS.md"}},
            {"type":"tool_use","id":"t2","name":"Grep","input":{"pattern":"5173","path":"web"}}]}});
        let steps = claude(&claude_read);
        assert!(
            matches!(&steps[0], ToolStep::Call { uses, .. } if *uses == [hash("/r/AGENTS.md")])
        );
        assert!(
            matches!(&steps[1], ToolStep::Call { uses, .. } if *uses == [ToolUse::Search(fnv1a("5173"))])
        );

        let codex_script = json!({"type":"response_item","payload":{"type":"custom_tool_call",
            "name":"exec","call_id":"c2","input":"text(await tools.exec_command({cmd:\"rg -n PORT web\"}))"}});
        assert!(
            matches!(&codex(&codex_script)[0], ToolStep::Call { uses, .. } if *uses == [ToolUse::Search(fnv1a("PORT"))])
        );
        let batched = json!({"type":"response_item","payload":{"type":"custom_tool_call",
            "name":"exec","call_id":"c3","input":"const a = await tools.exec_command({cmd:\"cat a.ts\"});\nconst b = await tools.exec_command({cmd:\"rg PORT web\"});\nconst c = await tools.exec_command({cmd:'rg PORT desktop'});"}});
        assert!(
            matches!(&codex(&batched)[0], ToolStep::Call { head, uses, .. }
                if head == "cat" && *uses == [hash("a.ts"), ToolUse::Search(fnv1a("PORT")), ToolUse::Search(fnv1a("PORT"))]),
            "every command a script runs is counted, its first names the call"
        );
        assert_eq!(shell_uses("cat -n a.ts"), [hash("a.ts")]);
        assert_eq!(shell_uses("tail -n 20 a.ts"), [hash("a.ts")]);
        assert_eq!(shell_uses("sed -n -e '1,40p' a.ts"), [hash("a.ts")]);

        let pi_read = json!({"type":"message","message":{"role":"assistant","content":[
            {"type":"toolCall","id":"p1","name":"read","arguments":{"path":"README.md"}}]}});
        assert!(
            matches!(&pi(&pi_read)[0], ToolStep::Call { uses, .. } if *uses == [hash("README.md")])
        );

        let grok_read = json!({"params":{"update":{"sessionUpdate":"tool_call","toolCallId":"g1",
            "title":"read_file","rawInput":{"target_file":"README.md"}}}});
        assert!(
            matches!(&grok(&grok_read)[0], ToolStep::Call { uses, .. } if *uses == [hash("README.md")])
        );

        let opencode_read = json!({"type":"tool","tool":"read","callID":"o1",
            "state":{"status":"completed","input":{"filePath":"/r/a.ts"}}});
        assert!(
            matches!(opencode_part(&opencode_read), Some(ToolStep::Both { uses, .. }) if uses == [hash("/r/a.ts")])
        );
    }
}
