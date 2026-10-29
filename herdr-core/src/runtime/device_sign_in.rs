//! What a device's connection attempt waits on from the operator, and how
//! their answer reaches it (PRD system-ssh-device-link D-05, D-06, D-08,
//! D-10, B4-B15, B27).
//!
//! The attempt runs on its worker, off the runtime lock; the transport
//! hands it a [`RowSignIn`], which puts what ssh asks on the device's row
//! and blocks until the operator answers, cancels, or five minutes pass. The
//! answer travels from the dispatch that carries it to the waiting attempt
//! over a channel of that one question, never through the snapshot, a file
//! or a log (B10). An attempt nobody may be asked for (`Wake::NotAllowed`,
//! Hide's own retries) is refused at once and shows nothing (B14).

use std::sync::mpsc;
use std::time::Duration;

use serde::Deserialize;

use super::*;
use crate::model::DeviceSignInSnapshot;
use crate::remote::Wake;
use hide_node_link::device::{Answer, Refusal, Secret, SignIn, SignInWait};

/// How long a question waits for the operator (D-05).
const ANSWER_WITHIN: Duration = Duration::from_secs(5 * 60);

/// What one device's current attempt waits on.
pub(super) struct SignInState {
    /// The number an answer names, so an answer typed for a question ssh no
    /// longer asks never answers the next one (B7).
    question: u64,
    wait: SignInWait,
    /// Where the operator's answer goes; `None` for a wait that ends by
    /// itself (queued, a browser check, a security key's touch).
    answer: Option<mpsc::SyncSender<Result<Answer, Refusal>>>,
}

/// The operator's reply to a question on the device row. Its `Debug` shows
/// nothing typed.
#[derive(Deserialize)]
#[serde(tag = "reply", rename_all = "snake_case")]
pub(super) enum SignInReply {
    Text { text: String },
    Accept,
    Cancel,
}

impl std::fmt::Debug for SignInReply {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Text { .. } => "Text(..)",
            Self::Accept => "Accept",
            Self::Cancel => "Cancel",
        })
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct DeviceSignInPayload {
    pub(super) device_id: String,
    pub(super) question: u64,
    #[serde(flatten)]
    pub(super) reply: SignInReply,
}

/// The row's side of one attempt, which the transport blocks on.
pub(super) struct RowSignIn {
    pub(super) context: RuntimeWorkerContext,
    pub(super) device_id: String,
    pub(super) generation: u64,
    pub(super) wake: Wake,
}

impl RowSignIn {
    /// Runs `apply` under the runtime lock and announces a change after it.
    fn with_runtime<T>(&self, apply: impl FnOnce(&mut Runtime) -> (bool, T)) -> Option<T> {
        let runtime = self.context.runtime.upgrade()?;
        let (changed, value) = {
            let mut guard = runtime.lock().ok()?;
            apply(&mut guard)
        };
        drop(runtime);
        if changed {
            self.context.notifier.notify();
        }
        Some(value)
    }
}

impl SignIn for RowSignIn {
    fn show(&self, wait: Option<SignInWait>) {
        // A touch needs the operator, so an attempt nobody may be asked for
        // shows none (B14); a queued attempt is shown whoever started it.
        if self.wake == Wake::NotAllowed
            && wait
                .as_ref()
                .is_some_and(|wait| !matches!(wait, SignInWait::Queued))
        {
            return;
        }
        self.with_runtime(|runtime| {
            (
                runtime.show_device_sign_in(&self.device_id, self.generation, wait),
                (),
            )
        });
    }

    fn ask(&self, wait: SignInWait) -> Result<Answer, Refusal> {
        if self.wake == Wake::NotAllowed {
            return Err(Refusal::NotAllowed);
        }
        let (sender, answers) = mpsc::sync_channel(1);
        let question = self
            .with_runtime(|runtime| {
                match runtime.ask_device_sign_in(&self.device_id, self.generation, wait, sender) {
                    Some(question) => (true, Some(question)),
                    None => (false, None),
                }
            })
            .flatten()
            .ok_or(Refusal::Ended)?;
        match answers.recv_timeout(ANSWER_WITHIN) {
            Ok(answer) => answer,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.with_runtime(|runtime| {
                    (runtime.end_device_question(&self.device_id, question), ())
                });
                Err(Refusal::TimedOut)
            }
            // The attempt's state went with a newer attempt, a close or the
            // device's removal.
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(Refusal::Ended),
        }
    }
}

impl Runtime {
    /// What the device's attempt now waits on that takes no answer, or
    /// `None` once it waits on nothing. A question waiting for its answer
    /// stays: it ends with its answer.
    fn show_device_sign_in(
        &mut self,
        device_id: &str,
        generation: u64,
        wait: Option<SignInWait>,
    ) -> bool {
        let question = self.next_sign_in_question();
        let Some(host) = self
            .device_hosts
            .get_mut(device_id)
            .filter(|host| host.generation == generation)
        else {
            return false;
        };
        if host
            .sign_in
            .as_ref()
            .is_some_and(|state| state.answer.is_some())
        {
            return false;
        }
        host.sign_in = wait.map(|wait| SignInState {
            question,
            wait,
            answer: None,
        });
        self.refresh_device_snapshots()
    }

