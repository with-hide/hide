//! Memory-only admission of passive metadata. Proof and all I/O run on the
//! bootstrap worker; only proven facts enter the existing links mailbox.

use std::sync::Arc;
use std::time::Instant;

use hide_agent_hooks::observation::{DirectObservation, ObservationStatus, ServerReportedTarget};

use super::Runtime;
use crate::direct_observation::{ADMISSION_TTL_MS, Endpoint, PreparedObservation, hash};
use crate::links::observation::DirectFact;
use crate::workspace_control::{Caller, Context};

impl Runtime {
    #[allow(clippy::too_many_arguments)] // identity facts and deadline cross the authenticated boundary intact
    pub(crate) fn prepare_direct_observation(
        &self,
        device: &str,
        caller: &str,
        expected: &Context,
        terminal: &str,
        observation: DirectObservation,
        start: bool,
        deadline: Instant,
    ) -> Result<PreparedObservation, String> {
        if !crate::direct_observation::valid(&observation) || Instant::now() >= deadline {
            return Err("observation_invalid".into());
        }
        let now = super::unix_milliseconds();
        // The timestamp is only a freshness claim. Never compensate for a
        // future client clock by treating it as an authoritative past event.
        if observation.observed_at_unix_ms > now
            || now.saturating_sub(observation.observed_at_unix_ms) > ADMISSION_TTL_MS
        {
            return Err("observation_stale".into());
        }
        let Caller::Pane(pane) = Caller::parse(caller) else {
            return Err("observation_agent_pane_required".into());
        };
        let context = self
            .resolve_caller(device, Caller::Pane(pane))
            .map_err(|_| "observation_context_changed")?;
        if context != *expected {
            return Err("observation_context_changed".into());
        }
        let sender = self
            .direct_endpoint(device, pane, observation.observed_at_unix_ms)
            .ok_or("observation_sender_unproven")?;
        let runtime = hide_agent_adapter::adapter(&observation.runtime_id)
            .ok_or("observation_runtime_invalid")?;
        if sender.terminal != terminal
            || hide_agent_adapter::canonical_kind(&sender.actor.kind) != runtime.herdr.name
        {
            return Err("observation_sender_changed".into());
        }
        // Hash addresses before durable metadata leaves the proof boundary.
        let scope = self
            .delivery_observations
            .get(pane)
            .and_then(|value| value.host_scope.clone());
        let at = observation.observed_at_unix_ms;
        let target = observation
            .target
            .as_ref()
            .filter(|_| observation.status != ObservationStatus::RejectedObserved);
        let (recipient, seen_before) = match target {
            // A start is proven against Herdr's own list after this lock: the
            // agent it made may not be projected yet. A pane Hide already saw
            // before the call names whatever ran there, not what it started.
            Some(target) if start => (
                None,
                (target.terminal_id.is_none())
                    .then(|| self.direct_target(device, scope.as_deref(), target, false))
                    .flatten()
                    .filter(|value| value.observed_since_unix_ms <= at)
                    .and_then(|value| value.actor.session.clone()),
            ),
            Some(target) => (
                self.direct_target(device, scope.as_deref(), target, true)
                    .and_then(|value| self.direct_endpoint(device, &value.actor.pane_id, at))
                    .filter(|endpoint| match &target.native_session {
                        Some(native) => {
                            target.terminal_id.as_deref() == Some(endpoint.terminal.as_str())
                                && crate::wire::session_digest(&native.value)
                                    == endpoint.actor.session
                                && hide_agent_adapter::canonical_kind(&native.agent)
                                    == hide_agent_adapter::canonical_kind(&endpoint.actor.kind)
                        }
                        None => target
                            .terminal_id
                            .as_deref()
                            .is_none_or(|terminal| terminal == endpoint.terminal),
                    }),
                None,
            ),
            None => (None, None),
        };
        let connector = self
            .delivery_connector(device)
            .ok_or("observation_server_unavailable")?;
        let (_, node) = self
            .linked_node(device)
            .map_err(|_| "observation_node_unavailable")?;
        // Device registrations already distinguish canonical server/session
        // routes. The local socket additionally distinguishes named servers.
        let source_server = self.direct_server_key(device, scope.as_deref());
        Ok(PreparedObservation {
            observation,
            sender,
            recipient,
            start,
            seen_before,
            connector,
            node,
            source_server,
            runtime: std::sync::Weak::new(),
            deadline,
            context: expected.clone(),
        })
    }

