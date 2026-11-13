//! A reply typed into an agent's pane from a screen, as a phone's reply is
//! (`hided/src/mobile/pane.rs`): the pane node's input permit, the sleep
//! gate, then the engine's `submit_text`, the words as they are and then
//! Enter. This file owns that write for the core; a session thread's input
//! box (`session_thread.rs`) and `pane_reply`, the Overview's reply to an
//! agent whose session is not open, both go through it.
//!
//! The runtime notes a key before the write and a key and a submit after it
//! lands (`note_delivery_reply`, `record_operator_submit`), so the doorbell
//! neither types over the words nor takes them for a draft, and the request
//! counts as the operator's own. The write itself runs off the lock, on a
//! worker, and asks the runtime once more, last, that the pane still runs the
//! agent it was meant for.
//!
//! `pane_reply`'s answer is its pane's entry in the `pane_replies` section,
//! named by its request id: `sending`, then `sent` or `refused` with a
//! reason. One reply per pane is out at a time; another is refused as
//! `in_flight`.

use super::*;
use crate::model::{ThreadInputSnapshot, ThreadInputState};
use crate::terminal_attachments as ingress;
use std::collections::HashSet;
use std::time::Duration;

const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// The Overview's reply to an agent (`pane_reply`): `text` and Enter into
/// the pane of `pane_id`, the core pane id of an agent Hide observes.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PaneReplyPayload {
    pub pane_id: String,
    pub request_id: String,
    pub text: String,
}

/// The panes a `pane_reply` is being written to.
#[derive(Default)]
pub(super) struct ReplyWork {
    sending: HashSet<String>,
}

/// What one reply's write needs, taken under the lock and moved to the
/// worker that writes it.
pub(super) struct ReplyWrite {
    engine: crate::engine::SharedEngine,
    terminals: Arc<dyn hide_node_link::terminal::TerminalRoutes>,
    gate: Arc<crate::input_gate::PaneInputGate>,
    core_pane: String,
    raw_pane: String,
}

impl ReplyWrite {
    /// Off the lock: waits for the pane node's room for `text` and its Enter
    /// and for the sleep gate, asks `still` whether the pane is still the
    /// one meant, then sends. A refusal before the engine typed nothing.
    pub(super) fn send(
        &self,
        text: &str,
        still: &dyn Fn() -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        match hide_node_link::terminal::wait_for_input_permit(
            self.terminals.as_ref(),
            &self.core_pane,
            text.len() + 1,
        ) {
            Ok(()) => {}
            Err(hide_node_link::terminal::InputPermitError::Asleep) => return Err("asleep"),
            Err(_) => return Err("input_refused"),
        }
        let _admission = self.gate.admit(&self.core_pane).map_err(|_| "asleep")?;
        // The pane may have moved on while the words waited for their permit
        // or the pane's admission: ask once more, last.
        still()?;
        self.engine
            .submit_text(
                crate::engine::Wait::Each(SEND_TIMEOUT),
                &self.raw_pane,
                text,
            )
            .map_err(|error| match error {
                crate::engine::EngineError::Refused { code, .. } if code.contains("not_found") => {
                    "pane_gone"
                }
                // No answer: the words may have landed.
                crate::engine::EngineError::Unanswered(_) => "uncertain",
                _ => "unavailable",
            })
    }
}

