//! The files a phone attaches to a reply (Doop 08, 09): each one uploaded
//! into hided's attachment store (`crate::attachments`), the store the
//! desktop's uploads use, with its caps, its eviction and its release of a
//! connection's open stages, and committed by the reply that names it
//! (`phone::input`), whose words then type the files' paths.
//!
//! A phone message is JSON text of at most [`super::phone::MAX_FRAME`], so
//! bytes travel as base64 in `attach_chunk` frames of at most [`CHUNK`] bytes:
//!
//! - `attach_begin {stage, name, size}` opens a stage of `size` bytes, at
//!   most [`FILE_BYTES`]; `stage` is a UUID the phone makes;
//! - `attach_chunk {stage, seq, data}` appends the next bytes, answered
//!   `attach_ack {stage, seq}` once they are on disk;
//! - `attach_end {stage}` says every byte was sent: the stage closes, its
//!   size checked against `size`, answered `attach_ack {stage, done: true}`;
//! - `attach_cancel {stage}` drops the stage and its file, answered by
//!   nothing.
//!
//! Any refusal is answered `attach_ack {stage, failed}` with the store's
//! reason. The phone keeps at most [`WINDOW`] chunks out that hided has not
//! acknowledged, so its frames never fill the eight that hided takes from
//! each phone (`relay::INBOUND_QUEUE`) while it reads them in order.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::attachments::{self, Attachments};

/// The largest file a phone attaches, the operator's rule for a phone
/// (photos are made smaller on the phone first); the store's own cap is
/// larger, for a file the desktop attaches.
pub const FILE_BYTES: u64 = 10 * 1024 * 1024;
/// The bytes one `attach_chunk` carries at most: its base64 and the frame's
/// other fields fit one phone message, in whole three-byte groups.
pub const CHUNK: usize = 12_000;
/// The chunks a phone keeps out that hided has not acknowledged: half the
/// eight frames hided takes from a phone at once, so a ping and a request
/// always fit beside them.
pub const WINDOW: usize = 4;
/// What an `attach_chunk` frame holds beside its base64: its type, a stage
/// id, a sequence number and the JSON around them.
const CHUNK_FRAME_EXTRA: usize = 128;

const _: () = assert!(CHUNK.is_multiple_of(3));
const _: () = assert!(CHUNK / 3 * 4 + CHUNK_FRAME_EXTRA <= super::phone::MAX_FRAME);
const _: () = assert!(FILE_BYTES <= attachments::MAX_FILE_BYTES);
const _: () = assert!(WINDOW < super::relay::INBOUND_QUEUE);

fn ack(stage: &str, fields: Value) -> Value {
    let mut frame = json!({"type": "attach_ack", "stage": stage});
    if let (Some(frame), Value::Object(fields)) = (frame.as_object_mut(), fields) {
        frame.extend(fields);
    }
    frame
}

fn failed(stage: &str, reason: &str) -> Value {
    ack(stage, json!({"failed": reason}))
}

/// The stage a frame names, when it has the shape a phone makes.
fn stage_of(message: &Value) -> Option<&str> {
    message
        .get("stage")
        .and_then(Value::as_str)
        .filter(|stage| attachments::valid_request_id(stage))
}

