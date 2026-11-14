//! A reply typed into an agent's pane from a screen, as a phone's reply is
//! (`hided/src/mobile/pane.rs`): the pane node's input permit, the sleep
//! gate, then the engine's `submit_text`, the words as they are and then
//! Enter. This file owns that write for the core, and the one admission in
//! front of it: a session thread's input box (`session_thread.rs`) and
//! `pane_reply`, the Overview's reply to an agent whose session is not open,
//! both take the pane's one writing slot, so two replies never interleave in
//! one pane (the engine types the words and the Enter as two calls, and the
//! sleep gate counts writers rather than ordering them).
//!
//! The runtime notes a key before the write and a key and a submit after it
//! lands (`note_delivery_reply`, `record_operator_submit`), so the doorbell
//! neither types over the words nor takes them for a draft, and the request
//! counts as the operator's own. The write itself runs off the lock, on a
//! worker, and asks the runtime once more, last, that the pane still runs
//! the agent it was meant for: the same engine pane, terminal, agent kind
//! and session.
//!
//! `pane_reply` names the session the screen drew (its digest), and is
//! refused as `session_changed` when the pane runs another one. Its answer
//! is its own receipt in the `pane_replies` section, by request id:
//! `sending`, then `sent` or `refused` with a reason. A request id the core
//! already holds a receipt for is never written again; its receipt stands.
//! The section holds at most [`RECEIPTS`] receipts. Admission keeps at most
//! [`PENDING`], half of them, still sending (a reply past that is refused as
//! `busy` before anything is asked), so every receipt still sending stays
//! and at least as many settled ones fit beside them; past the bound the
//! oldest settled receipt goes, so every screen finds its own answer in a
//! coalesced frame. A screen that finds none for a reply it sent (a restart,
//! a lost event) reads the result as unknown rather than waiting for it.

use super::*;
use crate::model::{PaneReplyReceipt, ThreadInputState};
use crate::terminal_attachments as ingress;
use std::collections::HashSet;
use std::time::Duration;

const SEND_TIMEOUT: Duration = Duration::from_secs(5);

/// The receipts `pane_replies` holds at most (the contract's `maxItems`).
pub(super) const RECEIPTS: usize = 64;
/// The receipts that may be sending at once (one per pane, as each pane has
/// one writing slot): a reply past it is refused as `busy`, so a sending
/// receipt is never dropped and the newest settled ones keep the rest.
pub(super) const PENDING: usize = RECEIPTS / 2;

/// The Overview's reply to an agent (`pane_reply`): `text` and Enter into
/// the pane of `pane_id`, the core pane id of an agent Hide observes, while
/// it still runs `session`, the digest of the session the screen drew there
/// (`lineage_session`; none when the screen drew none).
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PaneReplyPayload {
    pub pane_id: String,
    pub request_id: String,
    pub text: String,
    #[serde(default)]
    pub session: Option<String>,
}

/// The panes a reply is being written to, by either screen.
#[derive(Default)]
pub(super) struct ReplyWork {
    writing: HashSet<String>,
}

