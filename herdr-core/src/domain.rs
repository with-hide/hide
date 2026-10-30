use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::engine::herdr::contract::HERDR_PROTOCOL_REVISION;

/// Shared environment contract shape used by every process boundary.
///
/// `value` is `Some` when the boundary supplies a deterministic child value
/// (for example, the attached PTY); it is `None` when the boundary only reads
/// a value from the host environment. Keeping the shape here prevents each
/// transport from inventing a subtly different contract and validation path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EnvironmentContract {
    pub key: &'static str,
    pub value: Option<&'static str>,
    pub requirement: &'static str,
    pub missing_behavior: &'static str,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct HostScope {
    pub host_id: String,
    pub session_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceKind {
    Terminal,
    Editor,
    Browser,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitAxis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LayoutNode {
    Pane {
        pane_id: String,
    },
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<LayoutNode>,
        second: Box<LayoutNode>,
    },
}

impl LayoutNode {
    fn pane_ids(&self, output: &mut BTreeSet<String>) -> Result<(), ProjectionFailure> {
        match self {
            Self::Pane { pane_id } => {
                if !output.insert(pane_id.clone()) {
                    return Err(ProjectionFailure::InvalidSnapshot(format!(
                        "duplicate layout leaf {pane_id:?}"
                    )));
                }
            }
            Self::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if !(0.05..=0.95).contains(ratio) {
                    return Err(ProjectionFailure::InvalidSnapshot(format!(
                        "split ratio {ratio} is outside 0.05..=0.95"
                    )));
                }
                first.pane_ids(output)?;
                second.pane_ids(output)?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneProjection {
    pub pane_id: String,
    pub title: String,
    pub surface: SurfaceKind,
    pub agent_instance_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TabProjection {
    pub tab_id: String,
    pub name: String,
    pub focused_pane_id: String,
    pub panes: Vec<PaneProjection>,
    pub layout: LayoutNode,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceProjection {
    pub host: HostScope,
    pub workspace_id: String,
    pub name: String,
    pub remote: bool,
    pub active_tab_id: String,
    pub tabs: Vec<TabProjection>,
    pub worktree: Option<WorktreeProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorktreeProjection {
    pub repo_key: String,
    pub repo_name: String,
    pub repo_root: String,
    pub checkout_path: String,
    pub is_linked_worktree: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentPhase {
    Error,
    Attention,
    Working,
    Idle,
    Ended,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentProjection {
    pub agent_instance_id: String,
    pub parent_agent_instance_id: Option<String>,
    pub host: HostScope,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub name: String,
    pub kind: String,
    pub phase: AgentPhase,
    pub summary: Option<String>,
    pub elapsed_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomainSnapshot {
    pub protocol_revision: u32,
    pub sequence: u64,
    pub active_workspace_id: String,
    pub workspaces: Vec<WorkspaceProjection>,
    pub agents: Vec<AgentProjection>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Connected,
    Reconnecting { target: String },
    Stale { expected: u64, received: u64 },
    Failed { reason: String },
    ActionRequired { reason: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DomainEventKind {
    WorkspaceRenamed {
        workspace_id: String,
        name: String,
    },
    TabSelected {
        workspace_id: String,
        tab_id: String,
    },
    PaneFocused {
        workspace_id: String,
        tab_id: String,
        pane_id: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DomainEvent {
    pub sequence: u64,
    pub kind: DomainEventKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectionFailure {
    ProtocolMismatch { expected: u32, received: u32 },
    SequenceGap { expected: u64, received: u64 },
    StaleRequiresSnapshot { expected: u64, received: u64 },
    InvalidSnapshot(String),
    UnknownTarget(String),
}

impl fmt::Display for ProjectionFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ProtocolMismatch { expected, received } => {
                write!(
                    formatter,
                    "protocol mismatch: expected {expected}, received {received}"
                )
            }
            Self::SequenceGap { expected, received } => {
                write!(
                    formatter,
                    "event sequence gap: expected {expected}, received {received}"
                )
            }
            Self::StaleRequiresSnapshot { expected, received } => {
                write!(
                    formatter,
                    "stale projection requires full snapshot: expected {expected}, received {received}"
                )
            }
            Self::InvalidSnapshot(reason) => write!(formatter, "invalid snapshot: {reason}"),
            Self::UnknownTarget(target) => write!(formatter, "unknown projection target: {target}"),
        }
    }
}

impl std::error::Error for ProjectionFailure {}

#[derive(Clone, Debug)]
pub struct DomainProjection {
    sequence: u64,
    connection: ConnectionState,
    active_workspace_id: Option<String>,
    workspaces: BTreeMap<String, WorkspaceProjection>,
    agents: BTreeMap<String, AgentProjection>,
}

impl Default for DomainProjection {
    fn default() -> Self {
        Self {
            sequence: 0,
            connection: ConnectionState::Reconnecting {
                target: "local Herdr".to_owned(),
            },
            active_workspace_id: None,
            workspaces: BTreeMap::new(),
            agents: BTreeMap::new(),
        }
    }
}

impl DomainProjection {
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    pub fn connection(&self) -> &ConnectionState {
        &self.connection
    }

    pub fn workspaces(&self) -> impl Iterator<Item = &WorkspaceProjection> {
        self.workspaces.values()
    }

    pub fn active_workspace(&self) -> Option<&WorkspaceProjection> {
        self.active_workspace_id
            .as_ref()
            .and_then(|workspace_id| self.workspaces.get(workspace_id))
    }

    pub fn agents(&self) -> impl Iterator<Item = &AgentProjection> {
        self.agents.values()
    }

    pub fn worktrees(&self) -> impl Iterator<Item = (&WorkspaceProjection, &WorktreeProjection)> {
        self.workspaces.values().filter_map(|workspace| {
            workspace
                .worktree
                .as_ref()
                .map(|worktree| (workspace, worktree))
        })
    }

    pub fn apply_snapshot(&mut self, snapshot: DomainSnapshot) -> Result<(), ProjectionFailure> {
        validate_snapshot(&snapshot)?;
        if snapshot.protocol_revision != HERDR_PROTOCOL_REVISION {
            let failure = ProjectionFailure::ProtocolMismatch {
                expected: HERDR_PROTOCOL_REVISION,
                received: snapshot.protocol_revision,
            };
            self.connection = ConnectionState::Failed {
                reason: failure.to_string(),
            };
            return Err(failure);
        }
        self.sequence = snapshot.sequence;
        self.active_workspace_id = Some(snapshot.active_workspace_id);
        self.workspaces = snapshot
            .workspaces
            .into_iter()
            .map(|workspace| (workspace.workspace_id.clone(), workspace))
            .collect();
        self.agents = snapshot
            .agents
            .into_iter()
            .map(|agent| (agent.agent_instance_id.clone(), agent))
            .collect();
        self.connection = ConnectionState::Connected;
        Ok(())
    }

    pub fn apply_event(&mut self, event: DomainEvent) -> Result<(), ProjectionFailure> {
        if let ConnectionState::Stale { expected, .. } = self.connection {
            let failure = ProjectionFailure::StaleRequiresSnapshot {
                expected,
                received: event.sequence,
            };
            return Err(failure);
        }
        let expected = self.sequence.saturating_add(1);
        if event.sequence != expected {
            self.connection = ConnectionState::Stale {
                expected,
                received: event.sequence,
            };
            return Err(ProjectionFailure::SequenceGap {
                expected,
                received: event.sequence,
            });
        }
        match event.kind {
            DomainEventKind::WorkspaceRenamed { workspace_id, name } => {
                self.workspace_mut(&workspace_id)?.name = name;
            }
            DomainEventKind::TabSelected {
                workspace_id,
                tab_id,
            } => {
                let workspace = self.workspace_mut(&workspace_id)?;
                if !workspace.tabs.iter().any(|tab| tab.tab_id == tab_id) {
                    return Err(ProjectionFailure::UnknownTarget(tab_id));
                }
                workspace.active_tab_id = tab_id;
            }
            DomainEventKind::PaneFocused {
                workspace_id,
                tab_id,
                pane_id,
            } => {
                let workspace = self.workspace_mut(&workspace_id)?;
                let tab = workspace
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.tab_id == tab_id)
                    .ok_or_else(|| ProjectionFailure::UnknownTarget(tab_id.clone()))?;
                if !tab.panes.iter().any(|pane| pane.pane_id == pane_id) {
                    return Err(ProjectionFailure::UnknownTarget(pane_id));
                }
                tab.focused_pane_id = pane_id;
            }
        }
        self.sequence = event.sequence;
        Ok(())
    }

    fn workspace_mut(
        &mut self,
        workspace_id: &str,
    ) -> Result<&mut WorkspaceProjection, ProjectionFailure> {
        self.workspaces
            .get_mut(workspace_id)
            .ok_or_else(|| ProjectionFailure::UnknownTarget(workspace_id.to_owned()))
    }
}

fn validate_snapshot(snapshot: &DomainSnapshot) -> Result<(), ProjectionFailure> {
    let mut workspace_ids = BTreeSet::new();
    let mut agent_ids = BTreeSet::new();
    for workspace in &snapshot.workspaces {
        if !workspace_ids.insert(workspace.workspace_id.clone()) {
            return Err(ProjectionFailure::InvalidSnapshot(format!(
                "duplicate workspace {:?}",
                workspace.workspace_id
            )));
        }
        let mut tab_ids = BTreeSet::new();
        for tab in &workspace.tabs {
            if !tab_ids.insert(tab.tab_id.clone()) {
                return Err(ProjectionFailure::InvalidSnapshot(format!(
                    "duplicate tab {:?}",
                    tab.tab_id
                )));
            }
            let pane_ids = tab
                .panes
                .iter()
                .map(|pane| pane.pane_id.clone())
                .collect::<BTreeSet<_>>();
            if pane_ids.len() != tab.panes.len() {
                return Err(ProjectionFailure::InvalidSnapshot(format!(
                    "duplicate pane in tab {:?}",
                    tab.tab_id
                )));
            }
            let mut layout_ids = BTreeSet::new();
            tab.layout.pane_ids(&mut layout_ids)?;
            if pane_ids != layout_ids || !pane_ids.contains(&tab.focused_pane_id) {
                return Err(ProjectionFailure::InvalidSnapshot(format!(
                    "tab {:?} layout or focus does not match panes",
                    tab.tab_id
                )));
            }
        }
        if !tab_ids.contains(&workspace.active_tab_id) {
            return Err(ProjectionFailure::InvalidSnapshot(format!(
                "active tab {:?} is absent",
                workspace.active_tab_id
            )));
        }
    }
    if !workspace_ids.contains(&snapshot.active_workspace_id) {
        return Err(ProjectionFailure::InvalidSnapshot(format!(
            "active workspace {:?} is absent",
            snapshot.active_workspace_id
        )));
    }
    for agent in &snapshot.agents {
        if !agent_ids.insert(agent.agent_instance_id.clone()) {
            return Err(ProjectionFailure::InvalidSnapshot(format!(
                "duplicate agent {:?}",
                agent.agent_instance_id
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn fixture_snapshot(sequence: u64) -> DomainSnapshot {
    DomainSnapshot {
        protocol_revision: HERDR_PROTOCOL_REVISION,
        sequence,
        active_workspace_id: "workspace-1".to_owned(),
        workspaces: vec![WorkspaceProjection {
            host: HostScope {
                host_id: "local".to_owned(),
                session_id: "fixture".to_owned(),
            },
            workspace_id: "workspace-1".to_owned(),
            name: "Native IDE".to_owned(),
            remote: false,
            active_tab_id: "tab-1".to_owned(),
            tabs: vec![TabProjection {
                tab_id: "tab-1".to_owned(),
                name: "main".to_owned(),
                focused_pane_id: "pane-terminal".to_owned(),
                panes: vec![
                    PaneProjection {
                        pane_id: "pane-terminal".to_owned(),
                        title: "Terminal".to_owned(),
                        surface: SurfaceKind::Terminal,
                        agent_instance_id: Some("agent-1".to_owned()),
                    },
                    PaneProjection {
                        pane_id: "pane-editor".to_owned(),
                        title: "Editor".to_owned(),
                        surface: SurfaceKind::Editor,
                        agent_instance_id: None,
                    },
                ],
                layout: LayoutNode::Split {
                    axis: SplitAxis::Horizontal,
                    ratio: 0.55,
                    first: Box::new(LayoutNode::Pane {
                        pane_id: "pane-terminal".to_owned(),
                    }),
                    second: Box::new(LayoutNode::Pane {
                        pane_id: "pane-editor".to_owned(),
                    }),
                },
            }],
            worktree: Some(WorktreeProjection {
                repo_key: "hide".to_owned(),
                repo_name: "Hide".to_owned(),
                repo_root: "/fixtures/hide".to_owned(),
                checkout_path: "/fixtures/hide".to_owned(),
                is_linked_worktree: false,
            }),
        }],
        agents: vec![AgentProjection {
            agent_instance_id: "agent-1".to_owned(),
            parent_agent_instance_id: None,
            host: HostScope {
                host_id: "local".to_owned(),
                session_id: "fixture".to_owned(),
            },
            workspace_id: "workspace-1".to_owned(),
            tab_id: "tab-1".to_owned(),
            pane_id: "pane-terminal".to_owned(),
            name: "Codex".to_owned(),
            kind: "codex".to_owned(),
            phase: AgentPhase::Working,
            summary: Some("Building the native shell".to_owned()),
            elapsed_seconds: 42,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_gap_marks_stale_without_applying_event() {
        let mut projection = DomainProjection::default();
        projection.apply_snapshot(fixture_snapshot(40)).unwrap();
        let failure = projection
            .apply_event(DomainEvent {
                sequence: 42,
                kind: DomainEventKind::WorkspaceRenamed {
                    workspace_id: "workspace-1".to_owned(),
                    name: "Must not apply".to_owned(),
                },
            })
            .unwrap_err();
        assert_eq!(
            failure,
            ProjectionFailure::SequenceGap {
                expected: 41,
                received: 42
            }
        );
        assert_eq!(projection.workspaces().next().unwrap().name, "Native IDE");
        assert_eq!(projection.sequence(), 40);
    }

    #[test]
    fn stale_projection_rejects_follow_up_events_until_full_snapshot() {
        let mut projection = DomainProjection::default();
        projection.apply_snapshot(fixture_snapshot(40)).unwrap();
        projection
            .apply_event(DomainEvent {
                sequence: 42,
                kind: DomainEventKind::WorkspaceRenamed {
                    workspace_id: "workspace-1".to_owned(),
                    name: "gap".to_owned(),
                },
            })
            .unwrap_err();

        let failure = projection
            .apply_event(DomainEvent {
                sequence: 41,
                kind: DomainEventKind::WorkspaceRenamed {
                    workspace_id: "workspace-1".to_owned(),
                    name: "must wait for snapshot".to_owned(),
                },
            })
            .unwrap_err();
        assert_eq!(
            failure,
            ProjectionFailure::StaleRequiresSnapshot {
                expected: 41,
                received: 41,
            }
        );
        assert!(matches!(
            projection.connection(),
            ConnectionState::Stale { .. }
        ));
        assert_eq!(projection.workspaces().next().unwrap().name, "Native IDE");
    }

    #[test]
    fn full_snapshot_recovers_stale_projection_with_same_ids() {
        let mut projection = DomainProjection::default();
        projection.apply_snapshot(fixture_snapshot(8)).unwrap();
        let before = projection.workspaces().next().unwrap().tabs[0]
            .panes
            .clone();
        projection
            .apply_event(DomainEvent {
                sequence: 10,
                kind: DomainEventKind::WorkspaceRenamed {
                    workspace_id: "workspace-1".to_owned(),
                    name: "gap".to_owned(),
                },
            })
            .unwrap_err();
        projection.apply_snapshot(fixture_snapshot(12)).unwrap();
        assert_eq!(
            before,
            projection.workspaces().next().unwrap().tabs[0].panes
        );
        assert_eq!(projection.connection(), &ConnectionState::Connected);
    }

    #[test]
    fn invalid_layout_is_rejected_before_projection_changes() {
        let mut projection = DomainProjection::default();
        let mut invalid = fixture_snapshot(1);
        invalid.workspaces[0].tabs[0].layout = LayoutNode::Pane {
            pane_id: "missing".to_owned(),
        };
        assert!(matches!(
            projection.apply_snapshot(invalid),
            Err(ProjectionFailure::InvalidSnapshot(_))
        ));
        assert_eq!(projection.workspaces().count(), 0);
    }

    #[test]
    fn active_workspace_is_derived_from_the_snapshot_not_map_order() {
        let mut snapshot = fixture_snapshot(1);
        let mut second = snapshot.workspaces[0].clone();
        second.workspace_id = "workspace-2".to_owned();
        second.name = "Selected".to_owned();
        snapshot.active_workspace_id = second.workspace_id.clone();
        snapshot.workspaces.push(second);

        let mut projection = DomainProjection::default();
        projection.apply_snapshot(snapshot).unwrap();
        assert_eq!(projection.active_workspace().unwrap().name, "Selected");
    }

    #[test]
    fn missing_active_workspace_is_rejected_before_projection_changes() {
        let mut snapshot = fixture_snapshot(1);
        snapshot.active_workspace_id = "missing".to_owned();
        let mut projection = DomainProjection::default();
        assert!(matches!(
            projection.apply_snapshot(snapshot),
            Err(ProjectionFailure::InvalidSnapshot(_))
        ));
        assert_eq!(projection.workspaces().count(), 0);
    }
}
