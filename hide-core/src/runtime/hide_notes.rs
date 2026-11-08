//! Notes Hide sends an agent about one of its letters: who sends them, the
//! thread they go on, and what a note that cannot go leaves in the log. The
//! wake notes (`agent_sleep::letters`) and the unreadable-session notes
//! (`unreadable_letters`) send this way; each decides what to say and when.

use serde_json::json;

use super::Runtime;
use super::delivery::Observation;

/// How long one note may take to land.
const NOTE_CALL: std::time::Duration = std::time::Duration::from_secs(10);

/// A Hide letter to the sender of `letter`.
pub(in crate::runtime) struct Note {
    pub(in crate::runtime) letter: String,
    pub(in crate::runtime) target: Observation,
    pub(in crate::runtime) body: String,
}

/// One kind of note: the Hide role that sends it, its thread, the intent
/// prefix that makes a note once per letter, and the diagnostic a note that
/// cannot go writes.
pub(in crate::runtime) struct NoteKind {
    pub(in crate::runtime) role: &'static str,
    pub(in crate::runtime) thread: &'static str,
    pub(in crate::runtime) intent: &'static str,
    pub(in crate::runtime) component: &'static str,
    pub(in crate::runtime) unsent: &'static str,
    /// Delivered at the sender's next safe gap while it works (`--now`),
    /// for a note it must act on at once.
    pub(in crate::runtime) now: bool,
}

impl Runtime {
    /// Sends `notes` from Hide's `kind.role`, one after another, on a thread
    /// of their own, and answers whether that thread started. `ended` runs
    /// under the runtime lock when the thread ends, a panicking send
    /// included, so the caller never thinks a thread still sends. A runtime
    /// without a worker (a test) sends nothing.
    pub(in crate::runtime) fn send_hide_notes(
        &mut self,
        kind: NoteKind,
        notes: Vec<Note>,
        ended: fn(&mut Runtime),
    ) -> bool {
        let Some(context) = self.worker_context.clone() else {
            return false;
        };
        let (client, authority, actor) = match self.hide_delivery(kind.role) {
            Ok(delivery) => delivery,
            Err(code) => {
                crate::diagnostic!(json!({
                    "component": kind.component, "kind": kind.unsent,
                    "count": notes.len(), "code": code,
                }));
                return false;
            }
        };
        let runtime = context.runtime.clone();
        let spawned = std::thread::Builder::new()
            .name(kind.thread.to_owned())
            .spawn(move || {
                struct Ending(std::sync::Weak<std::sync::Mutex<Runtime>>, fn(&mut Runtime));
                impl Drop for Ending {
                    fn drop(&mut self) {
                        if let Some(runtime) = self.0.upgrade() {
                            let mut guard =
                                runtime.lock().unwrap_or_else(|poison| poison.into_inner());
                            (self.1)(&mut guard);
                        }
                    }
                }
                let _ending = Ending(runtime, ended);
                for note in notes {
                    let prepared = crate::delivery::worker::Prepared::new(
                        client.clone(),
                        crate::delivery::worker::Authority {
                            caller: authority.caller.clone(),
                            context: authority.context.clone(),
                            also_reads: Vec::new(),
                        },
                        actor.clone(),
                        Some(note.target.clone()),
                        crate::delivery::Command::Send {
                            target: note.target.actor.pane_id.clone(),
                            intent: format!("{}:{}", kind.intent, note.letter),
                            body: note.body,
                            // Hide reads no answer, and the note wakes no one.
                            kind: "progress".into(),
                            now: kind.now,
                            force: false,
                        },
                    );
                    if let Err(code) = prepared.run(NOTE_CALL) {
                        crate::diagnostic!(json!({
                            "component": kind.component, "kind": kind.unsent,
                            "letter_id": note.letter, "code": code,
                        }));
                    }
                }
            });
        match spawned {
            Ok(_) => true,
            Err(error) => {
                crate::diagnostic!(json!({
                    "component": kind.component, "kind": kind.unsent,
                    "code": "thread_not_started", "message": error.to_string(),
                }));
                false
            }
        }
    }
}
