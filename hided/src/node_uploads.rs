//! Files a node's own screen pastes or drops into a terminal (PRD
//! core-host-node-remote-core D-06, B4, B13). The screen uploads them as it
//! uploads to a core's daemon (`attachment_stage`, binary chunks,
//! `attachment_commit`), and this daemon stages every one on its own
//! machine, since a stage does not say which pane it is for. The commit
//! decides where the files go:
//!
//! - a pane of this machine: the core is told the paths staged here, and
//!   pastes them into the pane as it pastes any device's; the bytes never
//!   leave this machine;
//! - any other pane: the stages are sent to the core as the screen sent
//!   them, followed by the commit, and this machine keeps no copy; the bytes
//!   cross to the core once, as they did before.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::Arc;

use serde_json::{Value, json};
use tokio_tungstenite::tungstenite;

use crate::attachments::Attachments;
use crate::screen_event::Kind;
use crate::state_file::SCHEMA_VERSION;

/// The bytes one replayed chunk carries, as the web shell sends them.
const CHUNK_BYTES: usize = 4 * 1024 * 1024;
/// Stages one screen may have opened and not committed; the store's own
/// open-upload cap is the same.
const MAX_SCREEN_STAGES: usize = 64;

/// What a screen said about one stage, kept to send the stage on.
struct Stage {
    name: String,
    size: u64,
    clipboard: bool,
}

/// What became of a screen's frame.
pub enum Handled {
    /// Not an upload frame: it goes to the core.
    NotUpload,
    /// Taken here; these frames answer the screen.
    Answer(Vec<Value>),
    /// Taken here; these frames go to the core in order.
    Up(Vec<tungstenite::Message>),
}

/// One screen's uploads.
pub struct ScreenUploads {
    store: Arc<Attachments>,
    connection: u64,
    node: String,
    stages: HashMap<String, Stage>,
    order: VecDeque<String>,
}

