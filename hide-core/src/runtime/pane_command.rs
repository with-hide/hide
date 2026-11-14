//! What `hide pane`, `hide agent read` and `hide workspace purpose` take
//! from the runtime under its lock: the caller's checkout, the pane or agent
//! named, and the engine of the machine it is on (`crate::pane_command`).

use crate::pane_command::{PaneCommand, Prepared, Step};
use crate::workspace::authoritative_session_space;
use crate::workspace_control::{Caller, Context};

use super::Runtime;

impl Runtime {
    /// Admits `command` for the pane-bound `caller` on `device` whose
    /// checkout is `expected`, and resolves what it acts on. A bare pane id
    /// is a pane of the caller's own machine, as the engine numbered it there.
    pub(crate) fn prepare_pane_command(
        &self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: PaneCommand,
    ) -> Result<Prepared, String> {
        command.check()?;
        // An external tool reads panes and, at Act, types into them as the
        // operator does (PRD hide-external-control B20, B21, B28); it has
        // no pane to open another beside or a checkout to name.
        let (caller_pane, context) = match Caller::parse(caller) {
            Caller::Pane(caller_pane) => {
                let context = self
                    .resolve_caller(device, Caller::Pane(caller_pane))
                    .map_err(|refusal| refusal.reason.to_owned())?;
                if context != *expected {
                    return Err("caller_context_changed".into());
                }
                (caller_pane, context)
            }
            Caller::Access { connection, .. } => {
                let (level, _) = self
                    .access_actor(connection)
                    .filter(|_| device == self.node.as_str())
                    .ok_or(crate::access::Ended::Revoked.reason())?;
                match &command {
                    PaneCommand::Read { .. }
                    | PaneCommand::ReadScreen { .. }
                    | PaneCommand::AgentRead { .. } => {}
                    PaneCommand::Send { .. } if level == crate::access::Level::Act => {}
                    PaneCommand::Send { .. } => {
                        return Err(super::access::VIEW_ONLY.into());
                    }
                    PaneCommand::Open { .. } | PaneCommand::Purpose { .. } => {
                        return Err(super::access::NEEDS_AGENT.into());
                    }
                }
                ("", expected.clone())
            }
            _ => return Err("pane_required".into()),
        };
        let node = self.node.as_str();
        // A pane named in full stays on the machine it names; a bare one is
        // the caller's.
        let target = |pane: &str| -> (String, String) {
            let (machine, raw) = crate::address::split_pane(device, pane);
            (machine.to_owned(), raw.to_owned())
        };
        let engine_of = |machine: &str| self.delivery_engine(machine).ok_or("engine_unavailable");
        // The observed pane a send types into: input routed to it, like a key
        // the operator typed there, once the write completed (`Prepared::run`),
        // for the draft guard and for typed input waiting on someone to answer
        // the agent's screen (`delivery::typed`).
        let mut sent = None;
        let (machine, step) = match command {
            PaneCommand::ReadScreen { pane } => {
                let (machine, raw) = target(&pane);
                (machine, Step::ReadScreen { pane, raw })
            }
            PaneCommand::Read { pane, lines } => {
                let (machine, raw) = target(&pane);
                (machine, Step::Read { pane, raw, lines })
            }
            PaneCommand::Send {
                pane,
                text,
                keys,
                enter,
                paste,
            } => {
                let (machine, raw) = target(&pane);
                let observed = crate::address::pane_id_on(node, &machine, &raw);
                if paste && !self.takes_one_paste(&observed, &machine) {
                    return Err("paste_unsupported".into());
                }
                sent = Some(observed);
                (
                    machine,
                    Step::Send {
                        pane,
                        raw,
                        text,
                        keys,
                        enter,
                        paste,
                    },
                )
            }
            PaneCommand::AgentRead { agent, lines } => {
                let observation = self.agent_named(device, &agent)?;
                (
                    observation.actor.device_id.clone(),
                    // The answer names the agent's pane as `hide pane send`
                    // takes it from this caller: bare on the caller's own
                    // machine, in full on another.
                    Step::Read {
                        pane: if observation.actor.device_id == device {
                            observation.raw_pane_id.clone()
                        } else {
                            observation.actor.pane_id.clone()
                        },
                        raw: observation.raw_pane_id.clone(),
                        lines,
                    },
                )
            }
            PaneCommand::Open { command, focus } => {
                let (_, raw) = crate::address::split_pane(node, caller_pane);
                (
                    device.to_owned(),
                    Step::Open {
                        caller: raw.to_owned(),
                        cwd: context.checkout_path.clone(),
                        command,
                        focus,
                    },
                )
            }
            PaneCommand::Purpose { text } => (
                device.to_owned(),
                Step::Purpose {
                    workspace: self.purpose_workspace(device, &context.checkout_id)?,
                    text: text.trim().to_owned(),
                },
            ),
        };
        Ok(Prepared {
            engine: engine_of(&machine)?,
            step,
            gate: sent.as_ref().map(|_| self.input_gate()),
            agent_kind: sent
                .as_ref()
                .and_then(|pane| self.delivery_observations.get(pane))
                .map(|observation| observation.actor.kind.clone()),
            noted: sent,
            caller: caller_pane.to_owned(),
            runtime: std::sync::Weak::new(),
        })
    }

