//! The calls an agent has open, followed in its session's thread
//! (`hide_session::thread`, the reader the desktop conversation reads) for a
//! phone's detail alone: the newest open command, so the conversation can
//! say a command is running and since when (Doop 02), and the approval card
//! can show the command a permission waits on, cut (Doop 04). Nothing else
//! of a call leaves hided (docs/ARCHITECTURE.md, The mobile companion).
//!
//! The thread is read from where the last read ended, so a read costs what
//! the session wrote since; the first read, and one that fell more than
//! [`CATCH_UP_PAGES`] pages behind, reads the newest page.

use hide_session::Accounts;
use hide_session::thread::{ThreadCursor, ThreadRead, ThreadRequest, acp, read_pane_thread};

/// The open calls kept; past it the oldest is dropped.
const OPEN_LIMIT: usize = 32;
/// The forward pages one read follows before it starts over from the
/// newest page.
const CATCH_UP_PAGES: usize = 8;
/// What a phone is shown of a waiting command: its first lines, at most
/// this many characters; the rest, which may hold a secret, stays here.
pub const COMMAND_CHARS: usize = 200;
const COMMAND_LINES: usize = 2;

/// A call the session opened and has not ended.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenCall {
    id: String,
    /// The command it runs, for a call that runs one.
    pub command: Option<String>,
    /// When the session recorded the call.
    pub since_ms: Option<u64>,
}

/// One pane's open calls, followed through its session.
#[derive(Debug)]
pub struct Calls {
    request: ThreadRequest,
    cursor: Option<ThreadCursor>,
    open: Vec<OpenCall>,
    /// Why the last read failed, so a failure is logged once, not each read.
    failed: Option<String>,
}

impl Calls {
    /// Follows the session `request` names; its `read` is set by each read.
    pub fn new(request: ThreadRequest) -> Self {
        Self {
            request,
            cursor: None,
            open: Vec::new(),
            failed: None,
        }
    }

    /// Whether this follows the session `request` names, whatever it reads.
    pub fn follows(&self, request: &ThreadRequest) -> bool {
        let ours = &self.request;
        ours.agent == request.agent
            && ours.reference_kind == request.reference_kind
            && ours.reference_value == request.reference_value
            && ours.cwd == request.cwd
            && ours.account == request.account
    }

    /// Reads what the session wrote since the last read; a failure keeps
    /// what was read before and is answered once, until the next one differs.
    pub fn follow(&mut self, accounts: &Accounts) -> Option<String> {
        match self.read(accounts) {
            Ok(()) => {
                self.failed = None;
                None
            }
            Err(reason) if self.failed.as_ref() == Some(&reason) => None,
            Err(reason) => {
                self.failed = Some(reason.clone());
                Some(reason)
            }
        }
    }

    fn read(&mut self, accounts: &Accounts) -> Result<(), String> {
        for _ in 0..CATCH_UP_PAGES {
            self.request.read = match &self.cursor {
                None => ThreadRead::Newest,
                Some(cursor) => ThreadRead::From(cursor.clone()),
            };
            let page = match read_pane_thread(accounts, &self.request) {
                Ok(page) => page,
                // The file was replaced: read it anew from its newest page.
                Err(refusal)
                    if refusal.reason() == "thread_cursor_stale" && self.cursor.is_some() =>
                {
                    self.cursor = None;
                    continue;
                }
                Err(refusal) => return Err(refusal.to_string()),
            };
            if self.cursor.is_none() {
                self.open.clear();
            }
            self.fold(&page.updates);
            self.cursor = Some(page.end);
            if !page.more {
                return Ok(());
            }
        }
        // Too far behind to follow: the newest page, next time.
        self.cursor = None;
        Ok(())
    }