impl ScreenUploads {
    pub fn new(store: Arc<Attachments>, connection: u64, node: &str) -> Self {
        Self {
            store,
            connection,
            node: node.to_owned(),
            stages: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// Takes a screen's event that is part of an upload; `text` is the
    /// event as the screen wrote it, which a commit for a pane of another
    /// machine sends on.
    pub async fn event(&mut self, kind: Kind, event: &Value, text: &str) -> Handled {
        let field = |name: &str| {
            event
                .pointer(&format!("/payload/{name}"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match kind {
            Kind::AttachmentStage => {
                let request_id = field("request_id");
                let stage = Stage {
                    name: field("name"),
                    size: event
                        .pointer("/payload/size")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    clipboard: event
                        .pointer("/payload/clipboard")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                };
                if let Err(reason) = self.store.begin(
                    self.connection,
                    &request_id,
                    &stage.name,
                    stage.size,
                    stage.clipboard,
                ) {
                    return Handled::Answer(vec![refused(&request_id, reason)]);
                }
                if self.order.len() >= MAX_SCREEN_STAGES
                    && let Some(oldest) = self.order.pop_front()
                {
                    self.stages.remove(&oldest);
                }
                self.order.push_back(request_id.clone());
                self.stages.insert(request_id, stage);
                Handled::Answer(Vec::new())
            }
            Kind::AttachmentCancel => {
                let request_id = field("request_id");
                self.forget(&request_id);
                self.store.discard(&request_id);
                Handled::Answer(Vec::new())
            }
            Kind::AttachmentCommit => self.commit(event, text).await,
            _ => Handled::NotUpload,
        }
    }

    /// Takes one binary chunk: every binary frame a screen sends is one.
    pub fn chunk(&self, bytes: &[u8]) -> Vec<Value> {
        match self.store.receive(self.connection, bytes) {
            Some((request_id, reason)) => vec![refused(&request_id, reason)],
            None => Vec::new(),
        }
    }

    fn forget(&mut self, request_id: &str) {
        self.stages.remove(request_id);
        self.order.retain(|id| id != request_id);
    }

    async fn commit(&mut self, event: &Value, text: &str) -> Handled {
        let request_id = event
            .pointer("/payload/request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let pane = event
            .pointer("/payload/pane_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let clipboard = event
            .pointer("/payload/clipboard")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let stages: Vec<String> = event
            .pointer("/payload/stages")
            .and_then(Value::as_array)
            .map(|stages| {
                stages
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if pane.is_empty() {
            return Handled::Answer(vec![refused(&request_id, "no_pane")]);
        }
        if hide_node_link::address::is_pane_of(&pane, &self.node) {
            return self.commit_here(event, &request_id, pane, &stages, clipboard);
        }
        self.send_on(&request_id, text, &stages).await
    }

    /// A pane of this machine: the files stay where they were staged, and
    /// the core is told their paths here.
    fn commit_here(
        &mut self,
        event: &Value,
        request_id: &str,
        pane: &str,
        stages: &[String],
        clipboard: bool,
    ) -> Handled {
        if !crate::attachments::valid_request_id(request_id) {
            return Handled::Answer(vec![refused(request_id, "invalid_request_id")]);
        }
        // A clipboard image is staged at its own path, which the store's
        // commit does not name; it is pasted as a path like any file.
        let clipboard_path = if clipboard {
            match stages.first().map(|stage| self.store.staged_path(stage)) {
                Some(Ok(path)) => Some(path),
                Some(Err(reason)) => return Handled::Answer(vec![refused(request_id, reason)]),
                None => return Handled::Answer(vec![refused(request_id, "unknown_stage")]),
            }
        } else {
            None
        };
        let paths = match self.store.commit(stages, clipboard) {
            Ok(paths) => match clipboard_path {
                Some(path) => vec![path.display().to_string()],
                None => paths,
            },
            Err(reason) => return Handled::Answer(vec![refused(request_id, reason)]),
        };
        for stage in stages {
            self.forget(stage);
        }
        let attachment = json!({
            "schema_version": SCHEMA_VERSION,
            "kind": "terminal_attachment",
            "payload": {
                "request_id": request_id,
                "pane_id": pane,
                "bracketed_paste": event
                    .pointer("/payload/bracketed_paste")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
                "clipboard": false,
                "paths": paths,
                "staged_on": self.node,
            },
        });
        Handled::Up(vec![tungstenite::Message::Text(
            attachment.to_string().into(),
        )])
    }

    /// Another machine's pane: each stage goes to the core as the screen
    /// sent it, then the commit, and the copy here is removed.
    async fn send_on(&mut self, request_id: &str, commit: &str, stages: &[String]) -> Handled {
        // The batch caps the core holds a commit to, checked before any byte
        // is read, so a batch the core would refuse never crosses the link.
        if stages.is_empty() || stages.len() > crate::attachments::MAX_FILES {
            return Handled::Answer(vec![refused(request_id, "too_many_files")]);
        }
        let total: u64 = stages
            .iter()
            .filter_map(|stage| self.stages.get(stage))
            .map(|stage| stage.size)
            .sum();
        if total > crate::attachments::MAX_BATCH_BYTES {
            return Handled::Answer(vec![refused(request_id, "batch_too_large")]);
        }
        let mut frames = Vec::new();
        for stage_id in stages {
            let Some(stage) = self.stages.get(stage_id) else {
                return Handled::Answer(vec![refused(request_id, "unknown_stage")]);
            };
            let path = match self.store.staged_path(stage_id) {
                Ok(path) => path,
                Err(reason) => return Handled::Answer(vec![refused(request_id, reason)]),
            };
            let Ok(bytes) = read(&path).await else {
                return Handled::Answer(vec![refused(request_id, "stage_failed")]);
            };
            if bytes.len() as u64 != stage.size {
                return Handled::Answer(vec![refused(request_id, "size_mismatch")]);
            }
            frames.push(tungstenite::Message::Text(
                json!({
                    "schema_version": SCHEMA_VERSION,
                    "kind": "attachment_stage",
                    "payload": {
                        "request_id": stage_id,
                        "name": stage.name,
                        "size": stage.size,
                        "clipboard": stage.clipboard,
                    },
                })
                .to_string()
                .into(),
            ));
            let mut offset = 0;
            loop {
                let next = (offset + CHUNK_BYTES).min(bytes.len());
                frames.push(tungstenite::Message::Binary(
                    chunk_frame(stage_id, offset, &bytes[offset..next], next >= bytes.len()).into(),
                ));
                offset = next;
                if offset >= bytes.len() {
                    break;
                }
            }
        }
        for stage in stages {
            self.forget(stage);
            self.store.remove(stage);
        }
        frames.push(tungstenite::Message::Text(commit.into()));
        Handled::Up(frames)
    }
}

/// The stages a screen opened and never committed go with it.
impl Drop for ScreenUploads {
    fn drop(&mut self) {
        self.store.release(self.connection);
    }
}

async fn read(path: &Path) -> std::io::Result<Vec<u8>> {
    tokio::fs::read(path).await
}

/// One upload chunk as the web shell frames it: a 4-byte big-endian header
/// length, the header JSON, then the bytes.
fn chunk_frame(request_id: &str, offset: usize, bytes: &[u8], eof: bool) -> Vec<u8> {
    let header = json!({"request_id": request_id, "offset": offset, "eof": eof}).to_string();
    let mut frame = Vec::with_capacity(4 + header.len() + bytes.len());
    frame.extend_from_slice(&(header.len() as u32).to_be_bytes());
    frame.extend_from_slice(header.as_bytes());
    frame.extend_from_slice(bytes);
    frame
}

fn refused(request_id: &str, reason: &str) -> Value {
    json!({"type": "attachment_refused", "payload": {"request_id": request_id, "reason": reason}})
}

#[cfg(test)]
mod tests {
    use super::*;

    const BATCH: &str = "01234567-0123-0123-0123-0123456789ab";

    impl ScreenUploads {
        /// A screen's frame as the node's daemon hands it over.
        async fn text(&mut self, text: &str) -> Handled {
            let taken = [
                Kind::AttachmentStage,
                Kind::AttachmentCancel,
                Kind::AttachmentCommit,
                Kind::Key,
            ];
            match crate::screen_event::read(text, &taken) {
                Some(routed) => self.event(routed.kind, &routed.event, text).await,
                None => Handled::NotUpload,
            }
        }
    }

    fn stage(id: &str, size: usize, clipboard: bool) -> String {
        json!({"schema_version": 2, "kind": "attachment_stage",
            "payload": {"request_id": id, "name": "shot.png", "size": size, "clipboard": clipboard}})
        .to_string()
    }

    fn commit(pane: &str, stages: &[&str], clipboard: bool) -> String {
        json!({"schema_version": 2, "kind": "attachment_commit",
            "payload": {"request_id": BATCH, "pane_id": pane, "bracketed_paste": true,
                "clipboard": clipboard, "stages": stages}})
        .to_string()
    }

    fn uploads(folder: &Path) -> ScreenUploads {
        ScreenUploads::new(
            Arc::new(Attachments::new(folder)),
            1,
            "screen-node",
        )
    }

    async fn upload(uploads: &mut ScreenUploads, id: &str, bytes: &[u8], clipboard: bool) {
        assert!(matches!(
            uploads.text(&stage(id, bytes.len(), clipboard)).await,
            Handled::Answer(answer) if answer.is_empty()
        ));
        assert!(uploads.chunk(&chunk_frame(id, 0, bytes, true)).is_empty());
    }

    #[tokio::test]
    async fn a_file_for_this_machines_pane_stays_here_and_the_core_is_told_its_path() {
        let folder = tempfile::tempdir().unwrap();
        let mut uploads = uploads(folder.path());
        let stage_id = format!("{BATCH}-0");
        upload(&mut uploads, &stage_id, b"local bytes", false).await;
        let Handled::Up(frames) = uploads
            .text(&commit(
                "remote:screen-node:pane:w1:p1",
                &[&stage_id],
                false,
            ))
            .await
        else {
            panic!("the commit was not taken");
        };
        assert_eq!(frames.len(), 1, "only the paths go to the core");
        let tungstenite::Message::Text(text) = &frames[0] else {
            panic!("a text frame");
        };
        let event: Value = serde_json::from_str(text).unwrap();
        assert_eq!(event["kind"], "terminal_attachment");
        assert_eq!(event["payload"]["staged_on"], "screen-node");
        let path = event["payload"]["paths"][0].as_str().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"local bytes");
    }

    #[tokio::test]
    async fn a_clipboard_image_for_this_machines_pane_is_pasted_as_its_staged_path() {
        let folder = tempfile::tempdir().unwrap();
        let mut uploads = uploads(folder.path());
        upload(&mut uploads, BATCH, b"png", true).await;
        let Handled::Up(frames) = uploads
            .text(&commit("remote:screen-node:pane:w1:p1", &[BATCH], true))
            .await
        else {
            panic!("the commit was not taken");
        };
        let tungstenite::Message::Text(text) = &frames[0] else {
            panic!("a text frame");
        };
        let event: Value = serde_json::from_str(text).unwrap();
        assert_eq!(event["payload"]["clipboard"], false);
        let path = event["payload"]["paths"][0].as_str().unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"png");
    }

    #[tokio::test]
    async fn a_file_for_another_machines_pane_goes_to_the_core_as_sent_and_no_copy_stays() {
        let folder = tempfile::tempdir().unwrap();
        let mut uploads = uploads(folder.path());
        let stage_id = format!("{BATCH}-0");
        let bytes = vec![7_u8; CHUNK_BYTES + 10];
        upload(&mut uploads, &stage_id, &bytes, false).await;
        let staged = uploads.store.staged_path(&stage_id).unwrap();
        let committed = commit("local-pane", &[&stage_id], false);
        let Handled::Up(frames) = uploads.text(&committed).await else {
            panic!("the commit was not taken");
        };
        // The stage, two chunks, then the commit as the screen sent it.
        assert_eq!(frames.len(), 4);
        let core = Attachments::new(&folder.path().join("core"));
        for frame in &frames[..3] {
            match frame {
                tungstenite::Message::Text(text) => {
                    let event: Value = serde_json::from_str(text).unwrap();
                    core.begin(
                        2,
                        event["payload"]["request_id"].as_str().unwrap(),
                        "shot.png",
                        event["payload"]["size"].as_u64().unwrap(),
                        false,
                    )
                    .unwrap();
                }
                tungstenite::Message::Binary(chunk) => assert!(core.receive(2, chunk).is_none()),
                _ => panic!("an upload frame"),
            }
        }
        assert_eq!(frames[3], tungstenite::Message::Text(committed.into()));
        let paths = core.commit(std::slice::from_ref(&stage_id), false).unwrap();
        assert_eq!(std::fs::read(&paths[0]).unwrap(), bytes);
        assert!(!staged.exists(), "the copy here was removed");
    }

    #[tokio::test]
    async fn a_refused_stage_answers_the_screen_and_reaches_no_core() {
        let folder = tempfile::tempdir().unwrap();
        let mut uploads = uploads(folder.path());
        let too_large = stage(&format!("{BATCH}-0"), 64 * 1024 * 1024, false);
        let Handled::Answer(answer) = uploads.text(&too_large).await else {
            panic!("the stage was not taken");
        };
        assert_eq!(answer[0]["payload"]["reason"], "too_large");
        assert!(matches!(
            uploads.text(r#"{"kind":"key","payload":{}}"#).await,
            Handled::NotUpload
        ));
    }
}