impl Runtime {
    /// Under the lock: the write to `core_pane` (`raw_pane` on `device`), or
    /// why none can start. A write that can start notes its key now, before
    /// the words land, so the doorbell cannot judge the pane clean while
    /// they are landing; a refused write only holds the pane.
    pub(super) fn begin_reply_write(
        &mut self,
        device: &str,
        core_pane: &str,
        raw_pane: &str,
    ) -> Result<ReplyWrite, &'static str> {
        let Some(engine) = self.delivery_engine(device) else {
            return Err(if device == self.node.as_str() {
                "unavailable"
            } else {
                "device_unreachable"
            });
        };
        self.note_delivery_key_at(core_pane, unix_milliseconds());
        Ok(ReplyWrite {
            engine,
            terminals: Arc::clone(&self.terminals),
            gate: Arc::clone(&self.input_gate),
            core_pane: core_pane.to_owned(),
            raw_pane: raw_pane.to_owned(),
        })
    }

    /// A reply landed in `pane_id`: a key and a submit, the operator's own;
    /// `source` names the screen that sent it.
    pub(super) fn note_reply_landed(&mut self, pane_id: &str, source: &'static str) {
        self.note_delivery_reply(pane_id, source);
        self.record_operator_submit(pane_id);
    }

    pub(super) fn send_pane_reply(&mut self, payload: PaneReplyPayload) -> bool {
        if !ingress::valid_request_id(&payload.request_id) {
            return false;
        }
        let target = self.reply_target(&payload.pane_id);
        let refusal = session_thread::input_problem(&payload.text, &[])
            .or(target.as_ref().err().copied())
            .or(self
                .reply_work
                .sending
                .contains(&payload.pane_id)
                .then_some("in_flight"));
        if let Some(reason) = refusal {
            return self.settle_pane_reply(&payload.pane_id, &payload.request_id, Err(reason));
        }
        let Ok((device, raw_pane, terminal)) = target else {
            unreachable!("a missing target is refused above");
        };
        let reply = match self.begin_reply_write(&device, &payload.pane_id, &raw_pane) {
            Ok(reply) => reply,
            Err(reason) => {
                return self.settle_pane_reply(&payload.pane_id, &payload.request_id, Err(reason));
            }
        };
        self.reply_work.sending.insert(payload.pane_id.clone());
        self.set_pane_reply(
            &payload.pane_id,
            &payload.request_id,
            ThreadInputState::Sending,
            None,
        );
        let PaneReplyPayload {
            pane_id,
            request_id,
            text,
        } = payload;
        let Some(context) = self.worker_context.clone() else {
            // Nothing moves between here and the write without a worker.
            let sent = reply.send(&text, &|| Ok(()));
            return self.ingest_pane_reply(&pane_id, &request_id, sent);
        };
        let spawned = thread::Builder::new()
            .name("hide-pane-reply".to_owned())
            .spawn({
                let (pane_id, request_id) = (pane_id.clone(), request_id.clone());
                move || {
                    let still = || {
                        let runtime = context.runtime.upgrade().ok_or("unavailable")?;
                        let runtime = runtime.lock().map_err(|_| "unavailable")?;
                        runtime.reply_still_meant(&pane_id, &raw_pane, terminal.as_deref())
                    };
                    let sent = reply.send(&text, &still);
                    session_thread::apply_in(&context, move |runtime| {
                        runtime.ingest_pane_reply(&pane_id, &request_id, sent)
                    });
                }
            });
        if spawned.is_err() {
            return self.ingest_pane_reply(&pane_id, &request_id, Err("unavailable"));
        }
        true
    }

    /// Where a reply to `pane_id` goes: the device, the pane id the engine
    /// knows and its terminal, of the agent Hide observes there.
    fn reply_target(
        &self,
        pane_id: &str,
    ) -> Result<(String, String, Option<String>), &'static str> {
        let observation = self.delivery_observations.get(pane_id).ok_or("pane_gone")?;
        Ok((
            observation.actor.device_id.clone(),
            observation.raw_pane_id.clone(),
            observation.terminal_id.clone(),
        ))
    }

    /// Whether `pane_id` still runs, in the same pane and terminal, the agent
    /// a reply was meant for.
    fn reply_still_meant(
        &self,
        pane_id: &str,
        raw_pane: &str,
        terminal: Option<&str>,
    ) -> Result<(), &'static str> {
        let observation = self.delivery_observations.get(pane_id).ok_or("pane_gone")?;
        if observation.raw_pane_id != raw_pane || observation.terminal_id.as_deref() != terminal {
            return Err("pane_changed");
        }
        Ok(())
    }

    fn ingest_pane_reply(
        &mut self,
        pane_id: &str,
        request_id: &str,
        sent: Result<(), &'static str>,
    ) -> bool {
        self.reply_work.sending.remove(pane_id);
        if sent.is_ok() {
            self.note_reply_landed(pane_id, "pane_reply");
        }
        self.settle_pane_reply(pane_id, request_id, sent)
    }

    fn settle_pane_reply(
        &mut self,
        pane_id: &str,
        request_id: &str,
        sent: Result<(), &'static str>,
    ) -> bool {
        if let Err(reason) = sent {
            crate::diagnostic!(serde_json::json!({
                "kind": "pane_reply.refused", "pane_id": pane_id,
                "request_id": request_id, "reason": reason,
            }));
        }
        let (state, reason) = match sent {
            Ok(()) => (ThreadInputState::Sent, None),
            Err(reason) => (ThreadInputState::Refused, Some(reason)),
        };
        self.set_pane_reply(pane_id, request_id, state, reason)
    }

    /// Replaces `pane_id`'s entry, as a new section so a payload already
    /// handed out keeps the state it was taken at, and drops the entries of
    /// panes Hide no longer observes. An unchanged section is not published.
    fn set_pane_reply(
        &mut self,
        pane_id: &str,
        request_id: &str,
        state: ThreadInputState,
        reason: Option<&str>,
    ) -> bool {
        let mut next = self
            .snapshot
            .pane_replies
            .as_deref()
            .cloned()
            .unwrap_or_default();
        next.replies
            .retain(|pane, _| pane == pane_id || self.delivery_observations.contains_key(pane));
        next.replies.insert(
            pane_id.to_owned(),
            ThreadInputSnapshot {
                request_id: request_id.to_owned(),
                state,
                reason: reason.map(str::to_owned),
            },
        );
        if self.snapshot.pane_replies.as_deref() == Some(&next) {
            return false;
        }
        self.snapshot.pane_replies = Some(Arc::new(next));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    const REPLY: &str = "4f1c2b3a-5d6e-4f70-8a9b-0c1d2e3f4a5b";
    const AGAIN: &str = "9a8b7c6d-5e4f-4a3b-8c2d-1e0f9a8b7c6d";

    /// A pane node that grants every input permit, or holds it for the test
    /// to grant once `hold` is set, and remembers how many bytes each asked for.
    #[derive(Default)]
    struct GrantingTerminals {
        permits: Mutex<Vec<(String, usize)>>,
        hold: AtomicBool,
        held: Mutex<Option<hide_node_link::terminal::InputPermitReply>>,
    }

    impl hide_node_link::terminal::TerminalNode for GrantingTerminals {
        fn control(&self, _: hide_node_link::terminal::TerminalControl) {}
        fn key(&self, _: hide_node_link::terminal::KeyTarget, _: Vec<u8>, _: u64) {
            unreachable!("a reply never types keys");
        }
        fn view(&self, _: &str, _: hide_node_link::terminal::GridSize, _: bool) {
            unreachable!("a reply never attaches its pane");
        }
        fn redraw(&self, _: &str) {}
        fn input_permit(
            &self,
            pane: &str,
            bytes: usize,
            reply: hide_node_link::terminal::InputPermitReply,
        ) {
            self.permits.lock().unwrap().push((pane.to_owned(), bytes));
            if self.hold.load(Ordering::SeqCst) {
                *self.held.lock().unwrap() = Some(reply);
            } else {
                reply(Ok(()));
            }
        }
    }

    impl hide_node_link::terminal::TerminalRoutes for GrantingTerminals {
        fn install_device(&self, _: &str, _: Arc<dyn hide_node_link::terminal::TerminalNode>) {}
        fn remove_device(&self, _: &str) {}
        fn terminals_unstarted(&self, _: &str, _: &str) {}
    }

    /// The engine's session read of this machine: one Claude pane asking a
    /// question in terminal `terminal`.
    fn observe(runtime: &mut Runtime, terminal: &str) {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [{"id": "worker", "pane_id": "w1:p1", "agent": "claude",
                "agent_status": "blocked", "state_change_seq": 1,
                "lineage_session": "s", "cwd": "/tmp/project"}],
            "panes": [{"pane_id": "w1:p1", "terminal_id": terminal}],
        }))
        .unwrap();
        runtime.observe_delivery_laid(crate::node::TEST_NODE, &payload, None, None);
    }

    fn reply(pane: &str, request: &str, text: &str) -> PaneReplyPayload {
        PaneReplyPayload {
            pane_id: pane.to_owned(),
            request_id: request.to_owned(),
            text: text.to_owned(),
        }
    }

    fn answer(runtime: &Runtime, pane: &str) -> Option<(String, ThreadInputState, Option<String>)> {
        let entry = runtime
            .snapshot
            .pane_replies
            .as_ref()?
            .replies
            .get(pane)?
            .clone();
        Some((entry.request_id, entry.state, entry.reason))
    }

    /// The words and Enter go to the agent's pane through the engine as a
    /// phone's reply does, the answer names the request as sent, and the
    /// pane takes a submit, the operator's own.
    #[test]
    fn a_reply_reaches_the_agents_pane_and_reads_as_sent_and_submitted() {
        let mut runtime = crate::runtime::tests::runtime();
        let terminals = Arc::new(GrantingTerminals::default());
        runtime.set_terminals(Arc::clone(&terminals) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            assert_eq!(method, "submit_text");
            crate::engine::fake::reply(())
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "t1");
        let before = runtime.delivery_observations["w1:p1"].input_generation;

        assert!(runtime.send_pane_reply(reply("w1:p1", REPLY, "네, 그렇게 해 주세요")));

        let sent = fake.calls_of("submit_text");
        assert_eq!(sent.len(), 1, "{sent:?}");
        assert_eq!(sent[0]["pane"], "w1:p1");
        assert_eq!(sent[0]["text"], "네, 그렇게 해 주세요");
        assert_eq!(
            terminals.permits.lock().unwrap().as_slice(),
            &[("w1:p1".to_owned(), "네, 그렇게 해 주세요".len() + 1)]
        );
        assert_eq!(
            answer(&runtime, "w1:p1"),
            Some((REPLY.to_owned(), ThreadInputState::Sent, None))
        );
        // A key noted before the write and the reply's key and submit after it,
        // counted rather than timed: both can land in the same millisecond.
        assert_eq!(runtime.delivery_observations["w1:p1"].input_generation, before + 2);
    }

    /// A reply that cannot reach the pane types nothing, or nothing more,
    /// and its answer keeps its request id with the reason, so the field
    /// that sent it keeps the words and says why.
    #[test]
    fn a_reply_that_cannot_land_is_refused_by_its_request_with_the_reason() {
        let mut runtime = crate::runtime::tests::runtime();
        runtime.set_terminals(Arc::new(GrantingTerminals::default()) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            assert_eq!(method, "submit_text");
            crate::engine::fake::refused("pane_not_found", "no such pane")
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "t1");
        let refused = |reason: &str, request: &str| {
            Some((
                request.to_owned(),
                ThreadInputState::Refused,
                Some(reason.to_owned()),
            ))
        };

        runtime.send_pane_reply(reply("w1:p1", REPLY, "   "));
        assert_eq!(answer(&runtime, "w1:p1"), refused("empty", REPLY));
        runtime.send_pane_reply(reply("w1:p1", REPLY, "멈춰\u{1b}[A"));
        assert_eq!(
            answer(&runtime, "w1:p1"),
            refused("control_characters", REPLY)
        );
        runtime.send_pane_reply(reply("w9:p9", AGAIN, "계속"));
        assert_eq!(answer(&runtime, "w9:p9"), refused("pane_gone", AGAIN));
        assert!(
            fake.calls_of("submit_text").is_empty(),
            "nothing reached the engine"
        );

        // The core still sees the pane, the engine does not.
        runtime.send_pane_reply(reply("w1:p1", AGAIN, "계속"));
        assert_eq!(fake.calls_of("submit_text").len(), 1);
        assert_eq!(answer(&runtime, "w1:p1"), refused("pane_gone", AGAIN));

        // A request id the core cannot name an answer by is not taken.
        assert!(!runtime.send_pane_reply(reply("w1:p1", "not-a-uuid", "계속")));
        assert_eq!(answer(&runtime, "w1:p1"), refused("pane_gone", AGAIN));
    }

    /// While one reply waits for its permit, a second to the same pane is
    /// refused as in flight; the pane is asked once more just before the
    /// words go, so a terminal that changed meanwhile gets nothing.
    #[test]
    fn a_reply_whose_pane_changes_while_it_waits_is_never_typed() {
        let mut runtime = crate::runtime::tests::runtime();
        let terminals = Arc::new(GrantingTerminals::default());
        terminals.hold.store(true, Ordering::SeqCst);
        runtime.set_terminals(Arc::clone(&terminals) as _);
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            assert_eq!(method, "submit_text");
            crate::engine::fake::reply(())
        });
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(&fake));
        observe(&mut runtime, "t1");
        let shared = crate::runtime::tests::SharedRuntime::new(runtime);
        {
            let mut runtime = shared.lock().unwrap();
            runtime.worker_context = Some(RuntimeWorkerContext {
                runtime: shared.weak(),
                notifier: crate::handle::ChangeNotifier::noop(),
            });
            runtime.send_pane_reply(reply("w1:p1", REPLY, "계속"));
            assert_eq!(
                answer(&runtime, "w1:p1"),
                Some((REPLY.to_owned(), ThreadInputState::Sending, None))
            );
        }
        crate::runtime::tests::wait_for("the reply's permit ask", || {
            terminals.held.lock().unwrap().is_some()
        });
        shared
            .lock()
            .unwrap()
            .send_pane_reply(reply("w1:p1", AGAIN, "또"));
        assert_eq!(
            answer(&shared.lock().unwrap(), "w1:p1"),
            Some((
                AGAIN.to_owned(),
                ThreadInputState::Refused,
                Some("in_flight".to_owned())
            ))
        );
        observe(&mut shared.lock().unwrap(), "t2");
        let grant = terminals.held.lock().unwrap().take().unwrap();
        grant(Ok(()));
        crate::runtime::tests::wait_for("the reply's answer", || {
            answer(&shared.lock().unwrap(), "w1:p1").is_some_and(|(request, _, _)| request == REPLY)
        });
        assert!(fake.calls_of("submit_text").is_empty());
        assert_eq!(
            answer(&shared.lock().unwrap(), "w1:p1"),
            Some((
                REPLY.to_owned(),
                ThreadInputState::Refused,
                Some("pane_changed".to_owned())
            ))
        );
    }
}