    /// Puts a question of the device's attempt on its row; `None` when the
    /// attempt is no longer the device's.
    fn ask_device_sign_in(
        &mut self,
        device_id: &str,
        generation: u64,
        wait: SignInWait,
        answer: mpsc::SyncSender<Result<Answer, Refusal>>,
    ) -> Option<u64> {
        let question = self.next_sign_in_question();
        let host = self
            .device_hosts
            .get_mut(device_id)
            .filter(|host| host.generation == generation)?;
        crate::diagnostic!(serde_json::json!({
            "component": "remote_host",
            "kind": "host.sign_in_asked",
            "target": device_id,
            "generation": generation,
            "question": question,
            "asks": wait.kind(),
        }));
        host.sign_in = Some(SignInState {
            question,
            wait,
            answer: Some(answer),
        });
        self.refresh_device_snapshots();
        Some(question)
    }

    /// The question went unanswered; the row stops showing it.
    fn end_device_question(&mut self, device_id: &str, question: u64) -> bool {
        let Some(host) = self.device_hosts.get_mut(device_id) else {
            return false;
        };
        if host
            .sign_in
            .as_ref()
            .is_none_or(|state| state.question != question)
        {
            return false;
        }
        host.sign_in = None;
        self.refresh_device_snapshots()
    }

    /// The operator's answer to the question the row shows. An answer for a
    /// question ssh no longer asks reaches nothing; the row already shows
    /// what it asks now.
    pub(super) fn answer_device_sign_in(&mut self, payload: DeviceSignInPayload) -> bool {
        let device_id = payload.device_id.trim();
        let Some(host) = self.device_hosts.get_mut(device_id) else {
            return false;
        };
        let pending = host
            .sign_in
            .as_ref()
            .is_some_and(|state| state.question == payload.question && state.answer.is_some());
        let reply = match &payload.reply {
            SignInReply::Text { .. } => "text",
            SignInReply::Accept => "accept",
            SignInReply::Cancel => "cancel",
        };
        crate::diagnostic!(serde_json::json!({
            "component": "remote_host",
            "kind": if pending { "host.sign_in_answered" } else { "host.sign_in_answer_stale" },
            "target": device_id,
            "question": payload.question,
            "reply": reply,
        }));
        if !pending {
            return false;
        }
        let Some(SignInState {
            answer: Some(answer),
            ..
        }) = host.sign_in.take()
        else {
            return false;
        };
        let _ = answer.send(match payload.reply {
            SignInReply::Text { text } => Ok(Answer::Text(Secret::new(text))),
            SignInReply::Accept => Ok(Answer::Accept),
            SignInReply::Cancel => Err(Refusal::Cancelled),
        });
        self.refresh_device_snapshots()
    }

    fn next_sign_in_question(&mut self) -> u64 {
        self.last_sign_in_question += 1;
        self.last_sign_in_question
    }

    /// What the device row shows its attempt waiting on.
    pub(super) fn device_sign_in_snapshot(&self, device_id: &str) -> Option<DeviceSignInSnapshot> {
        let state = self.device_hosts.get(device_id)?.sign_in.as_ref()?;
        let key_command = match &state.wait {
            SignInWait::Secret {
                key_file: Some(file),
                ..
            } => Some(add_key_command(file)),
            _ => None,
        };
        Some(DeviceSignInSnapshot {
            question: state.question,
            wait: state.wait.clone(),
            key_command,
        })
    }
}

/// The command that adds a key to the agent of the machine ssh runs on, so
/// its passphrase is typed once rather than on every connection (D-07,
/// B9).
fn add_key_command(file: &str) -> String {
    let quoted = if file
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "/._-~+@:".contains(character))
    {
        file.to_owned()
    } else {
        format!("'{}'", file.replace('\'', r"'\''"))
    };
    if cfg!(target_os = "macos") {
        format!("ssh-add --apple-use-keychain {quoted}")
    } else {
        format!("ssh-add {quoted}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_add_key_command_quotes_a_file_a_shell_would_split() {
        let plain = add_key_command("/home/al/.ssh/id_ed25519");
        let spaced = add_key_command("/Users/example/a b/.ssh/it's");
        if cfg!(target_os = "macos") {
            assert_eq!(
                plain,
                "ssh-add --apple-use-keychain /home/al/.ssh/id_ed25519"
            );
            assert_eq!(
                spaced,
                r"ssh-add --apple-use-keychain '/Users/example/a b/.ssh/it'\''s'"
            );
        } else {
            assert_eq!(plain, "ssh-add /home/al/.ssh/id_ed25519");
            assert_eq!(spaced, r"ssh-add '/Users/example/a b/.ssh/it'\''s'");
        }
    }

    #[test]
    fn a_reply_names_nothing_typed_in_its_debug() {
        let payload: DeviceSignInPayload = serde_json::from_value(serde_json::json!({
            "device_id": "d",
            "question": 3,
            "reply": "text",
            "text": "hunter2",
        }))
        .unwrap();
        assert!(!format!("{payload:?}").contains("hunter2"));
    }
}