/// The agent a reply is meant for: where its pane runs and which session.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ReplyTarget {
    device: String,
    raw_pane: String,
    terminal: Option<String>,
    kind: String,
    session: Option<String>,
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
    /// Under the lock: takes `core_pane`'s one writing slot and the write to
    /// it (`raw_pane` on `device`), or says why none can start (`in_flight`
    /// while another reply is being written there). A write that can start
    /// notes its key now, before the words land, so the doorbell cannot judge
    /// the pane clean while they are landing. The caller gives the slot back
    /// with [`Runtime::end_reply_write`] on every path.
    pub(super) fn begin_reply_write(
        &mut self,
        device: &str,
        core_pane: &str,
        raw_pane: &str,
    ) -> Result<ReplyWrite, &'static str> {
        if self.reply_work.writing.contains(core_pane) {
            return Err("in_flight");
        }
        let Some(engine) = self.delivery_engine(device) else {
            return Err(if device == self.node.as_str() {
                "unavailable"
            } else {
                "device_unreachable"
            });
        };
        self.reply_work.writing.insert(core_pane.to_owned());
        self.note_delivery_key_at(core_pane, unix_milliseconds());
        Ok(ReplyWrite {
            engine,
            terminals: Arc::clone(&self.terminals),
            gate: Arc::clone(&self.input_gate),
            core_pane: core_pane.to_owned(),
            raw_pane: raw_pane.to_owned(),
        })
    }

    /// Gives `core_pane`'s writing slot back once its write answered, and,
    /// when the reply landed, notes a key and a submit, the operator's own;
    /// `source` names the screen that sent it.
    pub(super) fn end_reply_write(
        &mut self,
        core_pane: &str,
        sent: Result<(), &'static str>,
        source: &'static str,
    ) {
        self.reply_work.writing.remove(core_pane);
        if sent.is_ok() {
            self.note_delivery_reply(core_pane, source);
            self.record_operator_submit(core_pane);
        }
    }

    pub(super) fn send_pane_reply(&mut self, payload: PaneReplyPayload) -> bool {
        if !ingress::valid_request_id(&payload.request_id) {
            return false;
        }
        if self.pane_reply_receipt(&payload.request_id).is_some() {
            // A repeat, while its write runs or after it answered: the
            // receipt the core holds is the answer, and nothing is typed again.
            crate::diagnostic!(serde_json::json!({
                "kind": "pane_reply.repeated", "pane_id": payload.pane_id,
                "request_id": payload.request_id,
            }));
            return false;
        }
        let target = self.reply_target(&payload.pane_id).and_then(|target| {
            if target.session == payload.session {
                Ok(target)
            } else {
                Err("session_changed")
            }
        });
        let refusal = session_thread::input_problem(&payload.text, &[])
            .map(Err)
            .unwrap_or(target);
        let target = match refusal.and_then(|target| {
            if self.pending_pane_replies() < PENDING {
                Ok(target)
            } else {
                Err("busy")
            }
        }) {
            Ok(target) => target,
            Err(reason) => {
                return self.settle_pane_reply(&payload.pane_id, &payload.request_id, Err(reason));
            }
        };
        let reply = match self.begin_reply_write(&target.device, &payload.pane_id, &target.raw_pane)
        {
            Ok(reply) => reply,
            Err(reason) => {
                return self.settle_pane_reply(&payload.pane_id, &payload.request_id, Err(reason));
            }
        };
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
            ..
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
                        runtime.reply_still_meant(&pane_id, &target)
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

    /// The agent Hide observes in `pane_id`, a reply's target.
    fn reply_target(&self, pane_id: &str) -> Result<ReplyTarget, &'static str> {
        let observation = self.delivery_observations.get(pane_id).ok_or("pane_gone")?;
        Ok(ReplyTarget {
            device: observation.actor.device_id.clone(),
            raw_pane: observation.raw_pane_id.clone(),
            terminal: observation.terminal_id.clone(),
            kind: observation.actor.kind.clone(),
            session: observation.actor.session.clone(),
        })
    }

    /// Whether `pane_id` still runs `meant`: the same engine pane and
    /// terminal (else `pane_changed`), the same agent and session (else
    /// `session_changed`).
    fn reply_still_meant(&self, pane_id: &str, meant: &ReplyTarget) -> Result<(), &'static str> {
        let now = self.reply_target(pane_id)?;
        if now.raw_pane != meant.raw_pane
            || now.terminal != meant.terminal
            || now.device != meant.device
        {
            return Err("pane_changed");
        }
        if now.kind != meant.kind || now.session != meant.session {
            return Err("session_changed");
        }
        Ok(())
    }

    fn ingest_pane_reply(
        &mut self,
        pane_id: &str,
        request_id: &str,
        sent: Result<(), &'static str>,
    ) -> bool {
        self.end_reply_write(pane_id, sent, "pane_reply");
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

    fn pending_pane_replies(&self) -> usize {
        self.snapshot.pane_replies.as_deref().map_or(0, |section| {
            section
                .receipts
                .iter()
                .filter(|held| held.state == ThreadInputState::Sending)
                .count()
        })
    }

    fn pane_reply_receipt(&self, request_id: &str) -> Option<&PaneReplyReceipt> {
        self.snapshot
            .pane_replies
            .as_deref()?
            .receipts
            .iter()
            .find(|receipt| receipt.request_id == request_id)
    }

    /// Writes `request_id`'s receipt, as a new section so a payload already
    /// handed out keeps the state it was taken at: a new receipt goes last,
    /// a settled one keeps its place, and past [`RECEIPTS`] the oldest
    /// settled receipt goes. An unchanged section is not published.
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
        let receipt = PaneReplyReceipt {
            request_id: request_id.to_owned(),
            pane_id: pane_id.to_owned(),
            state,
            reason: reason.map(str::to_owned),
        };
        match next
            .receipts
            .iter_mut()
            .find(|held| held.request_id == request_id)
        {
            Some(held) => *held = receipt,
            None => next.receipts.push(receipt),
        }
        while next.receipts.len() > RECEIPTS {
            let Some(oldest) = next
                .receipts
                .iter()
                .position(|held| held.state != ThreadInputState::Sending)
            else {
                break;
            };
            next.receipts.remove(oldest);
        }
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
    /// question in terminal `terminal`, running session `session`.
    fn observe(runtime: &mut Runtime, terminal: &str, session: &str) {
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({
            "agents": [{"id": "worker", "pane_id": "w1:p1", "agent": "claude",
                "agent_status": "blocked", "state_change_seq": 1,
                "lineage_session": session, "cwd": "/tmp/project"}],
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
            session: Some("s".to_owned()),
        }
    }

    fn answer(
        runtime: &Runtime,
        request: &str,
    ) -> Option<(String, ThreadInputState, Option<String>)> {
        let receipt = runtime.pane_reply_receipt(request)?.clone();
        Some((receipt.pane_id, receipt.state, receipt.reason))
    }

    fn refused(reason: &str) -> Option<(String, ThreadInputState, Option<String>)> {
        Some((
            "w1:p1".to_owned(),
            ThreadInputState::Refused,
            Some(reason.to_owned()),
        ))
    }

    fn live(
        fake: &Arc<crate::engine::fake::FakeEngine<'static>>,
        terminals: &Arc<GrantingTerminals>,
    ) -> Runtime {
        let mut runtime = crate::runtime::tests::runtime();
        runtime.set_terminals(Arc::clone(terminals) as _);
        crate::runtime::delivery::tests::set_live(&mut runtime, Some(fake));
        observe(&mut runtime, "t1", "s");
        runtime
    }

    /// The words and Enter go to the agent's pane through the engine as a
    /// phone's reply does, the receipt names the request as sent, and the
    /// pane takes a key before and a key and a submit after, the operator's
    /// own; the same request again types nothing more.
    #[test]
    fn a_reply_reaches_the_agents_pane_once_and_reads_as_sent_and_submitted() {
        let terminals = Arc::new(GrantingTerminals::default());
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            assert_eq!(method, "submit_text");
            crate::engine::fake::reply(())
        });
        let mut runtime = live(&fake, &terminals);
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
            answer(&runtime, REPLY),
            Some(("w1:p1".to_owned(), ThreadInputState::Sent, None))
        );
        // Counted rather than timed: both can land in the same millisecond.
        assert_eq!(
            runtime.delivery_observations["w1:p1"].input_generation,
            before + 2
        );

        // A replay of the settled request is answered by its receipt alone.
        assert!(!runtime.send_pane_reply(reply("w1:p1", REPLY, "네, 그렇게 해 주세요")));
        assert_eq!(fake.calls_of("submit_text").len(), 1);
        assert_eq!(
            runtime.delivery_observations["w1:p1"].input_generation,
            before + 2
        );
        assert_eq!(
            answer(&runtime, REPLY),
            Some(("w1:p1".to_owned(), ThreadInputState::Sent, None))
        );
    }

    /// A reply that cannot reach the agent it was written for types nothing,
    /// or nothing more, and its receipt keeps the reason.
    #[test]
    fn a_reply_that_cannot_land_is_refused_by_its_request_with_the_reason() {
        let terminals = Arc::new(GrantingTerminals::default());
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            assert_eq!(method, "submit_text");
            crate::engine::fake::refused("pane_not_found", "no such pane")
        });
        let mut runtime = live(&fake, &terminals);
        let ids = [
            "00000000-0000-4000-8000-000000000001",
            "00000000-0000-4000-8000-000000000002",
            "00000000-0000-4000-8000-000000000003",
            "00000000-0000-4000-8000-000000000004",
        ];

        runtime.send_pane_reply(reply("w1:p1", ids[0], "   "));
        assert_eq!(answer(&runtime, ids[0]), refused("empty"));
        runtime.send_pane_reply(reply("w1:p1", ids[1], "멈춰\u{1b}[A"));
        assert_eq!(answer(&runtime, ids[1]), refused("control_characters"));
        // The screen drew another session than the one the pane runs now.
        runtime.send_pane_reply(PaneReplyPayload {
            session: Some("earlier".to_owned()),
            ..reply("w1:p1", ids[2], "계속")
        });
        assert_eq!(answer(&runtime, ids[2]), refused("session_changed"));
        runtime.send_pane_reply(reply("w9:p9", AGAIN, "계속"));
        assert_eq!(
            answer(&runtime, AGAIN),
            Some((
                "w9:p9".to_owned(),
                ThreadInputState::Refused,
                Some("pane_gone".to_owned())
            ))
        );
        assert!(
            fake.calls_of("submit_text").is_empty(),
            "nothing reached the engine"
        );

        // The core still sees the pane, the engine does not.
        runtime.send_pane_reply(reply("w1:p1", ids[3], "계속"));
        assert_eq!(fake.calls_of("submit_text").len(), 1);
        assert_eq!(answer(&runtime, ids[3]), refused("pane_gone"));

        // A request id the core cannot name a receipt by is not taken.
        assert!(!runtime.send_pane_reply(reply("w1:p1", "not-a-uuid", "계속")));
        assert!(answer(&runtime, "not-a-uuid").is_none());
    }

    /// The pane is asked once more just before the words go: a terminal
    /// that changed, or a new session started in the same terminal, gets
    /// nothing. While one reply waits, its repeat converges on it and
    /// another request to the same pane is refused as in flight.
    #[test]
    fn a_reply_whose_pane_or_session_changes_while_it_waits_is_never_typed() {
        for (terminal, session, reason) in [
            ("t2", "s", "pane_changed"),
            ("t1", "next", "session_changed"),
        ] {
            let terminals = Arc::new(GrantingTerminals::default());
            terminals.hold.store(true, Ordering::SeqCst);
            let fake = crate::engine::fake::FakeEngine::start(|method, _| {
                assert_eq!(method, "submit_text");
                crate::engine::fake::reply(())
            });
            let shared = crate::runtime::tests::SharedRuntime::new(live(&fake, &terminals));
            {
                let mut runtime = shared.lock().unwrap();
                runtime.worker_context = Some(RuntimeWorkerContext {
                    runtime: shared.weak(),
                    notifier: crate::handle::ChangeNotifier::noop(),
                });
                runtime.send_pane_reply(reply("w1:p1", REPLY, "계속"));
                assert_eq!(
                    answer(&runtime, REPLY),
                    Some(("w1:p1".to_owned(), ThreadInputState::Sending, None))
                );
            }
            crate::runtime::tests::wait_for("the reply's permit ask", || {
                terminals.held.lock().unwrap().is_some()
            });
            {
                let mut runtime = shared.lock().unwrap();
                assert!(!runtime.send_pane_reply(reply("w1:p1", REPLY, "계속")));
                assert_eq!(
                    answer(&runtime, REPLY),
                    Some(("w1:p1".to_owned(), ThreadInputState::Sending, None))
                );
                runtime.send_pane_reply(reply("w1:p1", AGAIN, "또"));
                assert_eq!(answer(&runtime, AGAIN), refused("in_flight"));
                assert_eq!(
                    terminals.permits.lock().unwrap().len(),
                    1,
                    "nothing else asked"
                );
                observe(&mut runtime, terminal, session);
            }
            let grant = terminals.held.lock().unwrap().take().unwrap();
            grant(Ok(()));
            crate::runtime::tests::wait_for("the reply's answer", || {
                answer(&shared.lock().unwrap(), REPLY)
                    .is_some_and(|(_, state, _)| state != ThreadInputState::Sending)
            });
            assert!(fake.calls_of("submit_text").is_empty(), "{reason}");
            // The other window's refusal is still there beside the later answer.
            assert_eq!(answer(&shared.lock().unwrap(), AGAIN), refused("in_flight"));
            assert_eq!(answer(&shared.lock().unwrap(), REPLY), refused(reason));
            assert!(shared.lock().unwrap().reply_work.writing.is_empty());
        }
    }

    /// Every request keeps its own receipt, so an answer published in the
    /// same frame as a later one is still there. At most half the receipts
    /// may be sending: one past that is refused as busy before anything is
    /// asked, every sending receipt stays, and the newest settled ones fill
    /// the rest of the bound.
    #[test]
    fn sending_receipts_are_admitted_up_to_half_the_bound_and_never_dropped() {
        let terminals = Arc::new(GrantingTerminals::default());
        let fake = crate::engine::fake::FakeEngine::start(|method, _| {
            panic!("a busy core asks the engine nothing, asked {method}");
        });
        let mut runtime = live(&fake, &terminals);
        let id = |index: usize| format!("00000000-0000-4000-8000-{index:012}");
        for index in 0..PENDING {
            runtime.set_pane_reply(
                &format!("w{index}:p1"),
                &id(index),
                ThreadInputState::Sending,
                None,
            );
        }
        runtime.send_pane_reply(reply("w1:p1", REPLY, "계속"));
        assert_eq!(answer(&runtime, REPLY), refused("busy"));
        assert!(terminals.permits.lock().unwrap().is_empty());
        for index in PENDING..PENDING + RECEIPTS + 5 {
            runtime.set_pane_reply(
                "w2:p1",
                &id(index),
                ThreadInputState::Refused,
                Some("asleep"),
            );
        }
        let receipts = &runtime.snapshot.pane_replies.as_ref().unwrap().receipts;
        assert_eq!(receipts.len(), RECEIPTS);
        assert!((0..PENDING).all(|index| {
            answer(&runtime, &id(index))
                .is_some_and(|(_, state, _)| state == ThreadInputState::Sending)
        }));
        assert!(
            answer(&runtime, &id(PENDING + RECEIPTS + 4)).is_some(),
            "the newest settled receipt is kept"
        );
        assert!(
            answer(&runtime, REPLY).is_none(),
            "the oldest settled receipt went first"
        );
        // Settling keeps the receipt's place; an unchanged section is not published.
        assert!(runtime.set_pane_reply("w0:p1", &id(0), ThreadInputState::Sent, None));
        assert_eq!(
            runtime.snapshot.pane_replies.as_ref().unwrap().receipts[0].state,
            ThreadInputState::Sent
        );
        assert!(!runtime.set_pane_reply("w0:p1", &id(0), ThreadInputState::Sent, None));
    }
}