    /// The one agent `target` names on the sender's own server (PRD
    /// direct-prompt-observation-widen D-11, D-18): a pane Herdr reported, by
    /// its exact pane id, or a target as a command wrote it, which Herdr
    /// itself resolves as a pane id or an agent name. Two answers are none.
    fn direct_target(
        &self,
        device: &str,
        scope: Option<&str>,
        target: &ServerReportedTarget,
        by_name: bool,
    ) -> Option<&super::delivery::Observation> {
        let literal = by_name && target.native_session.is_none();
        let mut found = self.delivery_observations.values().filter(|value| {
            value.actor.device_id == device
                && value.host_scope.as_deref() == scope
                && (value.raw_pane_id == target.pane_id
                    || (literal && value.actor.name == target.pane_id))
        });
        let first = found.next()?;
        found
            .all(|other| other.actor.pane_id == first.actor.pane_id)
            .then_some(first)
    }

    fn direct_server_key(&self, device: &str, scope: Option<&str>) -> String {
        let route = self
            .snapshot
            .ui_state
            .device_registrations
            .iter()
            .find(|registration| registration.id == device)
            .map(|registration| {
                (
                    registration.origin.ssh_alias(),
                    registration.herdr_session.as_deref().unwrap_or("default"),
                    registration.herdr_socket_path.as_deref(),
                )
            });
        hash(&serde_json::json!([device, scope, route]).to_string())
    }

    fn direct_endpoint(&self, device: &str, pane: &str, at: u64) -> Option<Endpoint> {
        if !self.delivery_connected.contains(device) {
            return None;
        }
        let value = self.delivery_observations.get(pane)?;
        value.actor.require_native_identity().ok()?;
        // This is a local receipt fence over the authoritative lifecycle
        // projection, not an invented session-start or command-start time.
        if value.actor.device_id != device || value.observed_since_unix_ms > at {
            return None;
        }
        Some(Endpoint {
            actor: value.actor.clone(),
            raw_pane: value.raw_pane_id.clone(),
            terminal: value.terminal_id.clone()?,
            observed_since: value.observed_since_unix_ms,
            reference: value.session.clone(),
        })
    }

