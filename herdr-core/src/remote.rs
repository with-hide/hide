//! The projection of a device's Herdr session: what the core makes of the
//! snapshot and events a device's Herdr sends. Reaching the device is the
//! node's (`hide_node::ssh`, behind `hide_node_link::device`); decoding what
//! comes back is the core's, because those are its domain shapes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::{DomainEvent, DomainProjection, DomainSnapshot, HostScope};

pub use crate::herdr_contract::HERDR_PROTOCOL_REVISION as REMOTE_PROTOCOL_REVISION;
pub use hide_node_link::device::{
    Arrived, Attempt, Cancel, CapabilityReport, CapabilityResult, CapabilityState, DEFAULT_CLI_DIR,
    DEFAULT_HELPER_ROOT, DeviceConnector, DeviceTransport, DialedTransport, EstablishError,
    Established, HOST_CONSENT_CARRIED_FROM, HOST_CONSENT_CONTRACT, NodeReady, OnClose,
    RemoteConnectionState, RemoteDiagnostic, RemoteError, RemoteHostIdentity, RemoteResult,
    RemoteStage, SshFailure, SshHostListing, SshProblem, Wake, WslListing,
    valid_remote_socket_path,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RemoteSnapshotEnvelope {
    pub host: HostScope,
    pub protocol: u32,
    pub workspace_ids: Vec<String>,
    pub pane_ids: Vec<String>,
    pub agent_ids: Vec<String>,
}

#[cfg(test)]
pub fn decode_remote_snapshot(
    value: &Value,
    operation_id: &str,
    host: &HostScope,
) -> RemoteResult<RemoteSnapshotEnvelope> {
    crate::wire::remote_snapshot(value, operation_id, host)
}

/// Decodes the snapshot a device's connection test fetched
/// (`hide_node_link::device::SnapshotCheck`): Herdr names no host in its
/// snapshot, so the device's host id and the socket it answered on are the
/// identity the envelope carries.
pub fn check_test_snapshot(value: &Value, host_id: &str, socket: &str) -> RemoteResult<u32> {
    let host = HostScope {
        host_id: host_id.to_owned(),
        session_id: socket.to_owned(),
    };
    crate::wire::remote_snapshot(value, "remote-herdr-snapshot", &host)
        .map(|snapshot| snapshot.protocol)
}

pub struct RemoteHerdrProjection {
    host: RemoteHostIdentity,
    host_scope: HostScope,
    projection: DomainProjection,
    state: RemoteConnectionState,
    last_snapshot: Option<RemoteSnapshotEnvelope>,
    wire_only_snapshot: bool,
    disconnect_reason: Option<String>,
}

impl RemoteHerdrProjection {
    pub fn new(host: RemoteHostIdentity, session_id: impl Into<String>) -> Self {
        let host_scope = HostScope {
            host_id: host.host_id.clone(),
            session_id: session_id.into(),
        };
        Self {
            host,
            host_scope,
            projection: DomainProjection::default(),
            state: RemoteConnectionState::Reconnecting { attempt: 0 },
            last_snapshot: None,
            wire_only_snapshot: false,
            disconnect_reason: None,
        }
    }

    pub fn host(&self) -> &RemoteHostIdentity {
        &self.host
    }

    pub fn host_scope(&self) -> &HostScope {
        &self.host_scope
    }

    pub fn state(&self) -> &RemoteConnectionState {
        &self.state
    }

    pub fn domain(&self) -> &DomainProjection {
        &self.projection
    }

    pub fn last_snapshot(&self) -> Option<&RemoteSnapshotEnvelope> {
        self.last_snapshot.as_ref()
    }

    pub fn disconnect_reason(&self) -> Option<&str> {
        self.disconnect_reason.as_deref()
    }

    pub fn apply_snapshot(&mut self, snapshot: DomainSnapshot) -> RemoteResult<()> {
        validate_snapshot_host(&snapshot, &self.host_scope).map_err(|reason| {
            self.state = RemoteConnectionState::Failed {
                reason: reason.clone(),
            };
            RemoteError::new(
                "remote-snapshot",
                &self.host.host_id,
                RemoteStage::Protocol,
                reason,
                false,
                true,
            )
        })?;
        self.projection.apply_snapshot(snapshot).map_err(|error| {
            self.state = RemoteConnectionState::Failed {
                reason: error.to_string(),
            };
            RemoteError::new(
                "remote-snapshot",
                &self.host.host_id,
                RemoteStage::Protocol,
                error.to_string(),
                true,
                false,
            )
        })?;
        self.last_snapshot = Some(snapshot_envelope(&self.projection, &self.host_scope));
        self.wire_only_snapshot = false;
        self.disconnect_reason = None;
        self.state = RemoteConnectionState::Connected;
        Ok(())
    }

    pub fn apply_event(&mut self, event: DomainEvent) -> RemoteResult<()> {
        if self.wire_only_snapshot {
            let reason =
                "typed domain snapshot is required before applying remote events".to_owned();
            self.state = RemoteConnectionState::Stale {
                reason: reason.clone(),
            };
            return Err(RemoteError::new(
                "remote-event",
                &self.host.host_id,
                RemoteStage::Protocol,
                reason,
                true,
                false,
            ));
        }
        self.projection.apply_event(event).map_err(|error| {
            self.state = RemoteConnectionState::Stale {
                reason: error.to_string(),
            };
            RemoteError::new(
                "remote-event",
                &self.host.host_id,
                RemoteStage::Protocol,
                error.to_string(),
                true,
                false,
            )
        })?;
        self.disconnect_reason = None;
        self.state = RemoteConnectionState::Connected;
        Ok(())
    }

    /// Applies the remote snapshot identity envelope. A full `DomainSnapshot` can be
    /// installed with [`Self::apply_snapshot`] after the caller has decoded the
    /// server-specific layout payload. Keeping that distinction explicit prevents a
    /// decoded ID envelope from being mistaken for a complete event baseline.
    #[cfg(test)]
    pub fn apply_wire_snapshot(&mut self, value: &Value, operation_id: &str) -> RemoteResult<()> {
        let envelope = match decode_remote_snapshot(value, operation_id, &self.host_scope) {
            Ok(envelope) => envelope,
            Err(error) => {
                self.state = RemoteConnectionState::Stale {
                    reason: error.to_string(),
                };
                return Err(error);
            }
        };
        self.last_snapshot = Some(envelope);
        self.wire_only_snapshot = true;
        self.disconnect_reason = None;
        self.state = RemoteConnectionState::Connected;
        Ok(())
    }

    pub fn disconnected(&mut self, reason: impl Into<String>) {
        self.disconnect_reason = Some(reason.into());
        self.state = RemoteConnectionState::Reconnecting { attempt: 0 };
    }

    pub fn reconnecting(&mut self, attempt: u32) {
        self.state = RemoteConnectionState::Reconnecting { attempt };
    }

    pub fn stale_tunnel(&mut self, reason: impl Into<String>) {
        self.state = RemoteConnectionState::Stale {
            reason: reason.into(),
        };
    }

    pub fn protocol_mismatch(&mut self, reason: impl Into<String>) {
        self.state = RemoteConnectionState::ActionRequired {
            reason: reason.into(),
        };
    }
}

fn snapshot_envelope(projection: &DomainProjection, host: &HostScope) -> RemoteSnapshotEnvelope {
    RemoteSnapshotEnvelope {
        host: host.clone(),
        protocol: REMOTE_PROTOCOL_REVISION,
        workspace_ids: projection
            .workspaces()
            .map(|workspace| workspace.workspace_id.clone())
            .collect(),
        pane_ids: projection
            .workspaces()
            .flat_map(|workspace| workspace.tabs.iter())
            .flat_map(|tab| tab.panes.iter())
            .map(|pane| pane.pane_id.clone())
            .collect(),
        agent_ids: projection
            .agents()
            .map(|agent| agent.agent_instance_id.clone())
            .collect(),
    }
}

fn validate_snapshot_host(snapshot: &DomainSnapshot, expected: &HostScope) -> Result<(), String> {
    for workspace in &snapshot.workspaces {
        if workspace.host != *expected {
            return Err(format!(
                "workspace {} belongs to {}:{}, expected {}:{}",
                workspace.workspace_id,
                workspace.host.host_id,
                workspace.host.session_id,
                expected.host_id,
                expected.session_id
            ));
        }
    }
    for agent in &snapshot.agents {
        if agent.host != *expected {
            return Err(format!(
                "agent {} belongs to {}:{}, expected {}:{}",
                agent.agent_instance_id,
                agent.host.host_id,
                agent.host.session_id,
                expected.host_id,
                expected.session_id
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AgentPhase, AgentProjection, DomainEventKind, LayoutNode, PaneProjection, TabProjection,
        WorkspaceProjection,
    };
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::Duration;

    const SSH_OPERATION_TIMEOUT: Duration = Duration::from_secs(15);

    #[test]
    #[ignore = "requires HERDR_TEST_SSH_ALIAS"]
    fn official_remote_socket_snapshot_probe() {
        let alias_name = std::env::var("HERDR_TEST_SSH_ALIAS")
            .expect("HERDR_TEST_SSH_ALIAS names a configured SSH host");
        let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is configured"));
        let transport = hide_node::ssh::Connector::new(None)
            .transport(&home, "probe", &alias_name, None)
            .expect("SSH alias resolves");
        let decoded = Mutex::new(None);
        let report =
            transport.capability_test("remote-herdr-snapshot", &|value, host_id, socket| {
                let host = HostScope {
                    host_id: host_id.to_owned(),
                    session_id: socket.to_owned(),
                };
                let snapshot = decode_remote_snapshot(value, "remote-herdr-snapshot", &host)?;
                let protocol = snapshot.protocol;
                *decoded.lock().unwrap() = Some(snapshot);
                Ok(protocol)
            });
        let snapshot = decoded
            .into_inner()
            .unwrap()
            .unwrap_or_else(|| panic!("official remote Socket API snapshot responds: {report:?}"));
        let connector = transport.herdr_api_connector();
        let agents = hide_herdr_client::request_with_connector(
            &*connector,
            "agent.list",
            json!({}),
            SSH_OPERATION_TIMEOUT,
        )
        .expect("a second channel reuses the authenticated SSH connection");
        assert_eq!(agents["type"], "agent_list");

        let subscription = hide_herdr_client::subscribe_with_connector(
            &*connector,
            crate::wire::subscription_params(&["pane.updated"]).expect("subscription params"),
            SSH_OPERATION_TIMEOUT,
        )
        .expect("official remote event subscription starts");
        assert_eq!(subscription.ack.kind, "subscription_started");
        let (reader, shutdown) = subscription.into_parts();
        shutdown.shutdown();
        drop(reader);

        assert_eq!(snapshot.protocol, REMOTE_PROTOCOL_REVISION);
        assert!(!snapshot.host.host_id.is_empty());
        assert!(!snapshot.host.session_id.is_empty());
    }

    #[test]
    fn remote_wire_snapshot_is_typed_and_protocol_checked() {
        let value = serde_json::json!({
            "result": {"snapshot": {
                "protocol": REMOTE_PROTOCOL_REVISION,
                "version": "fixture", "tabs": [], "layouts": [],
                "workspaces": [{"workspace_id": "w1", "number": 1, "label": "fixture", "focused": false, "pane_count": 1, "tab_count": 1, "active_tab_id": "w1:t1", "agent_status": "idle"}],
                "panes": [{"pane_id": "p1", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}],
                "agents": [
                    {"pane_id": "p1", "terminal_id": "fixture", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1},
                    {"pane_id": "p2", "terminal_id": "fixture2", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}
                ]
            }}
        });
        let envelope = decode_remote_snapshot(&value, "op", &fixture_scope()).unwrap();
        assert_eq!(envelope.host.host_id, "ssh:mini");
        assert_eq!(envelope.workspace_ids, ["w1"]);
        assert_eq!(envelope.pane_ids, ["p1"]);
        assert_eq!(envelope.agent_ids, ["p1", "p2"]);
    }

    fn host() -> RemoteHostIdentity {
        RemoteHostIdentity {
            host_id: "ssh:mini".to_owned(),
            alias: "mini".to_owned(),
            hostname: "mini.example.test".to_owned(),
            port: 2200,
        }
    }

    fn fixture_scope() -> HostScope {
        HostScope {
            host_id: "ssh:mini".to_owned(),
            session_id: "s1".to_owned(),
        }
    }

    #[test]
    fn remote_wire_snapshot_rejects_protocol_mismatch() {
        let value = serde_json::json!({"snapshot": {"protocol": REMOTE_PROTOCOL_REVISION - 1}});
        let error = decode_remote_snapshot(&value, "op", &fixture_scope()).unwrap_err();
        assert_eq!(error.stage(), RemoteStage::Protocol);
        assert!(error.diagnostic().action_required);
    }

    #[test]
    fn remote_wire_snapshot_rejects_duplicate_ids_and_protocol_wraparound() {
        let duplicate = serde_json::json!({
            "snapshot": {
                "protocol": REMOTE_PROTOCOL_REVISION,
                "version": "fixture", "tabs": [], "layouts": [],
                "workspaces": [], "agents": [],
                "panes": [{"pane_id": "p1", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}, {"pane_id": "p1", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}]
            }
        });
        let error = decode_remote_snapshot(&duplicate, "op", &fixture_scope()).unwrap_err();
        assert!(error.diagnostic().reason.contains("duplicate pane_id"));

        let wrapped = serde_json::json!({
            "snapshot": {
                "protocol": u64::from(u32::MAX) + 22
            }
        });
        assert_eq!(
            decode_remote_snapshot(&wrapped, "op", &fixture_scope())
                .unwrap_err()
                .stage(),
            RemoteStage::Protocol
        );
    }

    #[test]
    fn remote_projection_marks_gap_stale_and_host_scope_is_preserved() {
        let alias = host();
        let scope = HostScope {
            host_id: alias.host_id.clone(),
            session_id: "s1".to_owned(),
        };
        let mut projection = RemoteHerdrProjection::new(alias.clone(), "s1");
        projection
            .apply_snapshot(sample_snapshot(scope.clone()))
            .unwrap();
        let error = projection
            .apply_event(DomainEvent {
                sequence: 3,
                kind: DomainEventKind::WorkspaceRenamed {
                    workspace_id: "w1".to_owned(),
                    name: "new".to_owned(),
                },
            })
            .unwrap_err();
        assert_eq!(error.stage(), RemoteStage::Protocol);
        assert!(matches!(
            projection.state(),
            RemoteConnectionState::Stale { .. }
        ));
        projection.apply_snapshot(sample_snapshot(scope)).unwrap();
        assert_eq!(projection.state(), &RemoteConnectionState::Connected);
    }

    #[test]
    fn remote_projection_preserves_domain_and_records_disconnect_reason() {
        let alias = host();
        let scope = HostScope {
            host_id: alias.host_id.clone(),
            session_id: "s1".to_owned(),
        };
        let mut projection = RemoteHerdrProjection::new(alias.clone(), "s1");
        projection.apply_snapshot(sample_snapshot(scope)).unwrap();
        projection.disconnected("transport reset");
        assert_eq!(projection.disconnect_reason(), Some("transport reset"));
        assert_eq!(projection.domain().sequence(), 1);
        assert!(matches!(
            projection.state(),
            RemoteConnectionState::Reconnecting { attempt: 0 }
        ));
    }

    #[test]
    fn sparse_remote_wire_snapshot_requires_typed_bootstrap_for_events() {
        let alias = host();
        let mut projection = RemoteHerdrProjection::new(alias.clone(), "s1");
        let value = serde_json::json!({
            "snapshot": {
                "protocol": REMOTE_PROTOCOL_REVISION,
                "version": "fixture", "tabs": [], "layouts": [],
                "workspaces": [{"workspace_id": "w1", "number": 1, "label": "fixture", "focused": false, "pane_count": 1, "tab_count": 1, "active_tab_id": "w1:t1", "agent_status": "idle"}],
                "panes": [{"pane_id": "p1", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}],
                "agents": [{"pane_id": "p1", "terminal_id": "fixture", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}]
            }
        });
        projection.apply_wire_snapshot(&value, "op").unwrap();
        let error = projection
            .apply_event(DomainEvent {
                sequence: 10,
                kind: DomainEventKind::WorkspaceRenamed {
                    workspace_id: "w1".to_owned(),
                    name: "next".to_owned(),
                },
            })
            .unwrap_err();
        assert!(error.diagnostic().reason.contains("typed domain snapshot"));
        assert!(matches!(
            projection.state(),
            RemoteConnectionState::Stale { .. }
        ));
    }

    #[test]
    fn remote_wire_snapshot_same_identity_is_idempotent() {
        let alias = host();
        let mut projection = RemoteHerdrProjection::new(alias.clone(), "s1");
        let value = serde_json::json!({
            "snapshot": {
                "protocol": REMOTE_PROTOCOL_REVISION,
                "version": "fixture", "tabs": [], "layouts": [],
                "workspaces": [{"workspace_id": "w1", "number": 1, "label": "fixture", "focused": false, "pane_count": 1, "tab_count": 1, "active_tab_id": "w1:t1", "agent_status": "idle"}],
                "panes": [{"pane_id": "p1", "terminal_id": "fixture-terminal", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}],
                "agents": [{"pane_id": "p1", "terminal_id": "fixture", "workspace_id": "w1", "tab_id": "w1:t1", "focused": false, "agent_status": "idle", "revision": 1}]
            }
        });
        projection.apply_wire_snapshot(&value, "op-1").unwrap();
        projection.apply_wire_snapshot(&value, "op-2").unwrap();
        assert_eq!(projection.last_snapshot().unwrap().workspace_ids, ["w1"]);
        assert_eq!(projection.state(), &RemoteConnectionState::Connected);
    }

    #[test]
    fn malformed_wire_snapshot_marks_projection_stale() {
        let alias = host();
        let mut projection = RemoteHerdrProjection::new(alias.clone(), "s1");
        let error = projection
            .apply_wire_snapshot(
                &serde_json::json!({"snapshot": {"protocol": REMOTE_PROTOCOL_REVISION}}),
                "op",
            )
            .unwrap_err();
        assert!(matches!(
            projection.state(),
            RemoteConnectionState::Stale { .. }
        ));
        assert_eq!(error.stage(), RemoteStage::Protocol);
        assert!(error.diagnostic().reason.starts_with("missing field"));
    }

    #[test]
    fn remote_projection_rejects_foreign_workspace() {
        let alias = host();
        let foreign = HostScope {
            host_id: "ssh:other".to_owned(),
            session_id: "s1".to_owned(),
        };
        let mut projection = RemoteHerdrProjection::new(alias.clone(), "s1");
        let error = projection
            .apply_snapshot(sample_snapshot(foreign))
            .unwrap_err();
        assert_eq!(error.stage(), RemoteStage::Protocol);
        assert!(matches!(
            projection.state(),
            RemoteConnectionState::Failed { .. }
        ));
    }

    fn sample_snapshot(host: HostScope) -> DomainSnapshot {
        DomainSnapshot {
            protocol_revision: REMOTE_PROTOCOL_REVISION,
            sequence: 1,
            active_workspace_id: "w1".to_owned(),
            workspaces: vec![WorkspaceProjection {
                host,
                workspace_id: "w1".to_owned(),
                name: "Workspace".to_owned(),
                remote: true,
                active_tab_id: "tab1".to_owned(),
                tabs: vec![TabProjection {
                    tab_id: "tab1".to_owned(),
                    name: "Tab".to_owned(),
                    focused_pane_id: "p1".to_owned(),
                    panes: vec![PaneProjection {
                        pane_id: "p1".to_owned(),
                        title: "Terminal".to_owned(),
                        surface: crate::domain::SurfaceKind::Terminal,
                        agent_instance_id: Some("a1".to_owned()),
                    }],
                    layout: LayoutNode::Pane {
                        pane_id: "p1".to_owned(),
                    },
                }],
                worktree: None,
            }],
            agents: vec![AgentProjection {
                agent_instance_id: "a1".to_owned(),
                parent_agent_instance_id: None,
                host: HostScope {
                    host_id: "ssh:mini".to_owned(),
                    session_id: "s1".to_owned(),
                },
                workspace_id: "w1".to_owned(),
                tab_id: "tab1".to_owned(),
                pane_id: "p1".to_owned(),
                name: "Agent".to_owned(),
                kind: "codex".to_owned(),
                phase: AgentPhase::Working,
                summary: None,
                elapsed_seconds: 1,
            }],
        }
    }
}