/// One upload frame of `connection`; the answer to send, if any. `None` for
/// a frame that names no stage the phone could match an answer to: the
/// caller records it as refused.
pub fn handle(store: &Attachments, connection: u64, kind: &str, message: &Value) -> Result<Option<Value>, &'static str> {
    let stage = stage_of(message).ok_or("attach_stage")?;
    Ok(match kind {
        "attach_begin" => {
            let name = message.get("name").and_then(Value::as_str).unwrap_or_default();
            let Some(size) = message.get("size").and_then(Value::as_u64) else {
                return Ok(Some(failed(stage, "invalid_request")));
            };
            if size > FILE_BYTES {
                return Ok(Some(failed(stage, "too_large")));
            }
            store
                .begin(connection, stage, name, size, false)
                .err()
                .map(|reason| failed(stage, reason))
        }
        "attach_chunk" => {
            let (Some(seq), Some(data)) = (
                message.get("seq").and_then(Value::as_u64),
                message.get("data").and_then(Value::as_str),
            ) else {
                store.discard(stage);
                return Ok(Some(failed(stage, "invalid_request")));
            };
            let Ok(bytes) = STANDARD.decode(data) else {
                store.discard(stage);
                return Ok(Some(failed(stage, "invalid_request")));
            };
            if bytes.len() > CHUNK {
                store.discard(stage);
                return Ok(Some(failed(stage, "invalid_request")));
            }
            Some(match store.write(stage, &bytes, false) {
                Ok(_) => ack(stage, json!({"seq": seq})),
                Err(reason) => failed(stage, reason),
            })
        }
        "attach_end" => Some(match store.write(stage, &[], true) {
            Ok(_) => ack(stage, json!({"done": true})),
            Err(reason) => failed(stage, reason),
        }),
        "attach_cancel" => {
            store.discard(stage);
            None
        }
        _ => return Err("attach_kind"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAGE: &str = "01234567-0123-4123-8123-0123456789ab";

    fn store() -> (tempfile::TempDir, Attachments) {
        let dir = tempfile::tempdir().unwrap();
        let store = Attachments::new(dir.path());
        (dir, store)
    }

    fn frame(kind: &str, fields: Value) -> Value {
        let mut frame = json!({"type": kind, "stage": STAGE});
        frame.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
        frame
    }

    fn send(store: &Attachments, connection: u64, kind: &str, fields: Value) -> Option<Value> {
        handle(store, connection, kind, &frame(kind, fields)).unwrap()
    }

    fn chunk(store: &Attachments, seq: u64, bytes: &[u8]) -> Option<Value> {
        send(store, 1, "attach_chunk", json!({"seq": seq, "data": STANDARD.encode(bytes)}))
    }

    /// A file the phone uploads in chunks is acknowledged chunk by chunk,
    /// closes at its declared size, and the reply that names it commits it
    /// as a path holding exactly those bytes.
    #[test]
    fn a_phone_upload_is_acknowledged_by_chunk_and_committed_by_its_reply() {
        let (_dir, store) = store();
        assert_eq!(send(&store, 1, "attach_begin", json!({"name": "holder.log", "size": 5})), None);
        assert_eq!(chunk(&store, 0, b"abc"), Some(json!({"type": "attach_ack", "stage": STAGE, "seq": 0})));
        assert_eq!(chunk(&store, 1, b"de"), Some(json!({"type": "attach_ack", "stage": STAGE, "seq": 1})));
        assert_eq!(
            send(&store, 1, "attach_end", json!({})),
            Some(json!({"type": "attach_ack", "stage": STAGE, "done": true}))
        );
        let paths = store.commit_reply(1, &[STAGE.to_owned()]).unwrap();
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"abcde");
        assert!(paths[0].ends_with(&format!("{STAGE}-holder.log")), "{paths:?}");
    }

    /// The phone's file cap is refused before any byte crosses, and a stage
    /// that closes short of its declared size is dropped with its file.
    #[test]
    fn a_file_past_ten_mib_or_short_of_its_size_is_refused() {
        let (_dir, store) = store();
        assert_eq!(
            send(&store, 1, "attach_begin", json!({"name": "trace.json", "size": FILE_BYTES + 1})),
            Some(failed(STAGE, "too_large"))
        );
        assert_eq!(send(&store, 1, "attach_begin", json!({"name": "a.log", "size": 4})), None);
        chunk(&store, 0, b"ab");
        assert_eq!(send(&store, 1, "attach_end", json!({})), Some(failed(STAGE, "size_mismatch")));
        assert_eq!(store.commit_reply(1, &[STAGE.to_owned()]), Err("unknown_stage"));
    }

    /// A cancelled upload leaves nothing to commit, and a chunk after the
    /// cancel is answered as a stage hided does not hold.
    #[test]
    fn a_cancelled_upload_is_gone_and_its_late_chunk_is_refused() {
        let (dir, store) = store();
        send(&store, 1, "attach_begin", json!({"name": "a.log", "size": 4}));
        chunk(&store, 0, b"ab");
        assert_eq!(send(&store, 1, "attach_cancel", json!({})), None);
        assert_eq!(chunk(&store, 1, b"cd"), Some(failed(STAGE, "unknown_stage")));
        let left = std::fs::read_dir(dir.path().join("attachments")).unwrap().count();
        assert_eq!(left, 0, "the cancelled stage's file went with it");
    }

    /// A lost connection releases its uploads that were not committed, as
    /// the store releases a desktop window's: the phone uploads them again.
    #[test]
    fn a_lost_connection_releases_its_open_uploads() {
        let (_dir, store) = store();
        send(&store, 1, "attach_begin", json!({"name": "a.log", "size": 2}));
        chunk(&store, 0, b"ab");
        send(&store, 1, "attach_end", json!({}));
        store.release(1);
        assert_eq!(store.commit_reply(1, &[STAGE.to_owned()]), Err("unknown_stage"));
        // The same stage may be uploaded again once it was released.
        assert_eq!(send(&store, 2, "attach_begin", json!({"name": "a.log", "size": 2})), None);
    }

    /// A reply that did not land may be sent again naming the same stages:
    /// the same connection gets the same paths without the bytes crossing
    /// again, and no other connection may name them.
    #[test]
    fn the_same_reply_again_commits_the_same_paths_on_its_connection_only() {
        let (_dir, store) = store();
        send(&store, 1, "attach_begin", json!({"name": "a.log", "size": 2}));
        chunk(&store, 0, b"ab");
        send(&store, 1, "attach_end", json!({}));
        let first = store.commit_reply(1, &[STAGE.to_owned()]).unwrap();
        assert_eq!(store.commit_reply(1, &[STAGE.to_owned()]).unwrap(), first);
        assert_eq!(store.commit_reply(2, &[STAGE.to_owned()]), Err("unknown_stage"));
        // A desktop commit never names a stage again.
        assert_eq!(store.commit(&[STAGE.to_owned()], false), Err("unknown_stage"));
    }

    /// One reply that names a stage still uploading, or one it never began,
    /// commits none of them: the stages that did arrive stay to be named.
    #[test]
    fn a_reply_with_one_missing_stage_commits_none() {
        let (_dir, store) = store();
        send(&store, 1, "attach_begin", json!({"name": "a.log", "size": 2}));
        chunk(&store, 0, b"ab");
        send(&store, 1, "attach_end", json!({}));
        let other = "11111111-1111-4111-8111-111111111111".to_owned();
        assert_eq!(store.commit_reply(1, &[STAGE.to_owned(), other]), Err("unknown_stage"));
        assert!(store.commit_reply(1, &[STAGE.to_owned()]).is_ok(), "the arrived stage stays");
    }

    /// The largest chunk a phone sends, with a stage of the longest shape
    /// and a sequence number past any a 10 MiB file reaches, is one phone
    /// message: hided reads it rather than dropping it unread.
    #[test]
    fn the_largest_chunk_frame_fits_one_phone_message() {
        let largest = json!({
            "type": "attach_chunk",
            "stage": STAGE,
            "seq": u32::MAX,
            "data": STANDARD.encode(vec![0xff_u8; CHUNK]),
        })
        .to_string();
        assert!(FILE_BYTES.div_ceil(CHUNK as u64) < u64::from(u32::MAX));
        assert!(
            largest.len() <= super::super::phone::MAX_FRAME,
            "{} bytes",
            largest.len()
        );
        let (_dir, store) = store();
        send(&store, 1, "attach_begin", json!({"name": "a.bin", "size": CHUNK}));
        let read: Value = serde_json::from_str(&largest).unwrap();
        assert_eq!(
            handle(&store, 1, "attach_chunk", &read).unwrap(),
            Some(json!({"type": "attach_ack", "stage": STAGE, "seq": u32::MAX}))
        );
    }

    /// A frame that names no stage of the phone's shape is not an upload's
    /// to answer, and a chunk that is not base64 drops its stage.
    #[test]
    fn a_malformed_upload_frame_is_refused() {
        let (_dir, store) = store();
        assert!(handle(&store, 1, "attach_begin", &json!({"stage": "../x", "name": "a", "size": 1})).is_err());
        send(&store, 1, "attach_begin", json!({"name": "a.log", "size": 2}));
        assert_eq!(
            send(&store, 1, "attach_chunk", json!({"seq": 0, "data": "*not base64*"})),
            Some(failed(STAGE, "invalid_request"))
        );
        assert_eq!(chunk(&store, 1, b"ab"), Some(failed(STAGE, "unknown_stage")));
    }
}