    pub(crate) fn commit_direct_observation(
        &self,
        prepared: &PreparedObservation,
        fact: DirectFact,
    ) -> Result<(), String> {
        let current = |endpoint: &Endpoint| {
            self.direct_endpoint(
                &endpoint.actor.device_id,
                &endpoint.actor.pane_id,
                prepared.observation.observed_at_unix_ms,
            )
            .is_some_and(|now| {
                now.actor.same_identity(&endpoint.actor)
                    && now.terminal == endpoint.terminal
                    && now.observed_since == endpoint.observed_since
            })
        };
        let device = &prepared.sender.actor.device_id;
        if Instant::now() >= prepared.deadline {
            return Err("observation_expired".into());
        }
        let scope = self
            .delivery_observations
            .get(&prepared.sender.actor.pane_id)
            .and_then(|value| value.host_scope.as_deref());
        if self.direct_server_key(device, scope) != prepared.source_server
            || !self
                .resolve_caller(device, Caller::Pane(&prepared.sender.actor.pane_id))
                .is_ok_and(|context| context == prepared.context)
            || !current(&prepared.sender)
            || (fact.recipient.is_some()
                && prepared
                    .recipient
                    .as_ref()
                    .is_none_or(|value| !current(value)))
            || !self
                .delivery_connector(device)
                .is_some_and(|now| Arc::ptr_eq(&now, &prepared.connector))
            || !self
                .linked_node(device)
                .is_ok_and(|(_, now)| Arc::ptr_eq(&now, &prepared.node))
        {
            return Err("observation_native_changed".into());
        }
        self.direct_observation_client()
            .ok_or("observation_unavailable")?
            .observe(fact)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::direct_observation::PreparedObservation;
    use crate::direct_observation::valid;
    use crate::links::observation::DirectOutcome;
    use crate::model::GraphParty;
    use hide_agent_hooks::observation::{
        ObservationCorrelation, ObservationNativeSession, ObservationReason,
    };
    use serde_json::json;
    use std::sync::Mutex;
    use std::time::Duration;

    fn party(actor: &crate::delivery::Actor) -> Result<GraphParty, String> {
        Ok(GraphParty {
            device_id: actor.device_id.clone(),
            pane_id: actor.pane_id.clone(),
            session: actor.session.clone().ok_or("missing native identity")?,
        })
    }

    fn observation() -> DirectObservation {
        DirectObservation {
            runtime_id: "codex".into(),
            session_id: "sender-native".into(),
            call_id: "call-native".into(),
            correlation_kind: ObservationCorrelation::ToolCall,
            observed_at_unix_ms: crate::runtime::unix_milliseconds(),
            status: ObservationStatus::SubmittedObserved,
            reason: ObservationReason::Complete,
            target: Some(ServerReportedTarget {
                pane_id: "recipient".into(),
                terminal_id: Some("terminal-recipient".into()),
                native_session: Some(ObservationNativeSession {
                    source: "herdr:codex".into(),
                    agent: "codex".into(),
                    kind: "id".into(),
                    value: "recipient-native".into(),
                }),
            }),
        }
    }

    fn lifecycle(recipient_terminal: &str) -> crate::sidebar::SessionSnapshotPayload {
        lifecycle_for_kind(recipient_terminal, "codex")
    }

    fn lifecycle_for_kind(
        recipient_terminal: &str,
        kind: &str,
    ) -> crate::sidebar::SessionSnapshotPayload {
        serde_json::from_value(json!({"agents":[
            {"id":"sender","pane_id":"sender","agent":kind,"agent_status":"working","state_change_seq":1,
             "lineage_session":crate::wire::session_digest("sender-native"),
             "agent_session":{"source":format!("herdr:{kind}"),"agent":kind,"kind":"id","value":"sender-native"}},
            {"id":"recipient-name","pane_id":"recipient","agent":kind,"agent_status":"working","state_change_seq":1,
             "lineage_session":crate::wire::session_digest("recipient-native"),
             "agent_session":{"source":format!("herdr:{kind}"),"agent":kind,"kind":"id","value":"recipient-native"}}
        ],"panes":[{"pane_id":"sender","terminal_id":"terminal-sender"},
            {"pane_id":"recipient","terminal_id":recipient_terminal}]})).unwrap()
    }

    fn server(recipient_terminal: &'static str) -> crate::fake_herdr::FakeHerdr {
        server_for_kind(recipient_terminal, "codex")
    }

    fn server_for_kind(
        recipient_terminal: &'static str,
        kind: &'static str,
    ) -> crate::fake_herdr::FakeHerdr {
        crate::fake_herdr::FakeHerdr::start("direct-observation", move |method, params| {
            assert_eq!(method, "agent.get");
            let pane = params["target"].as_str().unwrap();
            let (native, terminal) = match pane {
                "sender" => ("sender-native", "terminal-sender"),
                "recipient" => ("recipient-native", recipient_terminal),
                other => panic!("unexpected target {other}"),
            };
            json!({"type":"agent_info","agent":{"pane_id":pane,"tab_id":"tab","workspace_id":"workspace",
                "terminal_id":terminal,"agent":kind,"agent_status":"working","state_change_seq":1,
                "focused":false,"revision":1,
                "agent_session":{"source":format!("herdr:{kind}"),"agent":kind,"kind":"id","value":native}}})
        })
    }

    fn prepare(
        runtime: &Arc<Mutex<Runtime>>,
        event: DirectObservation,
    ) -> Result<PreparedObservation, String> {
        let guard = runtime.lock().unwrap();
        let context = guard
            .resolve_caller(
                crate::node::TEST_NODE,
                crate::workspace_control::Caller::Pane("sender"),
            )
            .unwrap();
        let mut prepared = guard.prepare_direct_observation(
            crate::node::TEST_NODE,
            "sender",
            &context,
            "terminal-sender",
            event,
            false,
            Instant::now() + Duration::from_secs(1),
        )?;
        prepared.runtime = Arc::downgrade(runtime);
        Ok(prepared)
    }

    fn setup(root: &std::path::Path, herdr: &crate::fake_herdr::FakeHerdr) -> Arc<Mutex<Runtime>> {
        let (runtime, _, _, _) = crate::runtime::delivery::tests::fixture(root);
        {
            let mut guard = runtime.lock().unwrap();
            guard.observe_delivery(
                crate::node::TEST_NODE,
                &lifecycle("terminal-recipient"),
                Some("fixture-server"),
                None,
            );
            guard.live = Some(crate::live::LiveContext {
                socket_path: herdr.socket_path().to_owned(),
                runtime: Arc::downgrade(&runtime),
                notifier: crate::handle::ChangeNotifier::noop(),
                api_connector: Arc::new(herdr.connector()),
                node: guard.own_node(),
            });
        }
        runtime
    }

    #[test]
    fn complete_capture_matches_exact_current_native_endpoints() {
        let root = tempfile::tempdir().unwrap();
        let herdr = server("terminal-recipient");
        let runtime = setup(root.path(), &herdr);
        let prepared = prepare(&runtime, observation()).unwrap();
        prepared.prove_sender().unwrap();
        prepared
            .prove_recipient(
                prepared.recipient.as_ref().unwrap(),
                prepared.observation.target.as_ref().unwrap(),
            )
            .unwrap();
    }

    #[test]
    fn child_native_id_cannot_spend_its_root_pane_identity() {
        let root = tempfile::tempdir().unwrap();
        let herdr = server("terminal-recipient");
        let runtime = setup(root.path(), &herdr);
        let mut event = observation();
        event.session_id = "different-child-native".into();
        assert_eq!(
            prepare(&runtime, event)
                .unwrap()
                .prove_sender()
                .unwrap_err(),
            "observation_sender_session_changed"
        );
    }

    #[test]
    fn server_native_id_proves_opencode_without_a_transcript_file() {
        let root = tempfile::tempdir().unwrap();
        let herdr = server_for_kind("terminal-recipient", "opencode");
        let runtime = setup(root.path(), &herdr);
        {
            let mut guard = runtime.lock().unwrap();
            guard.observe_delivery(
                crate::node::TEST_NODE,
                &lifecycle_for_kind("terminal-recipient", "opencode"),
                Some("fixture-server"),
                None,
            );
            assert!(guard.delivery_observations["sender"].session.is_none());
        }
        let mut event = observation();
        event.runtime_id = "opencode".into();
        prepare(&runtime, event.clone())
            .unwrap()
            .prove_sender()
            .unwrap();
        event.session_id = "different-child-native".into();
        assert_eq!(
            prepare(&runtime, event)
                .unwrap()
                .prove_sender()
                .unwrap_err(),
            "observation_sender_session_changed"
        );
    }

    #[test]
    fn forged_future_stale_and_before_lifecycle_timestamps_are_not_admitted() {
        let root = tempfile::tempdir().unwrap();
        let herdr = server("terminal-recipient");
        let runtime = setup(root.path(), &herdr);
        for at in [
            u64::MAX,
            crate::runtime::unix_milliseconds() - ADMISSION_TTL_MS - 1,
        ] {
            let mut event = observation();
            event.observed_at_unix_ms = at;
            assert_eq!(prepare(&runtime, event).err().unwrap(), "observation_stale");
        }
        let mut event = observation();
        event.observed_at_unix_ms = runtime.lock().unwrap().delivery_observations["sender"]
            .observed_since_unix_ms
            .saturating_sub(1);
        assert_eq!(
            prepare(&runtime, event).err().unwrap(),
            "observation_sender_unproven"
        );
    }

    fn written(target: &str) -> DirectObservation {
        let mut event = observation();
        event.status = ObservationStatus::OutcomeUnknown;
        event.reason = ObservationReason::ResultMissing;
        event.target = Some(ServerReportedTarget {
            pane_id: target.into(),
            terminal_id: None,
            native_session: None,
        });
        event
    }

    /// PRD direct-prompt-observation-widen B4, B12: a target the command
    /// wrote, a pane id or Herdr's agent name, is the pane Hide projects
    /// for it on the sender's server, proven by the server's fresh answer,
    /// under either correlation.
    #[test]
    fn a_written_pane_or_name_resolves_to_the_projected_recipient() {
        let root = tempfile::tempdir().unwrap();
        let herdr = server("terminal-recipient");
        let runtime = setup(root.path(), &herdr);
        let mut step = written("recipient-name");
        step.runtime_id = "codex".into();
        step.correlation_kind = ObservationCorrelation::ConversationStep;
        for event in [written("recipient"), written("recipient-name"), step] {
            let prepared = prepare(&runtime, event).unwrap();
            let endpoint = prepared.recipient.as_ref().expect("recipient resolved");
            assert_eq!(endpoint.actor.pane_id, "recipient");
            prepared
                .prove_recipient(endpoint, prepared.observation.target.as_ref().unwrap())
                .unwrap();
        }
    }

    /// B6, B7, B8: no recipient for a name nothing holds, a refused input,
    /// or a pane whose session Hide first saw after the call.
    #[test]
    fn unknown_refused_and_later_sessions_gain_no_recipient() {
        let root = tempfile::tempdir().unwrap();
        let herdr = server("terminal-recipient");
        let runtime = setup(root.path(), &herdr);
        let mut refused = written("recipient");
        refused.status = ObservationStatus::RejectedObserved;
        let mut missing = observation();
        missing.target = None;
        let mut changed = observation();
        changed
            .target
            .as_mut()
            .unwrap()
            .native_session
            .as_mut()
            .unwrap()
            .value = "replacement-native".into();
        for event in [written("nobody"), refused, missing, changed] {
            assert!(prepare(&runtime, event).unwrap().recipient.is_none());
        }
        let since =
            runtime.lock().unwrap().delivery_observations["recipient"].observed_since_unix_ms;
        runtime
            .lock()
            .unwrap()
            .delivery_observations
            .get_mut("recipient")
            .unwrap()
            .observed_since_unix_ms = since + 60_000;
        let mut event = written("recipient");
        event.observed_at_unix_ms = since;
        assert!(prepare(&runtime, event).unwrap().recipient.is_none());
    }

    #[test]
    fn server_replacement_cannot_confirm_captured_recipient() {
        let root = tempfile::tempdir().unwrap();
        let herdr = server("replacement-terminal");
        let runtime = setup(root.path(), &herdr);
        let prepared = prepare(&runtime, observation()).unwrap();
        assert_eq!(
            prepared
                .prove_recipient(
                    prepared.recipient.as_ref().unwrap(),
                    prepared.observation.target.as_ref().unwrap()
                )
                .unwrap_err(),
            "observation_native_changed"
        );
    }

    #[test]
    fn lifecycle_replacement_during_proof_refuses_commit() {
        let root = tempfile::tempdir().unwrap();
        let herdr = server("terminal-recipient");
        let runtime = setup(root.path(), &herdr);
        let prepared = prepare(&runtime, observation()).unwrap();
        let fact = DirectFact {
            source_server: prepared.source_server.clone(),
            call_key: hash("call"),
            sender: party(&prepared.sender.actor).unwrap(),
            recipient: Some(party(&prepared.recipient.as_ref().unwrap().actor).unwrap()),
            at_unix_ms: prepared.observation.observed_at_unix_ms,
            outcome: DirectOutcome::Submitted,
            reason: None,
        };
        runtime.lock().unwrap().observe_delivery(
            crate::node::TEST_NODE,
            &lifecycle("replacement-terminal"),
            Some("fixture-server"),
            None,
        );
        assert_eq!(
            runtime
                .lock()
                .unwrap()
                .commit_direct_observation(&prepared, fact)
                .unwrap_err(),
            "observation_native_changed"
        );
    }

    #[test]
    fn oversized_or_unknown_runtime_metadata_is_invalid() {
        let mut event = observation();
        event.call_id = "x".repeat(hide_agent_hooks::observation::ITEM_LIMIT);
        assert!(!valid(&event));
        event = observation();
        event.runtime_id = "not-a-runtime".into();
        assert!(!valid(&event));
    }
}