    fn fold(&mut self, updates: &[acp::SessionUpdate]) {
        for update in updates {
            match update {
                // The operator's next message starts a turn: nothing before
                // it still runs, whatever a format left without its result.
                acp::SessionUpdate::UserMessageChunk(_) => self.open.clear(),
                acp::SessionUpdate::ToolCall(call) => {
                    let id = call.tool_call_id.0.as_ref();
                    self.open.retain(|open| open.id != id);
                    if ended(call.status) {
                        continue;
                    }
                    if self.open.len() == OPEN_LIMIT {
                        self.open.remove(0);
                    }
                    self.open.push(OpenCall {
                        id: id.to_owned(),
                        command: (call.kind == acp::ToolKind::Execute).then(|| call.title.clone()),
                        since_ms: call
                            .meta
                            .as_ref()
                            .and_then(|meta| meta.get("hide"))
                            .and_then(|hide| hide.get("at_ms"))
                            .and_then(serde_json::Value::as_u64),
                    });
                }
                acp::SessionUpdate::ToolCallUpdate(update)
                    if update.fields.status.is_some_and(ended) =>
                {
                    let id = update.tool_call_id.0.as_ref();
                    self.open.retain(|open| open.id != id);
                }
                _ => {}
            }
        }
    }

    /// The newest open call that runs a command.
    pub fn command(&self) -> Option<&OpenCall> {
        self.open.iter().rev().find(|call| call.command.is_some())
    }
}

fn ended(status: acp::ToolCallStatus) -> bool {
    matches!(
        status,
        acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed
    )
}

/// A command as a phone is shown it: its first [`COMMAND_LINES`] lines, at
/// most [`COMMAND_CHARS`] characters, and whether anything was left out.
pub fn cut(command: &str) -> (String, bool) {
    let command = command.trim();
    let mut lines = command.lines();
    let mut shown = lines
        .by_ref()
        .take(COMMAND_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    let mut cut = lines.next().is_some();
    if let Some((at, _)) = shown.char_indices().nth(COMMAND_CHARS) {
        shown.truncate(at);
        cut = true;
    }
    (shown, cut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn update(value: serde_json::Value) -> acp::SessionUpdate {
        serde_json::from_value(value).unwrap()
    }

    fn calls() -> Calls {
        Calls::new(ThreadRequest {
            agent: hide_session::Agent::Claude,
            reference_kind: "id".into(),
            reference_value: "s".into(),
            cwd: None,
            account: Default::default(),
            read: ThreadRead::Newest,
        })
    }

    /// The newest command still open is the one a phone is told of, with
    /// when its session recorded it; a result, or the operator's next
    /// message, ends it, and a call that runs no command is never one.
    #[test]
    fn the_newest_open_command_is_followed_until_its_result_or_the_next_turn() {
        let mut calls = calls();
        calls.fold(&[
            update(
                json!({"sessionUpdate": "tool_call", "toolCallId": "a", "title": "cargo test",
                "kind": "execute", "status": "pending", "_meta": {"hide": {"at_ms": 100}}}),
            ),
            update(
                json!({"sessionUpdate": "tool_call", "toolCallId": "b", "title": "src/lib.rs",
                "kind": "read", "status": "pending"}),
            ),
        ]);
        let open = calls.command().unwrap();
        assert_eq!(open.command.as_deref(), Some("cargo test"));
        assert_eq!(open.since_ms, Some(100));
        calls.fold(&[update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "a",
            "status": "completed"}),
        )]);
        assert_eq!(calls.command(), None);
        calls.fold(&[update(
            json!({"sessionUpdate": "tool_call", "toolCallId": "c",
            "title": "npm run build", "kind": "execute", "status": "pending"}),
        )]);
        assert!(calls.command().is_some());
        calls.fold(&[update(json!({"sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "멈추고 다시"}}))]);
        assert_eq!(calls.command(), None);
    }

    #[test]
    fn a_waiting_command_is_shown_by_its_first_lines_only() {
        assert_eq!(cut("  git status  "), ("git status".to_owned(), false));
        assert_eq!(
            cut("cat <<EOF\nTOKEN=abc\nsecret\nEOF"),
            ("cat <<EOF\nTOKEN=abc".to_owned(), true)
        );
        let long = "x".repeat(COMMAND_CHARS + 5);
        assert_eq!(cut(&long), ("x".repeat(COMMAND_CHARS), true));
    }
}