    /// Whether the agent in `pane` on `machine` is known to take a paste as
    /// one: an agent kind the composer journey measured taking one, on a
    /// system where it does (`AgentId::takes_one_paste`). A pane with no agent
    /// Hide observes, or a device whose system is unknown, is not known to.
    fn takes_one_paste(&self, pane: &str, machine: &str) -> bool {
        let Some(observation) = self.delivery_observations.get(pane) else {
            return false;
        };
        let windows = if machine == self.node.as_str() {
            Some(cfg!(windows))
        } else {
            self.snapshot
                .navigator
                .devices
                .iter()
                .find(|device| device.id == machine)
                .and_then(|device| device.host.platform.as_deref())
                .map(|platform| platform.starts_with("windows"))
        };
        hide_agent_adapter::adapter(&observation.actor.kind)
            .zip(windows)
            .is_some_and(|(row, windows)| row.key.takes_one_paste(windows))
    }

    /// The live agent `key` names for a caller on `device`, as a letter's
    /// target is named: its registered name or id, the name the engine gave it,
    /// or its pane.
    fn agent_named(
        &self,
        device: &str,
        key: &str,
    ) -> Result<&super::delivery::Observation, String> {
        let pane = crate::address::named_pane_id(self.node.as_str(), device, key);
        let registered = self
            .delivery_ledger
            .as_ref()
            .ok()
            .and_then(|ledger| crate::coordination::resolve_registered(ledger, key));
        let mut matches = self.delivery_observations.values().filter(|observation| {
            observation.actor.pane_id == pane
                || observation.given_name() == Some(key)
                || registered.is_some_and(|registered| registered.same_identity(&observation.actor))
        });
        let first = matches.next().ok_or("agent_unavailable")?;
        if matches.next().is_some() {
            return Err("target_ambiguous".into());
        }
        Ok(first)
    }

