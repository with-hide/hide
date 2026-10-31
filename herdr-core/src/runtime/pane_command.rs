//! What `hide pane`, `hide agent read` and `hide workspace purpose` take
//! from the runtime under its lock: the caller's checkout, the pane or agent
//! named, and the engine of the machine it is on (`crate::pane_command`).

use crate::pane_command::{PaneCommand, Prepared, Step};
use crate::workspace::authoritative_session_space;
use crate::workspace_control::{Caller, Context};

use super::Runtime;

/// A pane id as the core spells it: a device's as
/// `remote:<device>:pane:<id>`, this machine's bare.
fn split_scoped<'a>(node: &'a str, scoped: &'a str) -> (&'a str, &'a str) {
    scoped
        .strip_prefix("remote:")
        .and_then(|rest| rest.split_once(":pane:"))
        .unwrap_or((node, scoped))
}

impl Runtime {
    /// Admits `command` for the pane-bound `caller` on `device` whose
    /// checkout is `expected`, and resolves what it acts on. A bare pane id
    /// is a pane of the caller's own machine, as Herdr numbered it there.
    pub(crate) fn prepare_pane_command(
        &mut self,
        device: &str,
        caller: &str,
        expected: &Context,
        command: PaneCommand,
    ) -> Result<Prepared, String> {
        command.check()?;
        let Caller::Pane(caller_pane) = Caller::parse(caller) else {
            return Err("pane_required".into());
        };
        let context = self
            .resolve_caller(device, Caller::Pane(caller_pane))
            .map_err(|refusal| refusal.reason.to_owned())?;
        if context != *expected {
            return Err("caller_context_changed".into());
        }
        let node = self.node.as_str().to_owned();
        let node = node.as_str();
        // A pane named in full stays on the machine it names; a bare one is
        // the caller's.
        let target = |pane: &str| -> (String, String) {
            if pane.starts_with("remote:") {
                let (machine, raw) = split_scoped(node, pane);
                (machine.to_owned(), raw.to_owned())
            } else {
                (device.to_owned(), pane.to_owned())
            }
        };
        // A pane `hide pane send` types into: input, like a key the operator
        // typed there, for the draft guard and for typed input waiting on
        // someone to answer the agent's screen (`delivery::typed`).
        let mut sent = None;
        let (machine, step) = match command {
            PaneCommand::Read { pane, lines } => {
                let (machine, raw) = target(&pane);
                (machine, Step::Read { pane, raw, lines })
            }
            PaneCommand::Send { pane, text, enter } => {
                let (machine, raw) = target(&pane);
                sent = Some(if machine == node {
                    raw.clone()
                } else {
                    crate::session_sync::remote_pane_id(&machine, &raw)
                });
                (
                    machine,
                    Step::Send {
                        pane,
                        raw,
                        text,
                        enter,
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
                let (_, raw) = split_scoped(node, caller_pane);
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
        let engine = self.delivery_engine(&machine).ok_or("engine_unavailable")?;
        if let Some(pane) = sent {
            self.note_delivery_key(&pane, "pane_send");
        }
        Ok(Prepared { engine, step })
    }

    /// The live agent `key` names for a caller on `device`, as a letter's
    /// target is named: its registered name or id, the name Herdr gave it,
    /// or its pane.
    fn agent_named(
        &self,
        device: &str,
        key: &str,
    ) -> Result<&super::delivery::Observation, String> {
        let pane = if self.node == device || key.starts_with("remote:") {
            key.to_owned()
        } else {
            format!("remote:{device}:pane:{key}")
        };
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
    /// hosting another `writer`, and this machine's Herdr, which no
    /// admission calls.
    fn fixture(
        twin: bool,
    ) -> (
        Runtime,
        Context,
        tempfile::TempDir,
        crate::fake_herdr::FakeHerdr,
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
        runtime.observe_delivery(NODE, &payload, None, None);
        let herdr = crate::fake_herdr::FakeHerdr::start_with_errors("pane-command", |method, _| {
            panic!("admission calls no Herdr method, got {method}")
        });
        runtime.live = Some(crate::live::LiveContext {
            socket_path: herdr.socket_path().to_path_buf(),
            runtime: std::sync::Weak::new(),
            notifier: crate::handle::ChangeNotifier::noop(),
            engine: herdr.engine(),
            node: Arc::new(hide_node::Local::of_process()),
        });
        let context = runtime
            .workspace_control_query(NODE, "sender", Query::Info)
            .unwrap()
            .context;
        (runtime, context, root, herdr)
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
            Step::Read { pane, raw, .. } => Ok((pane, raw)),
            _ => panic!("a read"),
        }
    }

    fn agent(key: &str) -> PaneCommand {
        PaneCommand::AgentRead {
            agent: key.into(),
            lines: 10,
        }
    }

    /// PRD engine-bundle-1 D-5, B2: an agent is named as a letter's target
    /// is, by its name, its registration id or its pane, and the answer names
    /// its pane as `hide pane send` takes it.
    #[test]
    fn an_agent_is_found_by_its_name_registration_or_pane_and_answers_with_its_pane() {
        let (mut runtime, context, _root, _herdr) = fixture(false);
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
        let (runtime, context, _root, _herdr) = fixture(true);
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
        let (mut runtime, context, _root, _herdr) = fixture(false);
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
        let payload: SessionSnapshotPayload = serde_json::from_value(json!({"agents":[
            {"id":"far","pane_id":"w2:p1","agent":"codex","agent_status":"idle","state_change_seq":1,"lineage_session":"far-session"},
        ]}))
        .unwrap();
        runtime.observe_delivery("studio", &payload, None, None);
        assert_eq!(
            read(&runtime, &context, agent("far")).err(),
            Some("engine_unavailable".into())
        );
    }

    #[test]
    fn a_purpose_is_written_on_the_checkouts_engine_workspace_and_refused_without_one() {
        let (mut runtime, context, _root, _herdr) = fixture(false);
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
        let (runtime, context, _root, _herdr) = fixture(false);
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