    /// The engine workspace that holds the purpose of the caller's checkout:
    /// a device checkout's owner workspace, this machine's authoritative
    /// session space, as the checkout row's Set purpose writes it.
    fn purpose_workspace(&self, device: &str, checkout_id: &str) -> Result<String, String> {
        let find = |workspaces: &[crate::model::WorkspaceSnapshot]| {
            workspaces.iter().find_map(|workspace| {
                workspace
                    .checkouts
                    .iter()
                    .find(|checkout| checkout.id == checkout_id)
                    .map(|checkout| (workspace.clone(), checkout.clone()))
            })
        };
        let found = if self.node == device {
            find(&self.snapshot.navigator.workspaces).and_then(|(workspace, checkout)| {
                authoritative_session_space(&self.last_session_spaces, &workspace, &checkout.path)
                    .map(|space| space.id.clone())
            })
        } else {
            self.snapshot
                .status
                .remote
                .iter()
                .filter(|remote| remote.target_id == device)
                .find_map(|remote| find(&remote.session.as_ref()?.workspaces))
                .and_then(|(_, checkout)| checkout.owner_workspace_id)
        };
        found.ok_or_else(|| "purpose_no_workspace".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::sidebar::SessionSnapshotPayload;
    use crate::workspace_control::Query;

    const NODE: &str = crate::node::TEST_NODE;

    /// The delivery fixture's checkout with its panes `sender` and
    /// `recipient` hosting `writer` and `reader`, with `twin` a pane `third`
    /// hosting another `writer`, and this machine's engine, which no
    /// admission calls.
    fn fixture(
        twin: bool,
    ) -> (
        Runtime,
        Context,
        tempfile::TempDir,
        Arc<crate::engine::fake::FakeEngine<'static>>,
    ) {
        fixture_answering(twin, |method, _| {
            panic!("admission calls no engine method, got {method}")
        })
    }

    /// The fixture, its engine answering with `respond`.
    fn fixture_answering(
        twin: bool,
        respond: impl FnMut(&'static str, &serde_json::Value) -> crate::engine::fake::Reply
        + Send
        + 'static,
    ) -> (
        Runtime,
        Context,
        tempfile::TempDir,
        Arc<crate::engine::fake::FakeEngine<'static>>,
    ) {
        let root = tempfile::tempdir().unwrap();
        let (runtime, _, _, _) = crate::runtime::delivery::tests::fixture(root.path());
        let mut runtime = Arc::try_unwrap(runtime)
            .ok()
            .expect("one owner")
            .into_inner()
            .unwrap();
        let mut agents = vec![
            json!({"id":"writer","pane_id":"sender","agent":"codex","agent_status":"working","state_change_seq":1,"lineage_session":"sender-session"}),
            json!({"id":"reader","pane_id":"recipient","agent":"codex","agent_status":"idle","state_change_seq":1,"lineage_session":"recipient-session"}),
        ];
        if twin {
            agents.push(json!({"id":"writer","pane_id":"third","agent":"codex","agent_status":"idle","state_change_seq":1,"lineage_session":"third-session"}));
        }
        let payload: SessionSnapshotPayload =
            serde_json::from_value(json!({ "agents": agents })).unwrap();
        runtime.observe_delivery_laid(NODE, &payload, None, None);
        let fake = crate::engine::fake::FakeEngine::start(respond);
        runtime.live = Some(fake.live());
        let context = runtime
            .workspace_control_query(NODE, "sender", Query::Info)
            .unwrap()
            .context;
        (runtime, context, root, fake)
    }

    fn read(
        runtime: &Runtime,
        context: &Context,
        command: PaneCommand,
    ) -> Result<(String, String), String> {
        match runtime
            .prepare_pane_command(NODE, "sender", context, command)?
            .step
        {
            Step::Read { pane, raw, .. } | Step::ReadScreen { pane, raw } => Ok((pane, raw)),
            _ => panic!("a read"),
        }
    }

    fn agent(key: &str) -> PaneCommand {
        PaneCommand::AgentRead {
            agent: key.into(),
            lines: 10,
        }
    }

    #[test]
    fn a_screen_read_preserves_styles_rows_and_truncation_without_sending_input() {
        let screen = "\u{1b}[31mred\u{1b}[0m\r\n  next row";
        let (runtime, context, _root, _fake) = fixture_answering(false, move |method, params| {
            assert_eq!(method, "read_screen");
            assert_eq!(params["pane"], "recipient");
            crate::engine::fake::reply(crate::engine::PaneText {
                text: screen.to_owned(),
                truncated: true,
            })
        });
        let result = runtime
            .prepare_pane_command(
                NODE,
                "sender",
                &context,
                PaneCommand::ReadScreen {
                    pane: "recipient".into(),
                },
            )
            .unwrap()
            .run()
            .unwrap();
        assert_eq!(
            result,
            json!({"pane": "recipient", "text": screen, "truncated": true})
        );
    }

    #[test]
    fn a_refused_screen_read_is_an_error_not_an_empty_screen() {
        let (runtime, context, _root, _fake) = fixture_answering(false, |method, _| {
            assert_eq!(method, "read_screen");
            crate::engine::fake::refused("pane_not_found", "gone")
        });
        assert_eq!(
            runtime
                .prepare_pane_command(
                    NODE,
                    "sender",
                    &context,
                    PaneCommand::ReadScreen {
                        pane: "recipient".into()
                    },
                )
                .unwrap()
                .run(),
            Err("pane_unavailable".into())
        );
    }

    /// A send counts as input routed to the agent's pane, the answer that
    /// lets typed input be typed again (`delivery::typed`), only once its
    /// text reached the pane: a refused write arms nothing.
    #[test]
    fn a_send_counts_as_input_only_once_its_text_reached_the_pane() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let refuse = Arc::new(AtomicBool::new(true));
        let refusing = refuse.clone();
        let (runtime, context, _root, _fake) =
            fixture_answering(false, move |method, _| match method {
                // No composer to read, as on a trust screen: the send goes on.
                "read_screen" => crate::engine::fake::refused("pane_not_found", "no screen"),
                "send_text" if refusing.load(Ordering::SeqCst) => {
                    crate::engine::fake::refused("pane_not_found", "gone")
                }
                "send_text" => crate::engine::fake::reply(()),
                other => panic!("a send calls only send_text, got {other}"),
            });
        let runtime = Arc::new(std::sync::Mutex::new(runtime));
        let send = || {
            let mut prepared = runtime
                .lock()
                .unwrap()
                .prepare_pane_command(
                    NODE,
                    "sender",
                    &context,
                    PaneCommand::Send {
                        pane: "recipient".into(),
                        text: Some("y".into()),
                        keys: Vec::new(),
                        enter: false,
                        paste: false,
                    },
                )
                .unwrap();
            prepared.runtime = Arc::downgrade(&runtime);
            prepared
        };
        // The pane's input generation, not its input time: the fixture's
        // observation and a fast send can read the same millisecond.
        let input = || runtime.lock().unwrap().input_fence("recipient").unwrap();
        let before = input();
        assert_eq!(send().run(), Err("pane_unavailable".into()));
        assert_eq!(input(), before, "a refused write is no input");
        refuse.store(false, Ordering::SeqCst);
        assert!(send().run().is_ok());
        assert_eq!(input(), before + 1, "a completed write is one input");
    }

    /// How the fixture's engine answers a send: the screen a composer read
    /// gets, and which input methods refuse. Every call is recorded.
    #[derive(Default)]
    struct SendEngine {
        screen: Option<&'static str>,
        refuse: Vec<&'static str>,
        /// Methods taken and never answered, a lost reply.
        lose: Vec<&'static str>,
        /// A method during which a person types into `pane` of the runtime,
        /// before its answer.
        typed_during: Option<(
            &'static str,
            &'static str,
            std::sync::Weak<std::sync::Mutex<Runtime>>,
        )>,
        calls: Vec<(String, serde_json::Value)>,
    }

    /// The fixture with an engine answering by `send_engine`, behind the runtime's
    /// mutex as the daemon holds it.
    fn send_fixture(
        send_engine: Arc<std::sync::Mutex<SendEngine>>,
    ) -> (
        Arc<std::sync::Mutex<Runtime>>,
        Context,
        tempfile::TempDir,
        Arc<crate::engine::fake::FakeEngine<'static>>,
    ) {
        let (runtime, context, root, fake) = fixture_answering(false, send_answers(send_engine));
        (
            Arc::new(std::sync::Mutex::new(runtime)),
            context,
            root,
            fake,
        )
    }

    /// How the fake engine answers by `send_engine`.
    fn send_answers(
        send_engine: Arc<std::sync::Mutex<SendEngine>>,
    ) -> impl FnMut(&'static str, &serde_json::Value) -> crate::engine::fake::Reply + Send + 'static
    {
        move |method, params| {
            let mut send_engine = send_engine.lock().unwrap();
            send_engine.calls.push((method.to_owned(), params.clone()));
            if let Some((during, pane, runtime)) = &send_engine.typed_during
                && *during == method
                && let Some(runtime) = runtime.upgrade()
            {
                runtime
                    .lock()
                    .unwrap()
                    .note_delivery_key_at(pane, crate::runtime::unix_milliseconds());
            }
            if send_engine.lose.contains(&method) {
                return crate::engine::fake::unanswered("the reply was lost");
            }
            if send_engine.refuse.contains(&method) {
                return crate::engine::fake::refused("pane_not_found", "refused");
            }
            match method {
                "read_screen" => match send_engine.screen {
                    Some(text) => crate::engine::fake::reply(crate::engine::PaneText {
                        text: text.to_owned(),
                        truncated: false,
                    }),
                    None => crate::engine::fake::refused("pane_not_found", "no screen"),
                },
                "send_text" | "press_pane_keys" | "paste_text" => crate::engine::fake::reply(()),
                other => panic!("a send reads the composer and writes input, got {other}"),
            }
        }
    }

    fn send(
        runtime: &Arc<std::sync::Mutex<Runtime>>,
        context: &Context,
        pane: &str,
        text: Option<&str>,
        keys: &[crate::engine::keys::PaneKey],
        enter: bool,
        paste: bool,
    ) -> Result<serde_json::Value, String> {
        let mut prepared = runtime.lock().unwrap().prepare_pane_command(
            NODE,
            "sender",
            context,
            PaneCommand::Send {
                pane: pane.into(),
                text: text.map(str::to_owned),
                keys: keys.to_vec(),
                enter,
                paste,
            },
        )?;
        prepared.runtime = Arc::downgrade(runtime);
        prepared.run()
    }

    fn input_at(runtime: &Arc<std::sync::Mutex<Runtime>>) -> u64 {
        runtime.lock().unwrap().delivery_observations["recipient"].last_input_at_unix_ms
    }

    fn written(
        send_engine: &Arc<std::sync::Mutex<SendEngine>>,
    ) -> Vec<(String, serde_json::Value)> {
        send_engine
            .lock()
            .unwrap()
            .calls
            .iter()
            .filter(|(method, _)| method != "read_screen")
            .cloned()
            .collect()
    }

    /// The text goes in first, then the keys in order with Enter last, in
    /// one press, and the answer says what the engine took.
    #[test]
    fn a_send_types_the_text_then_presses_the_keys_and_enter_and_says_what_landed() {
        use crate::engine::keys::PaneKey;
        let send_engine = Arc::new(std::sync::Mutex::new(SendEngine::default()));
        let (runtime, context, _root, _fake) = send_fixture(send_engine.clone());
        let answer = send(
            &runtime,
            &context,
            "recipient",
            Some("2"),
            &[PaneKey::Down, PaneKey::Esc],
            true,
            false,
        )
        .unwrap();
        assert_eq!(
            answer,
            json!({"pane":"recipient","text_sent":true,"keys_sent":["down","esc"],"enter_sent":true})
        );
        assert_eq!(
            written(&send_engine),
            vec![
                (
                    "send_text".to_owned(),
                    json!({"pane": "recipient","text":"2"})
                ),
                (
                    "press_pane_keys".to_owned(),
                    json!({"pane": "recipient","keys":["down","esc","enter"]})
                ),
            ]
        );
    }

    /// Marks a person's input in the fixture's `recipient` after its last
    /// submission, the core's rule for a draft that may stand there.
    fn person_typed(runtime: &Arc<std::sync::Mutex<Runtime>>) {
        let mut runtime = runtime.lock().unwrap();
        let observation = runtime.delivery_observations.get_mut("recipient").unwrap();
        observation.last_submit_at_unix_ms = 1;
        observation.entered_working_at_unix_ms = 1;
        observation.last_input_at_unix_ms = 2;
    }

    /// A person's draft refuses the send by the two rules a held letter
    /// waits on, before anything is written: input since the last
    /// submission, and text the composer reads now.
    #[test]
    fn a_draft_in_the_composer_refuses_a_send_before_any_write() {
        use crate::engine::keys::PaneKey;
        let send_engine = Arc::new(std::sync::Mutex::new(SendEngine {
            screen: Some(include_str!(
                "../../../hide-agent-adapter/tests/fixtures/composer/codex-1-typed.ansi"
            )),
            ..SendEngine::default()
        }));
        let (runtime, context, _root, _fake) = send_fixture(send_engine.clone());
        person_typed(&runtime);
        let before = input_at(&runtime);
        assert_eq!(
            send(
                &runtime,
                &context,
                "recipient",
                None,
                &[PaneKey::Esc],
                false,
                false
            ),
            Err("composer_not_empty".into())
        );
        assert!(written(&send_engine).is_empty());
        assert_eq!(input_at(&runtime), before);
        send_engine.lock().unwrap().screen = Some(include_str!(
            "../../../hide-agent-adapter/tests/fixtures/composer/codex-0-empty.ansi"
        ));
        assert!(
            send(
                &runtime,
                &context,
                "recipient",
                None,
                &[PaneKey::Esc],
                false,
                false
            )
            .is_ok()
        );
        send_engine.lock().unwrap().screen = None;
        assert!(
            send(
                &runtime,
                &context,
                "recipient",
                None,
                &[PaneKey::Enter],
                false,
                false
            )
            .is_ok()
        );
    }

    /// Codex 0.166's hooks review, the screen of the 2026-10-31 incident,
    /// draws its list cursor `›` where its composer prompt would be, so it
    /// reads as text; with no input since the last submission no person's
    /// draft can stand there, and the key that answers it goes through.
    #[test]
    fn a_screen_no_person_typed_into_takes_the_keys_that_answer_it() {
        use crate::engine::keys::PaneKey;
        let send_engine = Arc::new(std::sync::Mutex::new(SendEngine {
            screen: Some(include_str!(
                "../../../hide-agent-adapter/tests/fixtures/composer/codex-0.166-hooks-review.ansi"
            )),
            ..SendEngine::default()
        }));
        let (runtime, context, _root, _fake) = send_fixture(send_engine.clone());
        assert!(!runtime.lock().unwrap().person_draft("recipient"));
        assert_eq!(
            send(
                &runtime,
                &context,
                "recipient",
                None,
                &[PaneKey::Esc],
                false,
                false
            )
            .unwrap()["keys_sent"],
            json!(["esc"])
        );
        assert_eq!(
            written(&send_engine),
            vec![(
                "press_pane_keys".to_owned(),
                json!({"pane": "recipient","keys":["esc"]})
            )]
        );
    }

    /// Text a send left in the composer is its sender's own: the sender's
    /// next keys, such as the Enter an agent's command popup did not take,
    /// go through, and a key a person types after it holds the next send.
    #[test]
    fn a_senders_own_text_is_no_draft_until_a_person_types_after_it() {
        use crate::engine::keys::PaneKey;
        let send_engine = Arc::new(std::sync::Mutex::new(SendEngine {
            screen: Some(include_str!(
                "../../../hide-agent-adapter/tests/fixtures/composer/codex-1-typed.ansi"
            )),
            ..SendEngine::default()
        }));
        let (runtime, context, _root, _fake) = send_fixture(send_engine.clone());
        assert!(
            send(
                &runtime,
                &context,
                "recipient",
                Some("/hooks"),
                &[],
                false,
                false
            )
            .is_ok()
        );
        assert!(!runtime.lock().unwrap().person_draft("recipient"));
        assert!(
            send(
                &runtime,
                &context,
                "recipient",
                None,
                &[PaneKey::Enter],
                false,
                false
            )
            .is_ok()
        );
        let typed = input_at(&runtime) + 1;
        runtime
            .lock()
            .unwrap()
            .note_delivery_key_at("recipient", typed);
        assert_eq!(
            send(
                &runtime,
                &context,
                "recipient",
                None,
                &[PaneKey::Enter],
                false,
                false
            ),
            Err("composer_not_empty".into())
        );
    }

    /// A send that fails before anything lands notes no input.
    #[test]
    fn a_refused_send_is_no_input() {
        use crate::engine::keys::PaneKey;
        let send_engine = Arc::new(std::sync::Mutex::new(SendEngine {
            refuse: vec!["press_pane_keys"],
            ..SendEngine::default()
        }));
        let (runtime, context, _root, _fake) = send_fixture(send_engine.clone());
        let before = input_at(&runtime);
        assert_eq!(
            send(
                &runtime,
                &context,
                "recipient",
                None,
                &[PaneKey::Esc],
                false,
                false
            ),
            Err("pane_unavailable".into())
        );
        assert_eq!(input_at(&runtime), before, "a refused key is no input");
    }

    /// The parent of a child whose brief is unconfirmed, with a fake engine
    /// answering by `send_engine`: the brief was typed at `BRIEF_AT`, the child's
    /// state moved right after, and it was judged unconfirmed with nobody
    /// having answered the child's screen, so only an answer lets it be
    /// typed again.
    fn unconfirmed_brief(
        send_engine: Arc<std::sync::Mutex<SendEngine>>,
    ) -> (
        Arc<std::sync::Mutex<Runtime>>,
        Context,
        tempfile::TempDir,
        Arc<crate::engine::fake::FakeEngine<'static>>,
    ) {
        use crate::delivery::typed::{RECORD_WAIT_MS, REREAD_WAIT_MS};
        const BRIEF_AT: u64 = 100_000;
        let (root, mut runtime, _) = crate::runtime::typed_input::tests::spawned(false, BRIEF_AT);
        crate::runtime::typed_input::tests::child_reads(&mut runtime, "idle", None, BRIEF_AT + 1);
        let fake = crate::engine::fake::FakeEngine::start(send_answers(send_engine));
        runtime.live = Some(fake.live());
        let asked = BRIEF_AT + 1 + RECORD_WAIT_MS;
        runtime.typed_input_work(asked);
        runtime.take_session_rereads();
        let work = runtime.typed_input_work(asked + REREAD_WAIT_MS);
        assert_eq!(
            work[0].resubmit.as_ref().map(|resubmit| resubmit.retype),
            Some(false)
        );
        let context = runtime
            .workspace_control_query(NODE, "sender", Query::Info)
            .unwrap()
            .context;
        (
            Arc::new(std::sync::Mutex::new(runtime)),
            context,
            root,
            fake,
        )
    }

    /// Whether the child's screen counts as answered once input is quiet,
    /// the condition the brief is typed again on (`typed::retype_due`).
    fn screen_answered(runtime: &Arc<std::sync::Mutex<Runtime>>) -> Option<bool> {
        let quiet = crate::runtime::unix_milliseconds() + crate::delivery::INPUT_QUIET_MS;
        runtime.lock().unwrap().typed_input_work(quiet)[0]
            .resubmit
            .as_ref()
            .map(|resubmit| resubmit.retype)
    }

    /// The landed part of a partial send, as its refusal's `sent` detail.
    fn sent_of(refusal: &str) -> (&str, serde_json::Value) {
        let (code, detail) = crate::coordination::split_refusal(refusal);
        (
            code,
            serde_json::from_str(detail.expect("what landed")).unwrap(),
        )
    }

    /// A send that stopped partway, its keys refused or their reply lost,
    /// landed its text and answered nothing: the parent's unconfirmed brief
    /// is not typed again for it, and the refusal says what landed. The
    /// same send landing whole is the answer that lets the brief be typed
    /// again.
    #[test]
    fn only_a_send_that_landed_whole_lets_an_unconfirmed_brief_be_typed_again() {
        use crate::engine::keys::PaneKey;
        for (send_engine, code) in [
            (
                SendEngine {
                    refuse: vec!["press_pane_keys"],
                    ..SendEngine::default()
                },
                "keys_not_sent",
            ),
            (
                SendEngine {
                    lose: vec!["press_pane_keys"],
                    ..SendEngine::default()
                },
                "send_unconfirmed",
            ),
        ] {
            let send_engine = Arc::new(std::sync::Mutex::new(send_engine));
            let (runtime, context, _root, _fake) = unconfirmed_brief(send_engine.clone());
            let refused = send(
                &runtime,
                &context,
                "child",
                Some("t"),
                &[PaneKey::Esc],
                false,
                false,
            )
            .unwrap_err();
            assert_eq!(
                sent_of(&refused),
                (
                    code,
                    json!({"text_sent": true, "keys_sent": [], "enter_sent": false})
                )
            );
            assert_eq!(
                screen_answered(&runtime),
                Some(false),
                "{code}: nothing answered"
            );
            send_engine.lock().unwrap().refuse.clear();
            send_engine.lock().unwrap().lose.clear();
            send(
                &runtime,
                &context,
                "child",
                None,
                &[PaneKey::Esc],
                false,
                false,
            )
            .unwrap();
            assert_eq!(
                screen_answered(&runtime),
                Some(true),
                "{code}: a whole send answers"
            );
        }
    }

    /// A person who types while the send's text is on its way keeps their
    /// input: the send stops before its keys with `operator_typing` and
    /// says the text landed, and the person's input stays a draft the next
    /// send is held by.
    #[test]
    fn a_person_typing_during_a_send_stops_it_before_its_next_write_and_keeps_their_draft() {
        use crate::engine::keys::PaneKey;
        let send_engine = Arc::new(std::sync::Mutex::new(SendEngine {
            screen: Some(include_str!(
                "../../../hide-agent-adapter/tests/fixtures/composer/codex-1-typed.ansi"
            )),
            ..SendEngine::default()
        }));
        let (runtime, context, _root, _fake) = send_fixture(send_engine.clone());
        {
            let mut runtime = runtime.lock().unwrap();
            let observation = runtime.delivery_observations.get_mut("recipient").unwrap();
            observation.last_submit_at_unix_ms = 1;
            observation.entered_working_at_unix_ms = 1;
            observation.last_input_at_unix_ms = 1;
        }
        send_engine.lock().unwrap().typed_during =
            Some(("send_text", "recipient", Arc::downgrade(&runtime)));
        let refused = send(
            &runtime,
            &context,
            "recipient",
            Some("y"),
            &[PaneKey::Down],
            true,
            false,
        )
        .unwrap_err();
        assert_eq!(
            sent_of(&refused),
            (
                "operator_typing",
                json!({"text_sent": true, "keys_sent": [], "enter_sent": false})
            )
        );
        assert_eq!(
            written(&send_engine),
            vec![(
                "send_text".to_owned(),
                json!({"pane": "recipient", "text": "y"})
            )],
            "nothing is written after the person's input"
        );
        assert!(runtime.lock().unwrap().person_draft("recipient"));
        send_engine.lock().unwrap().typed_during = None;
        assert_eq!(
            send(
                &runtime,
                &context,
                "recipient",
                None,
                &[PaneKey::Enter],
                false,
                false
            ),
            Err("composer_not_empty".into())
        );
    }

    /// A paste goes in as one input with each line break a CR, only where
    /// the pane's agent is known to take one; a pane with no agent is
    /// refused before anything is asked.
    #[test]
    fn a_paste_goes_in_whole_only_where_its_agent_is_known_to_take_one() {
        let send_engine = Arc::new(std::sync::Mutex::new(SendEngine::default()));
        let (runtime, context, _root, _fake) = send_fixture(send_engine.clone());
        send(
            &runtime,
            &context,
            "recipient",
            Some("one\ntwo\r\nthree"),
            &[],
            false,
            true,
        )
        .unwrap();
        assert_eq!(
            written(&send_engine),
            vec![(
                "paste_text".to_owned(),
                json!({"pane": "recipient","text":"one\rtwo\rthree"})
            )]
        );
        send_engine.lock().unwrap().calls.clear();
        assert_eq!(
            send(
                &runtime,
                &context,
                "w7:p3",
                Some("one\ntwo"),
                &[],
                false,
                true
            ),
            Err("paste_unsupported".into())
        );
        assert!(send_engine.lock().unwrap().calls.is_empty());
    }

    /// The pane of an agent being put to sleep takes no send.
    #[test]
    fn a_pane_closed_to_input_takes_no_send() {
        use crate::engine::keys::PaneKey;
        let send_engine = Arc::new(std::sync::Mutex::new(SendEngine::default()));
        let (runtime, context, _root, _fake) = send_fixture(send_engine.clone());
        runtime.lock().unwrap().input_gate().close("recipient");
        assert_eq!(
            send(
                &runtime,
                &context,
                "recipient",
                None,
                &[PaneKey::Esc],
                false,
                false
            ),
            Err("pane_asleep".into())
        );
        assert!(send_engine.lock().unwrap().calls.is_empty());
    }

    /// PRD engine-bundle-1 D-5, B2: an agent is named as a letter's target
    /// is, by its name, its registration id or its pane, and the answer names
    /// its pane as `hide pane send` takes it.
    #[test]
    fn an_agent_is_found_by_its_name_registration_or_pane_and_answers_with_its_pane() {
        let (mut runtime, context, _root, _fake) = fixture(false);
        assert_eq!(
            read(&runtime, &context, agent("reader")),
            Ok(("recipient".into(), "recipient".into()))
        );
        assert_eq!(
            read(&runtime, &context, agent("recipient")),
            Ok(("recipient".into(), "recipient".into()))
        );
        let actor = runtime.delivery_observations["recipient"].actor.clone();
        let mut ledger = (*runtime.delivery_state().unwrap()).clone();
        ledger.agents.push(crate::coordination::AgentRecord {
            id: "agent-7".into(),
            name: Some("reviewer".into()),
            machine: NODE.into(),
            host_scope: "fixture-scope".into(),
            native_machine: "fixture-machine".into(),
            session: "recipient-session".into(),
            instance: "terminal-recipient".into(),
            pane: "recipient".into(),
            parent: None,
            origin: None,
            project: None,
            actor,
            ended: false,
            terminal_id: None,
            forked: false,
        });
        runtime.delivery_ledger = Ok(Arc::new(ledger));
        for key in ["agent-7", "reviewer"] {
            assert_eq!(
                read(&runtime, &context, agent(key)),
                Ok(("recipient".into(), "recipient".into())),
                "{key}"
            );
        }
        assert_eq!(
            read(&runtime, &context, agent("nobody")),
            Err("agent_unavailable".into())
        );
    }

    #[test]
    fn two_agents_of_one_name_are_ambiguous_rather_than_either() {
        let (runtime, context, _root, _fake) = fixture(true);
        assert_eq!(
            read(&runtime, &context, agent("writer")),
            Err("target_ambiguous".into())
        );
        // Each is still reached by its pane.
        assert_eq!(
            read(&runtime, &context, agent("third")).map(|(pane, _)| pane),
            Ok("third".into())
        );
    }

    /// A bare pane id is a pane of the caller's machine; one named in full
    /// is the named device's, whose engine it then needs, and an agent there
    /// is read through that device's engine too.
    #[test]
    fn a_pane_named_in_full_is_read_on_its_device_and_a_bare_one_on_the_callers() {
        let (mut runtime, context, _root, _fake) = fixture(false);
        let pane = |id: &str| PaneCommand::Read {
            pane: id.into(),
            lines: 10,
        };
        assert_eq!(
            read(&runtime, &context, pane("w7:p3")),
            Ok(("w7:p3".into(), "w7:p3".into()))
        );
        assert_eq!(
            read(&runtime, &context, pane("remote:studio:pane:w7:p3")).err(),
            Some("engine_unavailable".into()),
            "the device's engine, not this machine's"
        );
        assert_eq!(
            read(
                &runtime,
                &context,
                PaneCommand::ReadScreen {
                    pane: "w7:p3".into()
                }
            ),
            Ok(("w7:p3".into(), "w7:p3".into()))
        );
        assert_eq!(
            read(
                &runtime,
                &context,
                PaneCommand::ReadScreen {
                    pane: "remote:studio:pane:w7:p3".into()
                }
            )
            .err(),
            Some("engine_unavailable".into())
        );
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"far","pane_id":"w2:p1","agent":"codex","agent_status":"idle","state_change_seq":1,"lineage_session":"far-session"},
        ]}))
        .unwrap();
        runtime.observe_delivery_laid("studio", &payload, None, None);
        assert_eq!(
            read(&runtime, &context, agent("far")).err(),
            Some("engine_unavailable".into())
        );
    }

    #[test]
    fn a_purpose_is_written_on_the_checkouts_engine_workspace_and_refused_without_one() {
        let (mut runtime, context, _root, _fake) = fixture(false);
        let purpose = || PaneCommand::Purpose {
            text: "  fix the bell  ".into(),
        };
        assert_eq!(
            runtime
                .prepare_pane_command(NODE, "sender", &context, purpose())
                .err(),
            Some("purpose_no_workspace".into())
        );
        runtime.snapshot.navigator.workspaces[0].session_workspace_ids = vec!["w4".into()];
        runtime.last_session_spaces = vec![crate::workspace::SessionSpace {
            id: "w4".into(),
            label: "fixture".into(),
            cwds: vec!["/checkouts/fixture/src".into()],
            purpose: None,
            checkout: None,
            owner_mark: None,
        }];
        match runtime
            .prepare_pane_command(NODE, "sender", &context, purpose())
            .unwrap()
            .step
        {
            Step::Purpose { workspace, text } => {
                assert_eq!((workspace.as_str(), text.as_str()), ("w4", "fix the bell"));
            }
            _ => panic!("a purpose"),
        }
    }

    #[test]
    fn a_caller_whose_checkout_moved_or_that_is_no_pane_is_refused() {
        let (runtime, context, _root, _fake) = fixture(false);
        let moved = Context {
            checkout_id: "another".into(),
            ..context.clone()
        };
        let command = || PaneCommand::Read {
            pane: "recipient".into(),
            lines: 10,
        };
        assert_eq!(
            runtime
                .prepare_pane_command(NODE, "sender", &moved, command())
                .err(),
            Some("caller_context_changed".into())
        );
        assert_eq!(
            runtime
                .prepare_pane_command(
                    NODE,
                    "checkout:key1:/checkouts/fixture",
                    &context,
                    command()
                )
                .err(),
            Some("pane_required".into())
        );
        // A pane the caller opens beside is its own, in its checkout.
        match runtime
            .prepare_pane_command(
                NODE,
                "sender",
                &context,
                PaneCommand::Open {
                    command: "npm test".into(),
                    focus: false,
                },
            )
            .unwrap()
            .step
        {
            Step::Open { caller, cwd, .. } => {
                assert_eq!(
                    (caller.as_str(), cwd.as_str()),
                    ("sender", "/checkouts/fixture")
                );
            }
            _ => panic!("an open"),
        }
    }
}
