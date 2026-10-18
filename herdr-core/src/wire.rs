//! The only conversion boundary between the pinned Herdr schema and the replica.
//! Generated values are consumed into domain inputs; no JSON round trip converts
//! a generated payload back into a hand-written deserialization shape.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use self::generated::{
    error_response as err, event as ev, request as req, success_response as res,
};
use crate::live::SessionFetchError;
use crate::model::PaneLayoutDirection;
use crate::recent_closed::{ClosedLayout, ClosedLayoutNode, ClosedSplitDirection};
use crate::session_sync::{
    PaneMove, ProjectedAgent, ProjectedPane, ProjectedTab, ProjectedWorkspace, ProjectedWorktree,
    ProjectionState, ReplicaEvent, SubscriptionLine,
};
use crate::sidebar::{
    SessionAgentSessionPayload, SessionLayoutPanePayload, SessionLayoutPayload, SessionLayoutRect,
    SessionLayoutSplitPayload,
};
use hide_herdr_client::HERDR_PROTOCOL_REVISION;

/// The types generated from the pinned contract (`build.rs`). The module is
/// private, so this file is the only place in the crate that can name them:
/// anything else that imports one fails to compile.
#[allow(
    dead_code,
    clippy::derivable_impls,
    clippy::enum_variant_names,
    clippy::large_enum_variant
)]
mod generated {
    pub mod request {
        include!(concat!(env!("OUT_DIR"), "/herdr_request.rs"));
    }
    pub mod success_response {
        include!(concat!(env!("OUT_DIR"), "/herdr_success_response.rs"));
    }
    pub mod event {
        include!(concat!(env!("OUT_DIR"), "/herdr_event.rs"));
    }
    pub mod error_response {
        include!(concat!(env!("OUT_DIR"), "/herdr_error_response.rs"));
    }
}

pub(crate) fn protocol_mismatch(
    received_protocol: u64,
    received_version: Option<String>,
) -> SessionFetchError {
    let message = if received_protocol < HERDR_PROTOCOL_REVISION {
        format!(
            "The running Herdr uses protocol {received_protocol}, but Hide requires protocol {HERDR_PROTOCOL_REVISION}. When your current work is safe, stop the Herdr session and reopen Hide. Hide will start its compatible bundled Herdr. No workspace or agent was created."
        )
    } else {
        format!(
            "The running Herdr uses protocol {received_protocol}, but this Hide supports protocol {HERDR_PROTOCOL_REVISION}. Update Hide to a compatible release, then try again. No workspace or agent was created."
        )
    };
    SessionFetchError::Protocol {
        message,
        expected_protocol: HERDR_PROTOCOL_REVISION,
        received_protocol,
        received_version,
    }
}

/// What a device's row says when its Herdr speaks an older protocol than
/// this build: the device's node hands its server to the Herdr Hide brings
/// when it connects, and the step left is the device's own.
pub(crate) fn device_protocol_behind(received_protocol: u64, version: Option<&str>) -> String {
    let herdr = match version {
        Some(version) => format!("Herdr {version}"),
        None => "Herdr".to_owned(),
    };
    format!(
        "This device's {herdr} uses protocol {received_protocol}, but Hide requires protocol {HERDR_PROTOCOL_REVISION}. Hide moves the device's Herdr server to its own Herdr when the device connects; if it stays, restart Herdr on the device when its work is safe, or update that machine's Herdr."
    )
}

fn malformed(message: impl Into<String>) -> SessionFetchError {
    SessionFetchError::Malformed(message.into())
}

// Preflight preserves the existing diagnostic priority and exact messages.
// It does not supply defaults: a successful preflight still has to deserialize
// the complete generated snapshot, including fields the projection discards.
fn validate_snapshot(value: &Value) -> Result<(), SessionFetchError> {
    let protocol = value
        .get("protocol")
        .and_then(Value::as_u64)
        .ok_or_else(|| malformed("snapshot is missing protocol"))?;
    let version = value
        .get("version")
        .and_then(Value::as_str)
        .filter(|version| !version.trim().is_empty());
    if protocol != HERDR_PROTOCOL_REVISION {
        return Err(protocol_mismatch(protocol, version.map(str::to_owned)));
    }
    if version.is_none() {
        return Err(malformed("snapshot is missing version"));
    }
    for field in ["workspaces", "tabs", "panes", "layouts", "agents"] {
        if !value.get(field).is_some_and(Value::is_array) {
            return Err(malformed(format!("snapshot is missing {field}")));
        }
    }
    Ok(())
}

pub(crate) fn snapshot(value: Value) -> Result<ProjectionState, SessionFetchError> {
    validate_snapshot(&value)?;
    let snapshot: res::SessionSnapshot = serde_json::from_value(value)
        .map_err(|e| malformed(format!("snapshot projection is malformed: {e}")))?;
    Ok(convert_snapshot(snapshot))
}

pub(crate) fn snapshot_response(value: Value) -> Result<ProjectionState, SessionFetchError> {
    decode_snapshot_response(value).map(convert_snapshot)
}

/// Cleanup protects both the launch directory and any current foreground
/// directory, without changing the navigation projection's ownership policy.
/// Each folder is paired with its pane, so a checkout's own panes can be told
/// from a pane that merely stands inside it.
pub(crate) fn cleanup_usage_paths(
    value: Value,
) -> Result<Vec<(String, Option<String>)>, SessionFetchError> {
    let snapshot = decode_snapshot_response(value)?;
    let mut paths = Vec::new();
    for (pane, cwd, foreground) in snapshot
        .panes
        .into_iter()
        .map(|p| (p.pane_id, p.cwd, p.foreground_cwd))
        .chain(
            snapshot
                .agents
                .into_iter()
                .map(|a| (a.pane_id, a.cwd, a.foreground_cwd)),
        )
    {
        if cwd.is_none() && foreground.is_none() {
            paths.push((pane.clone(), None));
        }
        paths.extend(
            cwd.into_iter()
                .chain(foreground)
                .map(|path| (pane.clone(), Some(herdr_path(path)))),
        );
    }
    Ok(paths)
}

/// A path Herdr reported, in the wire spelling (`hide_platform::path`).
/// Herdr spells a path as its own system does, so on this machine that is
/// the native spelling and this is the one place it becomes the wire's. A
/// path a device's Herdr reported is that device's: no system reads
/// another's spelling as absolute (`C:\\x` is not absolute on macOS, nor
/// `/x` on Windows), so it is left as it came; a Windows device's Herdr
/// seen from a Mac is not spelled yet (docs/ARCHITECTURE.md, The platform
/// layer).
fn herdr_path(path: String) -> String {
    match hide_platform::path::to_wire(std::path::Path::new(&path)) {
        Ok(wire) => wire,
        Err(_) => path,
    }
}

/// A wire spelling sent to Herdr as its own system spells it, the way back
/// of [`herdr_path`]: on this machine the native path, and a device's path
/// as it came.
fn herdr_param(path: &str) -> String {
    match hide_platform::path::from_wire(path) {
        Ok(native) => native.to_string_lossy().into_owned(),
        Err(_) => path.to_owned(),
    }
}

fn decode_snapshot_response(value: Value) -> Result<res::SessionSnapshot, SessionFetchError> {
    validate_snapshot(
        value
            .get("snapshot")
            .ok_or_else(|| malformed("response is missing snapshot"))?,
    )?;
    let response: res::ResponseResult = serde_json::from_value(value)
        .map_err(|e| malformed(format!("snapshot projection is malformed: {e}")))?;
    match response {
        res::ResponseResult::SessionSnapshot { snapshot } => Ok(snapshot),
        _ => Err(malformed("response is missing snapshot")),
    }
}

fn convert_snapshot(snapshot: res::SessionSnapshot) -> ProjectionState {
    ProjectionState {
        tab_focus: None,
        tab_moves: None,
        focused_pane_id: snapshot.focused_pane_id,
        focused_workspace_id: snapshot.focused_workspace_id,
        workspaces: snapshot.workspaces.into_iter().map(Into::into).collect(),
        tabs: snapshot.tabs.into_iter().map(Into::into).collect(),
        panes: snapshot.panes.into_iter().map(Into::into).collect(),
        layouts: snapshot.layouts.into_iter().map(Into::into).collect(),
        agents: snapshot.agents.into_iter().map(Into::into).collect(),
        herdr_version: Some(snapshot.version),
    }
}

pub(crate) fn agents_response(value: Value) -> Result<Vec<ProjectedAgent>, SessionFetchError> {
    if let Some(kind) = value.get("type").and_then(Value::as_str)
        && kind != "agent_list"
    {
        return Err(malformed(format!(
            "agent.list returned unexpected result type {kind:?}"
        )));
    }
    let response: res::ResponseResult = serde_json::from_value(value)
        .map_err(|e| malformed(format!("agent.list response is malformed: {e}")))?;
    match response {
        res::ResponseResult::AgentList { agents } => {
            Ok(agents.into_iter().map(Into::into).collect())
        }
        _ => unreachable!("the result discriminator was checked before deserialization"),
    }
}

pub(crate) fn parse_subscription_line(line: &str) -> Result<SubscriptionLine, SessionFetchError> {
    if line.trim().is_empty() {
        return Err(malformed("Herdr event stream emitted an empty line"));
    }
    let value: Value = serde_json::from_str(line)
        .map_err(|e| malformed(format!("Herdr event stream emitted invalid JSON: {e}")))?;
    if let Some(error) = value.get("error") {
        let id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| malformed("Herdr subscription error is missing id"))?;
        if id != "herdr-core:events.subscribe" {
            return Err(malformed(format!(
                "Herdr subscription error id {id:?} is unexpected"
            )));
        }
        for field in ["code", "message"] {
            if error.get(field).and_then(Value::as_str).is_none() {
                return Err(malformed(format!(
                    "Herdr subscription error is missing {field}"
                )));
            }
        }
        let response: err::ErrorResponse = serde_json::from_value(value)
            .map_err(|e| malformed(format!("Herdr subscription error is malformed: {e}")))?;
        // The schema leaves error codes open; the Socket API names this one
        // for a subscriber that fell behind the server's event history.
        if response.error.code == "events_lost" {
            return Ok(SubscriptionLine::EventsLost {
                message: response.error.message,
            });
        }
        return Ok(SubscriptionLine::Error {
            code: response.error.code,
            message: response.error.message,
        });
    }
    let envelope: ev::EventEnvelope = serde_json::from_value(value)
        .map_err(|e| malformed(format!("Herdr event is malformed: {e}")))?;
    let kind = envelope.event.to_string();
    let (actual, data) = convert_event(envelope.data);
    if actual != kind {
        return Err(malformed(format!(
            "Herdr {kind} event event data type is {actual:?}"
        )));
    }
    Ok(SubscriptionLine::Event(data))
}

// The two independently generated sub-schemas describe the same record shapes.
// Expand explicit field moves for each, keeping all generated type references here.
macro_rules! record_conversions {
    ($wire:ident) => {
        impl From<$wire::WorkspaceInfo> for ProjectedWorkspace {
            fn from(v: $wire::WorkspaceInfo) -> Self {
                Self {
                    workspace_id: v.workspace_id,
                    label: v.label,
                    active_tab_id: v.active_tab_id,
                    tokens: v
                        .tokens
                        .into_iter()
                        .map(|(key, value)| (String::from(key), Value::String(value)))
                        .collect(),
                    worktree: v.worktree.map(Into::into),
                }
            }
        }
        impl From<$wire::WorkspaceWorktreeInfo> for ProjectedWorktree {
            fn from(v: $wire::WorkspaceWorktreeInfo) -> Self {
                Self {
                    repo_key: v.repo_key,
                    repo_name: v.repo_name,
                    repo_root: herdr_path(v.repo_root),
                    checkout_path: herdr_path(v.checkout_path),
                    is_linked_worktree: v.is_linked_worktree,
                }
            }
        }
        impl From<$wire::TabInfo> for ProjectedTab {
            fn from(v: $wire::TabInfo) -> Self {
                Self {
                    tab_id: v.tab_id,
                    workspace_id: v.workspace_id,
                    number: v.number,
                    label: v.label,
                }
            }
        }
        impl From<$wire::PaneInfo> for ProjectedPane {
            fn from(v: $wire::PaneInfo) -> Self {
                Self {
                    terminal_id: v.terminal_id,
                    restore_error: v.restore_error,
                    pane_id: v.pane_id,
                    foreground_process: None,
                    agent_status: v.agent_status.to_string(),
                    workspace_id: v.workspace_id,
                    tab_id: v.tab_id,
                    cwd: v.cwd.map(herdr_path),
                    tokens: v
                        .tokens
                        .into_iter()
                        .map(|(k, v)| (k.into(), Value::String(v)))
                        .collect(),
                    label: v.label,
                    terminal_title: v.terminal_title,
                    terminal_title_stripped: v.terminal_title_stripped,
                }
            }
        }
        impl From<$wire::PaneLayoutRect> for SessionLayoutRect {
            fn from(v: $wire::PaneLayoutRect) -> Self {
                Self {
                    x: v.x,
                    y: v.y,
                    width: v.width,
                    height: v.height,
                }
            }
        }
        impl From<$wire::PaneLayoutSnapshot> for SessionLayoutPayload {
            fn from(v: $wire::PaneLayoutSnapshot) -> Self {
                Self {
                    workspace_id: v.workspace_id,
                    tab_id: v.tab_id,
                    zoomed: v.zoomed,
                    area: v.area.into(),
                    focused_pane_id: v.focused_pane_id,
                    panes: v
                        .panes
                        .into_iter()
                        .map(|p| SessionLayoutPanePayload {
                            pane_id: p.pane_id,
                            rect: p.rect.into(),
                        })
                        .collect(),
                    splits: v
                        .splits
                        .into_iter()
                        .map(|s| SessionLayoutSplitPayload {
                            direction: match s.direction {
                                $wire::SplitDirection::Right => PaneLayoutDirection::Right,
                                $wire::SplitDirection::Down => PaneLayoutDirection::Down,
                            },
                            ratio: s.ratio,
                            rect: s.rect.into(),
                        })
                        .collect(),
                }
            }
        }
    };
}
record_conversions!(res);
record_conversions!(ev);

/// The pane metadata token by which whoever created a pane declares the pane
/// it was spawned from. It is the only lineage there is: Herdr records none,
/// so an agent is a child exactly when its spawner said so.
///
/// hcoord is the sole writer, including for a child created by Hide's Fork.
/// It declares the token after both executions exist with
/// `pane.report_metadata`, the same display-only channel the hook helper
/// already uses. The value is the parent's pane id, it dies
/// with the pane, and Hide reads it here and nowhere else.
///
/// The token outlives the agent that earned it, because it dies with the
/// pane and a pane can host another agent later, so it is only a claim until
/// `CHILD_SESSION_TOKEN` and `PARENT_SESSION_TOKEN` prove that both panes
/// still host the sessions it was written for.
pub(crate) const PARENT_PANE_TOKEN: &str = "parent_pane";

/// The digest of the child's agent session when the relationship was written.
/// A relationship holds only while the child's pane still reports that session.
pub(crate) const CHILD_SESSION_TOKEN: &str = "child_session";

/// The digest of the parent's agent session when the relationship was written.
/// `crate::agent_state::apply_lineage` compares it with the parent pane's current session,
/// because only there are both rows at hand, on this machine or another.
pub(crate) const PARENT_SESSION_TOKEN: &str = "parent_session";

/// The pane a fork was forked from, written by `crate::coordination` on the
/// pane that hosts a fork's registration (`AgentRecord::forked` with its
/// `origin`), so a wake that continues the registration in another pane writes
/// it there too. It is provenance, never responsibility: a fork is the
/// operator's root, which is why it is not `PARENT_PANE_TOKEN`. Like that
/// token it is a claim that holds only while `FORK_SESSION_TOKEN` and
/// `FORK_SOURCE_SESSION_TOKEN` still name the sessions the two panes host.
pub(crate) const FORK_PANE_TOKEN: &str = "fork_pane";

/// The digest of the fork's own session when the provenance was written. A
/// pane that hosts another session later is not a fork.
pub(crate) const FORK_SESSION_TOKEN: &str = "fork_session";

/// The digest of the source's session when the provenance was written. The
/// source is named only while its pane still hosts it
/// (`crate::agent_state::apply_fork_sources`).
pub(crate) const FORK_SOURCE_SESSION_TOKEN: &str = "fork_source_session";

/// Hide's mark on a pane whose saved read state it vouches for, written by
/// the lineage writer (`crate::coordination::lineage`). Herdr keeps a pane
/// token only for as long as the server that took it, so a pane that still
/// carries the mark a saved state names is on the server that state was read
/// from, and its `state_change_seq` can be compared with the saved one
/// (`crate::agent_state::apply_read_state`).
pub(crate) const SERVER_MARK_TOKEN: &str = "hide_server";

/// The server mark a pane carries, when it carries one.
pub(crate) fn server_mark(tokens: &std::collections::BTreeMap<String, Value>) -> Option<String> {
    tokens
        .get(SERVER_MARK_TOKEN)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|mark| !mark.is_empty())
        .map(str::to_owned)
}

/// The tokens a sleeping fork's row stands in for, as its pane carried them.
pub(crate) fn fork_tokens(
    source_pane: &str,
    own_session: Option<&str>,
    source_session: Option<&str>,
) -> std::collections::BTreeMap<String, Value> {
    [
        (FORK_PANE_TOKEN, Some(source_pane)),
        (FORK_SESSION_TOKEN, own_session),
        (FORK_SOURCE_SESSION_TOKEN, source_session),
    ]
    .into_iter()
    .filter_map(|(key, value)| Some((key.to_owned(), Value::String(value?.to_owned()))))
    .collect()
}

/// What a fork's pane declares about its source: the source's pane and the
/// session it hosted, read only while the pane still hosts the fork's own
/// session (`own_session`, its digest).
pub(crate) fn fork_declaration(
    tokens: &std::collections::BTreeMap<String, Value>,
    own_session: Option<&str>,
) -> Option<(String, Option<String>)> {
    let token = |name: &str| lineage_parent(tokens.get(name).and_then(Value::as_str));
    if !session_holds(token(FORK_SESSION_TOKEN).as_deref(), own_session) {
        return None;
    }
    Some((token(FORK_PANE_TOKEN)?, token(FORK_SOURCE_SESSION_TOKEN)))
}

/// How a session is written into a token: the lowercase hex SHA-256 of the
/// value Herdr reports in `agent_session.value`. Herdr cuts a token value at
/// 80 characters and a session can be a path, so a digest is the one form
/// that keeps two sessions apart whatever they look like; hcoord writes the
/// same function. A blank value names no session.
pub(crate) fn session_digest(value: &str) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    Some(
        Sha256::digest(value.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

/// Whether a pane still hosts the session a relationship was written for.
/// A missing side proves nothing, so it does not hold: a pane Herdr reports
/// without a session (Codex before its first turn) cannot confirm a match, and
/// a token written before sessions were recorded has nothing to compare.
pub(crate) fn session_holds(declared: Option<&str>, current: Option<&str>) -> bool {
    matches!((declared, current), (Some(declared), Some(current)) if declared == current)
}

/// Stable machine identity paired with `parent_pane` for cross-device
/// lineage. Same-machine children omit it so an ordinary local parent keeps
/// the existing device-scoped path.
pub(crate) const PARENT_MACHINE_TOKEN: &str = "parent_machine";

/// The source under which Hide writes its own pane tokens. Herdr keeps one
/// token set per source, so a token Hide wrote can never overwrite one an
/// orchestrator or the hook helper wrote under theirs.
pub(crate) const HIDE_METADATA_SOURCE: &str = "hide";

/// The parent a spawner declared. An empty value is no declaration, because
/// Herdr clears a token by setting it empty.
fn lineage_parent(declared: Option<&str>) -> Option<String> {
    declared
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
}

impl From<res::AgentInfo> for ProjectedAgent {
    fn from(v: res::AgentInfo) -> Self {
        let token = |name: &str| {
            v.tokens
                .iter()
                .find(|(key, _)| key.as_str() == name)
                .map(|(_, value)| value.as_str())
        };
        // A relationship is the spawner's claim plus the child still being the
        // session the claim was written for. The parent's session is checked
        // where the parent's row is known (`crate::agent_state::apply_lineage`).
        let own_session = v
            .agent_session
            .as_ref()
            .and_then(|session| session_digest(&session.value));
        let child_holds = session_holds(
            lineage_parent(token(CHILD_SESSION_TOKEN)).as_deref(),
            own_session.as_deref(),
        );
        let declared = |name: &str| child_holds.then(|| lineage_parent(token(name))).flatten();
        let spawned_from_pane_id = declared(PARENT_PANE_TOKEN);
        let spawned_from_machine_id = declared(PARENT_MACHINE_TOKEN);
        let declared_parent_session = declared(PARENT_SESSION_TOKEN);
        Self {
            pane_id: v.pane_id,
            // `terminal_id` and `launch_pending` serve only the start settle,
            // a workaround for herdrdev/herdr#3853 that goes with them.
            terminal_id: v.terminal_id,
            name: v.name,
            workspace_id: v.workspace_id,
            tab_id: v.tab_id,
            cwd: v.cwd.map(herdr_path),
            agent: v.agent,
            agent_status: Some(v.agent_status.to_string()),
            agent_session: v.agent_session.map(|s| SessionAgentSessionPayload {
                kind: s.kind.to_string(),
                value: s.value,
            }),
            spawned_from_pane_id,
            spawned_from_machine_id,
            declared_parent_session,
            lineage_session: own_session,
            state_change_seq: v.state_change_seq,
            completion_seq: v.completion_seq,
            launch_pending: v.launch_pending == Some(true),
            tokens: v
                .tokens
                .into_iter()
                .map(|(k, v)| (k.into(), Value::String(v)))
                .collect(),
        }
    }
}

fn convert_event(data: ev::EventData) -> (&'static str, ReplicaEvent) {
    match data {
        ev::EventData::WorkspaceCreated { workspace, .. } => (
            "workspace_created",
            ReplicaEvent::WorkspaceCreated {
                workspace: workspace.into(),
            },
        ),
        ev::EventData::WorkspaceUpdated { workspace, .. } => (
            "workspace_updated",
            ReplicaEvent::WorkspaceUpdated {
                workspace: workspace.into(),
            },
        ),
        ev::EventData::WorkspaceMetadataUpdated { workspace, .. } => (
            "workspace_metadata_updated",
            ReplicaEvent::WorkspaceUpdated {
                workspace: workspace.into(),
            },
        ),
        ev::EventData::WorkspaceClosed { workspace_id, .. } => (
            "workspace_closed",
            ReplicaEvent::WorkspaceClosed { workspace_id },
        ),
        ev::EventData::WorkspaceRenamed {
            label,
            workspace_id,
            ..
        } => (
            "workspace_renamed",
            ReplicaEvent::WorkspaceRenamed {
                label,
                workspace_id,
            },
        ),
        ev::EventData::WorkspaceMoved {
            insert_index,
            workspace_id,
            workspaces,
            ..
        } => (
            "workspace_moved",
            ReplicaEvent::WorkspaceMoved {
                insert_index: insert_index as usize,
                workspace_id,
                workspaces: workspaces.into_iter().map(Into::into).collect(),
            },
        ),
        ev::EventData::WorkspaceReordered {
            workspace_ids,
            workspaces,
            ..
        } => (
            "workspace_reordered",
            ReplicaEvent::WorkspaceReordered {
                workspace_ids,
                workspaces: workspaces.into_iter().map(Into::into).collect(),
            },
        ),
        ev::EventData::WorkspaceFocused { workspace_id, .. } => (
            "workspace_focused",
            ReplicaEvent::WorkspaceFocused { workspace_id },
        ),
        ev::EventData::WorktreeCreated { workspace, .. } => (
            "worktree_created",
            ReplicaEvent::WorktreeCreated {
                workspace: workspace.into(),
            },
        ),
        ev::EventData::WorktreeOpened { workspace, .. } => (
            "worktree_opened",
            ReplicaEvent::WorktreeOpened {
                workspace: workspace.into(),
            },
        ),
        ev::EventData::WorktreeRemoved {
            workspace,
            workspace_id,
            ..
        } => (
            "worktree_removed",
            ReplicaEvent::WorktreeRemoved {
                workspace: workspace.map(Into::into),
                workspace_id,
            },
        ),
        ev::EventData::TabCreated { tab, .. } => (
            "tab_created",
            ReplicaEvent::TabCreated {
                focused: tab.focused,
                tab: tab.into(),
            },
        ),
        ev::EventData::TabClosed {
            tab_id,
            workspace_id,
            ..
        } => (
            "tab_closed",
            ReplicaEvent::TabClosed {
                tab_id,
                workspace_id,
            },
        ),
        ev::EventData::TabRenamed {
            label,
            tab_id,
            workspace_id,
            ..
        } => (
            "tab_renamed",
            ReplicaEvent::TabRenamed {
                label,
                tab_id,
                workspace_id,
            },
        ),
        ev::EventData::TabMoved {
            insert_index,
            tab_id,
            tabs,
            workspace_id,
            ..
        } => (
            "tab_moved",
            ReplicaEvent::TabMoved {
                insert_index: insert_index as usize,
                tab_id,
                tabs: tabs.into_iter().map(Into::into).collect(),
                workspace_id,
            },
        ),
        ev::EventData::TabFocused {
            tab_id,
            workspace_id,
            ..
        } => (
            "tab_focused",
            ReplicaEvent::TabFocused {
                tab_id,
                workspace_id,
            },
        ),
        ev::EventData::PaneCreated { pane, .. } => (
            "pane_created",
            ReplicaEvent::PaneCreated { pane: pane.into() },
        ),
        ev::EventData::PaneClosed {
            pane_id,
            workspace_id,
            ..
        } => (
            "pane_closed",
            ReplicaEvent::PaneClosed {
                pane_id,
                workspace_id,
            },
        ),
        ev::EventData::PaneUpdated { pane, .. } => (
            "pane_updated",
            ReplicaEvent::PaneUpdated { pane: pane.into() },
        ),
        ev::EventData::PaneFocused {
            pane_id,
            workspace_id,
            ..
        } => (
            "pane_focused",
            ReplicaEvent::PaneFocused {
                pane_id,
                workspace_id,
            },
        ),
        ev::EventData::PaneExited {
            pane_id,
            workspace_id,
            ..
        } => (
            "pane_exited",
            ReplicaEvent::PaneExited {
                pane_id,
                workspace_id,
            },
        ),
        ev::EventData::PaneAgentDetected {
            pane_id,
            workspace_id,
            ..
        } => (
            "pane_agent_detected",
            ReplicaEvent::PaneAgentDetected {
                pane_id,
                workspace_id,
            },
        ),
        ev::EventData::LayoutUpdated { layout, .. } => (
            "layout_updated",
            ReplicaEvent::LayoutUpdated {
                layout: layout.into(),
            },
        ),
        ev::EventData::PaneMoved {
            previous_pane_id,
            previous_workspace_id,
            previous_tab_id,
            pane,
            created_workspace,
            created_tab,
            closed_workspace_id,
            closed_tab_id,
        } => (
            "pane_moved",
            ReplicaEvent::PaneMoved(PaneMove {
                previous_pane_id,
                previous_workspace_id,
                previous_tab_id,
                pane: pane.into(),
                created_workspace: created_workspace.map(Into::into),
                created_tab: created_tab.map(Into::into),
                closed_workspace_id,
                closed_tab_id,
            }),
        ),
        ev::EventData::PaneOutputChanged { .. } => (
            "pane_output_changed",
            ReplicaEvent::Unrequested("pane_output_changed".to_owned()),
        ),
        ev::EventData::PaneAgentStatusChanged { .. } => (
            "pane_agent_status_changed",
            ReplicaEvent::Unrequested("pane_agent_status_changed".to_owned()),
        ),
    }
}

fn params(value: impl serde::Serialize) -> Result<Value, String> {
    serde_json::to_value(value)
        .map_err(|error| format!("Herdr parameters could not be encoded: {error}"))
}

// session.snapshot has no parameters in the pinned request schema.
pub(crate) fn empty_params() -> Value {
    Value::Object(Default::default())
}

pub(crate) fn workspace_create_with_env_params(
    cwd: &str,
    label: &str,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
) -> Result<Value, String> {
    params(req::WorkspaceCreateParams {
        cwd: Some(herdr_param(cwd)),
        label: Some(label.into()),
        focus,
        env: env.into_iter().collect(),
        source_workspace_id: None,
    })
}
pub(crate) fn workspace_target_params(id: &str) -> Result<Value, String> {
    params(req::WorkspaceTarget {
        workspace_id: id.into(),
    })
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct IssueTokens {
    pub panes: std::collections::BTreeMap<String, String>,
    pub workspaces: std::collections::BTreeMap<String, String>,
}

pub(crate) fn issue_tokens(payload: &crate::sidebar::SessionSnapshotPayload) -> IssueTokens {
    fn issue(tokens: &std::collections::BTreeMap<String, Value>) -> Option<String> {
        tokens
            .get("issue")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    }
    let mut panes: std::collections::BTreeMap<_, _> = payload
        .agents
        .iter()
        .filter_map(|agent| {
            Some((
                agent.pane_id.as_ref().or(agent.id.as_ref())?.clone(),
                issue(&agent.tokens)?,
            ))
        })
        .collect();
    for pane in &payload.panes {
        if let Some(value) = issue(&pane.tokens) {
            panes.insert(pane.pane_id.clone(), value);
        }
    }
    IssueTokens {
        panes,
        workspaces: payload
            .workspaces
            .iter()
            .filter_map(|workspace| {
                Some((workspace.workspace_id.clone(), issue(&workspace.tokens)?))
            })
            .collect(),
    }
}

pub(crate) fn workspace_issue_params(
    workspace_id: &str,
    issue: Option<&str>,
) -> Result<Value, String> {
    workspace_metadata_params(workspace_id, "issue", issue)
}

/// `worktree.open` for one checkout path, from its repository's main
/// worktree: Herdr answers with the workspace it already binds to that
/// checkout, or binds or opens one (`already_open`). It carries no label,
/// because Herdr applies one to a workspace that was already open too.
pub(crate) fn worktree_open_params(
    path: &str,
    repository_root: &str,
    focus: bool,
) -> Result<Value, String> {
    params(req::WorktreeOpenParams {
        branch: None,
        cwd: Some(herdr_param(repository_root)),
        focus,
        label: None,
        path: Some(herdr_param(path)),
        trust_repository: None,
        workspace_id: None,
    })
}

/// What `worktree.open` answered: the bound workspace, its active tab and
/// pane, and whether Herdr had it open before this request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OpenedWorktree {
    pub(crate) tab_label: String,
    pub(crate) workspace_id: String,
    pub(crate) label: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    pub(crate) already_open: bool,
}

pub(crate) fn opened_worktree(value: Value) -> Result<OpenedWorktree, String> {
    let missing = "worktree.open response is missing workspace, tab or root pane";
    match response(value, missing)? {
        res::ResponseResult::WorktreeOpened {
            workspace,
            tab,
            root_pane,
            already_open,
            ..
        } => Ok(OpenedWorktree {
            tab_label: tab.label,
            label: workspace.label,
            workspace_id: nonempty_id(workspace.workspace_id, missing)?,
            tab_id: nonempty_id(tab.tab_id, missing)?,
            pane_id: nonempty_id(root_pane.pane_id, missing)?,
            already_open,
        }),
        _ => Err(missing.into()),
    }
}

/// Each workspace's id and the value of one of its metadata tokens, from a
/// `workspace.list` answer.
pub(crate) fn listed_workspace_tokens(
    value: Value,
    token: &str,
) -> Result<Vec<(String, Option<String>)>, String> {
    let missing = "workspace.list response is missing workspaces";
    match response(value, missing)? {
        res::ResponseResult::WorkspaceList { workspaces } => Ok(workspaces
            .into_iter()
            .map(|workspace| {
                let mark = workspace
                    .tokens
                    .into_iter()
                    .find(|(key, _)| String::from(key.clone()) == token)
                    .map(|(_, value)| value);
                (workspace.workspace_id, mark)
            })
            .collect()),
        _ => Err(missing.into()),
    }
}

pub(crate) fn workspace_rename_params(workspace_id: &str, label: &str) -> Result<Value, String> {
    params(req::WorkspaceRenameParams {
        workspace_id: workspace_id.into(),
        label: label.into(),
    })
}

pub(crate) fn workspace_list_params() -> Result<Value, String> {
    params(req::EmptyParams(Default::default()))
}

/// Hide's owner mark on a plain-folder workspace it opened (`checkout_owner`).
pub(crate) fn workspace_owner_mark_params(workspace_id: &str, mark: &str) -> Result<Value, String> {
    workspace_metadata_params(workspace_id, crate::checkout_owner::OWNER_TOKEN, Some(mark))
}

pub(crate) fn workspace_purpose_params(
    workspace_id: &str,
    purpose: Option<&str>,
) -> Result<Value, String> {
    workspace_metadata_params(workspace_id, "purpose", purpose)
}

fn workspace_metadata_params(
    workspace_id: &str,
    name: &str,
    purpose: Option<&str>,
) -> Result<Value, String> {
    let key = name
        .try_into()
        .map_err(|error| format!("invalid workspace purpose token key: {error}"))?;
    params(req::WorkspaceReportMetadataParams {
        workspace_id: workspace_id.to_owned(),
        source: HIDE_METADATA_SOURCE.to_owned(),
        tokens: std::collections::HashMap::from([(key, purpose.map(str::to_owned))]),
        seq: None,
        ttl_ms: None,
    })
}
pub(crate) fn tab_target_params(id: &str) -> Result<Value, String> {
    params(req::TabTarget { tab_id: id.into() })
}
pub(crate) fn pane_target_params(id: &str) -> Result<Value, String> {
    params(req::PaneTarget { pane_id: id.into() })
}
pub(crate) fn pane_scroll_params(pane_id: &str, offset_from_bottom: u64) -> Result<Value, String> {
    params(req::PaneScrollParams {
        pane_id: pane_id.into(),
        offset_from_bottom,
    })
}
/// The scroll metrics a `pane.get` or `pane.scroll` answer carries; `None`
/// when Herdr reports none for the pane.
pub(crate) fn pane_scroll(value: Value) -> Result<Option<crate::live::PaneScroll>, String> {
    let missing = "pane response is missing pane";
    match response(value, missing)? {
        res::ResponseResult::PaneInfo { pane } => {
            Ok(pane.scroll.map(|scroll| crate::live::PaneScroll {
                offset_from_bottom: scroll.offset_from_bottom,
                max_offset_from_bottom: scroll.max_offset_from_bottom,
            }))
        }
        _ => Err(missing.into()),
    }
}
/// The agent status a `pane.get` answer carries for the pane.
pub(crate) fn pane_agent_status(value: Value) -> Result<String, String> {
    let missing = "pane response is missing pane";
    match response(value, missing)? {
        res::ResponseResult::PaneInfo { pane } => Ok(pane.agent_status.to_string()),
        _ => Err(missing.into()),
    }
}
pub(crate) fn pane_send_keys_params(pane_id: &str, keys: &[&str]) -> Result<Value, String> {
    params(req::PaneSendKeysParams {
        pane_id: pane_id.into(),
        keys: keys.iter().map(|key| (*key).to_owned()).collect(),
    })
}
pub(crate) fn pane_send_text_params(pane_id: &str, text: &str) -> Result<Value, String> {
    params(req::PaneSendTextParams {
        pane_id: pane_id.into(),
        text: text.into(),
    })
}
/// Moves a pane into a tab of its own inside the workspace it is already in.
///
/// The workspace matters: a delegated child runs in the same working
/// directory as its parent, so a new workspace would split one checkout into
/// two rows describing the same path (PRD D-43). `focus` stays false because
/// the point of the move is that the operator's canvas does not change.
pub(crate) fn pane_move_to_new_tab_params(
    pane_id: &str,
    workspace_id: &str,
    label: &str,
) -> Result<Value, String> {
    params(req::PaneMoveParams {
        pane_id: pane_id.into(),
        destination: req::PaneMoveDestination::NewTab {
            workspace_id: Some(workspace_id.into()),
            label: Some(label.into()),
        },
        focus: false,
    })
}

/// Joins an existing tab without changing focus. The caller verifies that
/// both panes belong to the expected workspace; this destination carries
/// only the tab and pane identifiers in the pinned protocol.
pub(crate) fn pane_join_params(
    pane_id: &str,
    target_tab_id: &str,
    target_pane_id: &str,
) -> Result<Value, String> {
    params(req::PaneMoveParams {
        pane_id: pane_id.into(),
        destination: req::PaneMoveDestination::Tab {
            tab_id: target_tab_id.into(),
            target_pane_id: Some(target_pane_id.into()),
            split: req::SplitDirection::Right,
            ratio: Some(0.5),
        },
        focus: false,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PaneJoinResult {
    pub(crate) changed: bool,
    pub(crate) reason: Option<String>,
    pub(crate) workspace_id: String,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
    pub(crate) terminal_id: String,
    pub(crate) previous_workspace_id: String,
    pub(crate) previous_tab_id: String,
    pub(crate) previous_pane_id: String,
    pub(crate) closed_tab_id: Option<String>,
    pub(crate) closed_workspace_id: Option<String>,
}

/// Keeps the actual resulting identity even for an unchanged move. A
/// `same_tab` answer is not proof of ownership or of an earlier successful
/// join; the caller compares it with its durable claim and observed binding.
pub(crate) fn joined_pane(value: Value) -> Result<PaneJoinResult, String> {
    let missing = "pane.move response is missing joined pane identity";
    match response(value, missing)? {
        res::ResponseResult::PaneMove { move_result } => Ok(PaneJoinResult {
            changed: move_result.changed,
            reason: move_result.reason.map(|reason| reason.to_string()),
            workspace_id: nonempty_id(move_result.pane.workspace_id, missing)?,
            tab_id: nonempty_id(move_result.pane.tab_id, missing)?,
            pane_id: nonempty_id(move_result.pane.pane_id, missing)?,
            terminal_id: nonempty_id(move_result.pane.terminal_id, missing)?,
            previous_workspace_id: nonempty_id(move_result.previous_workspace_id, missing)?,
            previous_tab_id: nonempty_id(move_result.previous_tab_id, missing)?,
            previous_pane_id: nonempty_id(move_result.previous_pane_id, missing)?,
            closed_tab_id: move_result.closed_tab_id,
            closed_workspace_id: move_result.closed_workspace_id,
        }),
        _ => Err(missing.into()),
    }
}

/// The tab `pane.move` created, when it created one.
///
/// Herdr answers a move it declined with `changed: false` and a reason
/// instead of an error, so a refusal has to be read off the result rather
/// than inferred from a missing field.
pub(crate) fn moved_pane_tab(value: Value) -> Result<String, String> {
    let missing = "pane.move response is missing created_tab.tab_id";
    match response(value, missing)? {
        res::ResponseResult::PaneMove { move_result } => move_outcome(
            move_result.changed,
            move_result.reason.map(|reason| format!("{reason:?}")),
            move_result.created_tab.map(|tab| tab.tab_id),
        ),
        _ => Err(missing.into()),
    }
}

/// The decision `pane.move`'s result carries, apart from decoding it.
///
/// Herdr answers a move it declined with `changed: false` and a reason rather
/// than an error, so a refusal has to be read rather than inferred from a
/// missing field.
pub(crate) fn move_outcome(
    changed: bool,
    reason: Option<String>,
    created_tab_id: Option<String>,
) -> Result<String, String> {
    let missing = "pane.move response is missing created_tab.tab_id";
    if !changed {
        return Err(match reason {
            Some(reason) => format!("Herdr declined the move: {reason}"),
            None => "Herdr declined the move".to_owned(),
        });
    }
    nonempty_id(created_tab_id.ok_or_else(|| missing.to_owned())?, missing)
}

pub(crate) fn tab_create_with_env_params(
    workspace: &str,
    cwd: &str,
    label: &str,
    focus: bool,
    env: std::collections::BTreeMap<String, String>,
) -> Result<Value, String> {
    params(req::TabCreateParams {
        workspace_id: Some(workspace.into()),
        cwd: Some(herdr_param(cwd)),
        label: Some(label.into()),
        focus,
        env: env.into_iter().collect(),
    })
}
pub(crate) fn replacement_tab_params(
    workspace: &str,
    cwd: &str,
    env: std::collections::BTreeMap<String, String>,
) -> Result<Value, String> {
    params(req::TabCreateParams {
        workspace_id: Some(workspace.into()),
        cwd: Some(herdr_param(cwd)),
        label: None,
        focus: false,
        env: env.into_iter().collect(),
    })
}
/// The shell a named tab keeps, split beside its closing last pane: no focus,
/// since the close moves the keyboard itself, and the intent marker in `env`.
pub(crate) fn replacement_pane_params(
    pane: &str,
    cwd: &str,
    env: std::collections::BTreeMap<String, String>,
) -> Result<Value, String> {
    params(req::PaneSplitParams {
        target_pane_id: Some(pane.into()),
        direction: req::SplitDirection::Right,
        cwd: Some(herdr_param(cwd)),
        focus: false,
        env: env.into_iter().collect(),
        ratio: None,
        right_click: req::PaneRightClickTarget::Herdr,
        workspace_id: None,
    })
}
pub(crate) fn worktree_create_params(
    cwd: &str,
    branch: &str,
    base: Option<&str>,
    focus: bool,
) -> Result<Value, String> {
    params(req::WorktreeCreateParams {
        base: base.map(str::to_owned),
        branch: Some(branch.to_owned()),
        cwd: Some(herdr_param(cwd)),
        focus,
        label: None,
        // Herdr owns the default checkout location. Hide must never restate it.
        path: None,
        // Keep Herdr's repository trust checks in force.
        trust_repository: None,
        workspace_id: None,
    })
}
pub(crate) fn worktree_list_params(cwd: &str) -> Result<Value, String> {
    params(req::WorktreeListParams {
        cwd: Some(herdr_param(cwd)),
        // Listing must not implicitly trust a repository on the user's behalf.
        trust_repository: None,
        workspace_id: None,
    })
}
pub(crate) fn worktree_remove_params(workspace_id: &str) -> Result<Value, String> {
    params(req::WorktreeRemoveParams {
        force: false,
        // Removal must not implicitly trust a repository on the user's behalf.
        trust_repository: None,
        workspace_id: workspace_id.to_owned(),
    })
}
pub(crate) fn tab_move_params(tab: &str, index: usize) -> Result<Value, String> {
    params(req::TabMoveParams {
        tab_id: tab.into(),
        insert_index: index
            .try_into()
            .map_err(|_| "tab.move insert_index exceeds u32")?,
    })
}
pub(crate) fn pane_split_params(
    pane: &str,
    direction: crate::live::PaneSplitDirection,
    cwd: Option<&str>,
) -> Result<Value, String> {
    params(req::PaneSplitParams {
        target_pane_id: Some(pane.into()),
        direction: match direction {
            crate::live::PaneSplitDirection::Right => req::SplitDirection::Right,
            crate::live::PaneSplitDirection::Down => req::SplitDirection::Down,
        },
        cwd: cwd.filter(|v| !v.trim().is_empty()).map(herdr_param),
        focus: true,
        env: Default::default(),
        ratio: None,
        right_click: req::PaneRightClickTarget::Herdr,
        workspace_id: None,
    })
}

pub(crate) fn pane_split_with_ratio_params(
    pane: &str,
    direction: ClosedSplitDirection,
    cwd: &str,
    ratio: f32,
    env: std::collections::BTreeMap<String, String>,
) -> Result<Value, String> {
    params(req::PaneSplitParams {
        target_pane_id: Some(pane.into()),
        direction: match direction {
            ClosedSplitDirection::Right => req::SplitDirection::Right,
            ClosedSplitDirection::Down => req::SplitDirection::Down,
        },
        cwd: Some(herdr_param(cwd)),
        focus: true,
        env: env.into_iter().collect(),
        ratio: Some(ratio),
        right_click: req::PaneRightClickTarget::Herdr,
        workspace_id: None,
    })
}

pub(crate) fn layout_export_params(tab_id: &str) -> Result<Value, String> {
    params(req::LayoutExportParams {
        pane_id: None,
        tab_id: Some(tab_id.into()),
    })
}

pub(crate) fn layout_apply_params(
    workspace_id: &str,
    tab_id: Option<&str>,
    tab_label: Option<&str>,
    root: &ClosedLayoutNode,
) -> Result<Value, String> {
    params(req::LayoutApplyParams {
        focus: true,
        root: request_layout_node(root),
        tab_id: tab_id.map(str::to_owned),
        tab_label: tab_label.map(str::to_owned),
        // Herdr accepts either a concrete tab to replace or a workspace in
        // which to create a tab. Sending both makes the pinned server reject
        // an otherwise valid restore of workspace.create's seed tab.
        workspace_id: tab_id.is_none().then(|| workspace_id.into()),
    })
}

/// A tab whose one pane runs `command` with `env` in `cwd` (a shell started
/// with Hide's integration), new in `workspace_id`, or in place of the tab
/// `replace` names, which Herdr then closes.
#[allow(clippy::too_many_arguments)] // one layout.apply request: its target, tab and pane
pub(crate) fn launched_tab_params(
    workspace_id: &str,
    replace: Option<&str>,
    label: &str,
    focus: bool,
    cwd: &str,
    env: std::collections::BTreeMap<String, String>,
    command: &[String],
) -> Result<Value, String> {
    params(req::LayoutApplyParams {
        focus,
        root: req::LayoutNode::Pane {
            pane_id: None,
            label: None,
            cwd: Some(herdr_param(cwd)),
            command: Some(command.to_vec()),
            env: env.into_iter().collect(),
        },
        tab_id: replace.map(str::to_owned),
        tab_label: Some(label.into()),
        // Herdr takes either a tab to replace or a workspace to add to.
        workspace_id: replace.is_none().then(|| workspace_id.into()),
    })
}

/// Creates a start's staging tab with a reserved creation claim on its pane.
/// The claim survives layout export; launch environment echo is not required.
/// `seed_tab` may replace only the caller's newly created workspace seed.
pub(crate) fn checkout_start_tab_params(
    workspace: &str,
    seed_tab: Option<&str>,
    cwd: &str,
    label: &str,
    env: std::collections::BTreeMap<String, String>,
    command: Option<&[String]>,
    claim: &str,
) -> Result<Value, String> {
    params(req::LayoutApplyParams {
        focus: false,
        root: req::LayoutNode::Pane {
            pane_id: None,
            label: Some(claim.into()),
            cwd: Some(herdr_param(cwd)),
            command: command.map(<[String]>::to_vec),
            env: env.into_iter().collect(),
        },
        tab_id: seed_tab.map(str::to_owned),
        tab_label: Some(label.into()),
        workspace_id: seed_tab.is_none().then(|| workspace.into()),
    })
}

/// The tab and the one pane `layout.apply` made for
/// [`launched_tab_params`] or [`checkout_start_tab_params`].
pub(crate) fn launched_tab(value: Value) -> Result<(String, String), String> {
    let layout = applied_layout(value)?;
    match layout.root {
        ClosedLayoutNode::Pane {
            pane_id: Some(pane_id),
            ..
        } => Ok((layout.tab_id, pane_id)),
        _ => Err("layout.apply answered no single pane for a launched tab".into()),
    }
}

/// Every agent start Hide makes; a Codex with the shared daemon gets
/// `--no-daemon` here, so no start path can leave it out (PRD
/// overview-request-view D-20).
pub(crate) fn agent_start_params(
    pane_id: &str,
    name: &str,
    kind: &str,
    args: Vec<String>,
    codex_daemon: crate::codex_launch::CodexDaemon,
) -> Result<Value, String> {
    let kind = hide_agent_adapter::canonical_kind(kind);
    params(req::AgentStartParams {
        args: crate::codex_launch::start_arguments(kind, codex_daemon, args)?,
        kind: kind.into(),
        name: name.into(),
        pane_id: pane_id.into(),
        timeout_ms: Some(crate::agent_start::AGENT_START_DEADLINE.as_millis() as u64),
    })
}

/// The session report an agent's own Herdr integration would have sent for
/// `pane_id`: under the source that integration uses, with a sequence that
/// grows with time so a later report from the integration still replaces it.
pub(crate) fn report_agent_session_params(
    pane_id: &str,
    kind: &str,
    session_id: &str,
    seq: u64,
) -> Result<Value, String> {
    let adapter = hide_agent_adapter::adapter(kind)
        .ok_or_else(|| format!("{kind} has no Herdr integration to report as"))?;
    params(req::PaneReportAgentSessionParams {
        pane_id: pane_id.into(),
        source: adapter.herdr.session_source(),
        agent: adapter.herdr.name.into(),
        agent_session_id: Some(session_id.into()),
        agent_session_path: None,
        resume_argv: None,
        seq: Some(seq),
        session_start_source: None,
    })
}

/// The most bytes the pinned Herdr types into the pane's shell for
/// `agent.start` with `params`: the kind's program, then each argument
/// quoted for a POSIX shell (`'…'`, a `'` inside written `'\''`; PowerShell
/// quoting is shorter), separated by spaces, inside bracketed-paste markers
/// when the shell asked for them, then Enter. `FRAMING_BYTES` covers the
/// program name, the markers and the Enter.
pub(crate) fn agent_start_line_bytes(params: &Value) -> usize {
    const FRAMING_BYTES: usize = 64;
    let args = params["args"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    FRAMING_BYTES
        + args
            .iter()
            .filter_map(Value::as_str)
            .map(|arg| 1 + 2 + arg.len() + 3 * arg.matches('\'').count())
            .sum::<usize>()
}

/// A split that creates the pane `Fork agent to the side` runs in. It does not
/// take focus itself: the core moves the keyboard to it, unless the operator
/// chose another view after the press (`Runtime::ingest_fork_pane`).
pub(crate) fn fork_split_params(
    parent_pane_id: &str,
    cwd: Option<&str>,
    env: std::collections::BTreeMap<String, String>,
) -> Result<Value, String> {
    params(req::PaneSplitParams {
        target_pane_id: Some(parent_pane_id.into()),
        direction: req::SplitDirection::Right,
        cwd: cwd.filter(|v| !v.trim().is_empty()).map(herdr_param),
        focus: false,
        env: env.into_iter().collect(),
        ratio: None,
        right_click: req::PaneRightClickTarget::Herdr,
        workspace_id: None,
    })
}

fn request_layout_node(node: &ClosedLayoutNode) -> req::LayoutNode {
    match node {
        ClosedLayoutNode::Pane {
            pane_id,
            label,
            cwd,
            command,
            env,
        } => req::LayoutNode::Pane {
            pane_id: pane_id.clone(),
            label: label.clone(),
            cwd: cwd.as_deref().map(herdr_param),
            command: command.clone(),
            env: env.clone().into_iter().collect(),
        },
        ClosedLayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => req::LayoutNode::Split {
            direction: match direction {
                ClosedSplitDirection::Right => req::SplitDirection::Right,
                ClosedSplitDirection::Down => req::SplitDirection::Down,
            },
            ratio: *ratio,
            first: Box::new(request_layout_node(first)),
            second: Box::new(request_layout_node(second)),
        },
    }
}

fn response_layout_node(node: res::LayoutNode) -> ClosedLayoutNode {
    match node {
        res::LayoutNode::Pane {
            pane_id,
            label,
            cwd,
            command,
            env,
        } => ClosedLayoutNode::Pane {
            pane_id,
            label,
            cwd: cwd.map(herdr_path),
            command,
            env: env.into_iter().collect(),
        },
        res::LayoutNode::Split {
            direction,
            ratio,
            first,
            second,
        } => ClosedLayoutNode::Split {
            direction: match direction {
                res::SplitDirection::Right => ClosedSplitDirection::Right,
                res::SplitDirection::Down => ClosedSplitDirection::Down,
            },
            ratio,
            first: Box::new(response_layout_node(*first)),
            second: Box::new(response_layout_node(*second)),
        },
    }
}

pub(crate) fn exported_layout(value: Value) -> Result<ClosedLayout, String> {
    let missing = "layout.export response is missing layout";
    match response(value, missing)? {
        res::ResponseResult::LayoutExport { layout } => Ok(ClosedLayout {
            workspace_id: layout.workspace_id,
            tab_id: layout.tab_id,
            zoomed: layout.zoomed,
            focused_pane_id: layout.focused_pane_id,
            root: response_layout_node(layout.root),
        }),
        _ => Err(missing.into()),
    }
}

pub(crate) fn applied_layout(value: Value) -> Result<ClosedLayout, String> {
    let missing = "layout.apply response is missing layout";
    match response(value, missing)? {
        res::ResponseResult::LayoutApply { layout } => Ok(ClosedLayout {
            workspace_id: layout.workspace_id,
            tab_id: layout.tab_id,
            zoomed: layout.zoomed,
            focused_pane_id: layout.focused_pane_id,
            root: response_layout_node(layout.root),
        }),
        _ => Err(missing.into()),
    }
}

pub(crate) fn created_workspace(value: Value) -> Result<(String, String, String), String> {
    let missing = "workspace.create response is missing workspace or root pane";
    match response(value, missing)? {
        res::ResponseResult::WorkspaceCreated {
            workspace,
            root_pane,
            tab,
        } => Ok((workspace.workspace_id, tab.tab_id, root_pane.pane_id)),
        _ => Err(missing.into()),
    }
}

pub(crate) fn pane_swap_params(first: &str, second: &str) -> Result<Value, String> {
    params(req::PaneSwapParams {
        direction: None,
        pane_id: None,
        source_pane_id: Some(first.into()),
        target_pane_id: Some(second.into()),
    })
}

/// One agent as Herdr reports it right now, for the check agent sleep makes
/// just before it ends the agent's process.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AgentState {
    pub(crate) agent: Option<String>,
    pub(crate) status: String,
    pub(crate) state_change_seq: u64,
}

pub(crate) fn agent_target_params(target: &str) -> Result<Value, String> {
    params(req::AgentTarget {
        target: target.into(),
    })
}

pub(crate) fn agent_state(value: Value) -> Result<AgentState, String> {
    let missing = "agent.get response is missing agent";
    match response(value, missing)? {
        res::ResponseResult::AgentInfo { agent } => Ok(AgentState {
            agent: agent.agent,
            status: agent.agent_status.to_string(),
            state_change_seq: agent.state_change_seq,
        }),
        _ => Err(missing.into()),
    }
}

/// Fresh delivery guard, converted here rather than exposing generated types.
pub(crate) struct DeliveryAgent {
    pub pane_id: String,
    pub terminal_id: String,
    pub name: String,
    pub kind: Option<String>,
    pub session: Option<String>,
    pub status: String,
    pub state_change_seq: u64,
    pub readiness: Readiness,
}

/// What Herdr says about whether an agent takes typed input. Herdr reports
/// readiness only for an agent it started itself (`agent.start`): it sends
/// `launch_pending: true` while that start settles or is blocked, and
/// `interactive_ready: true` once it is active, and omits either flag when it
/// is false. An agent the operator started by typing its program in a shell
/// carries neither flag, so its readiness is unreported, not refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Readiness {
    Ready,
    Unreported,
    NotReady,
    Launching,
}

impl Readiness {
    pub(crate) fn accepts_input(self) -> bool {
        matches!(self, Self::Ready | Self::Unreported)
    }

    fn of(interactive_ready: Option<bool>, launch_pending: Option<bool>) -> Self {
        match (interactive_ready, launch_pending) {
            (_, Some(true)) => Self::Launching,
            (Some(false), _) => Self::NotReady,
            (Some(true), _) => Self::Ready,
            (None, _) => Self::Unreported,
        }
    }
}

pub(crate) fn delivery_agent(value: Value) -> Result<DeliveryAgent, String> {
    match response(value, "delivery_agent_format")? {
        res::ResponseResult::AgentInfo { agent } => Ok(DeliveryAgent {
            name: agent.name.unwrap_or_else(|| agent.pane_id.clone()),
            pane_id: agent.pane_id,
            terminal_id: agent.terminal_id,
            kind: agent.agent,
            session: agent
                .agent_session
                .and_then(|session| session_digest(&session.value)),
            status: agent.agent_status.to_string(),
            state_change_seq: agent.state_change_seq,
            readiness: Readiness::of(agent.interactive_ready, agent.launch_pending),
        }),
        _ => Err("delivery_agent_format".into()),
    }
}

pub(crate) fn delivery_input_params(pane: &str, text: &str) -> Result<Value, String> {
    params(req::PaneSendInputParams {
        pane_id: pane.into(),
        text: Some(text.into()),
        keys: vec!["enter".into()],
    })
}

/// Which process group holds a pane's terminal, which is its shell's, and
/// the processes in the foreground group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PaneProcessGroup {
    pub(crate) shell_pid: Option<u32>,
    pub(crate) foreground_process_group_id: Option<u32>,
    pub(crate) foreground_pids: Vec<u32>,
    /// The foreground processes' names, in the order of `foreground_pids`.
    pub(crate) foreground_names: Vec<String>,
}

impl PaneProcessGroup {
    /// Herdr's condition for `agent.start`: the foreground group is the
    /// shell's own and holds nothing but the shell.
    pub(crate) fn shell_holds_terminal(&self) -> bool {
        match self.shell_pid {
            Some(shell) if shell > 1 => {
                self.foreground_process_group_id == Some(shell)
                    && self.foreground_pids.iter().all(|pid| *pid == shell)
            }
            _ => false,
        }
    }

    /// The shell's own program name, read while it is in the foreground.
    pub(crate) fn shell_name(&self) -> Option<&str> {
        let shell = self.shell_pid?;
        self.foreground_pids
            .iter()
            .zip(&self.foreground_names)
            .find(|(pid, _)| **pid == shell)
            .map(|(_, name)| name.trim_start_matches('-'))
    }

    /// The first foreground process that is not the pane's shell.
    pub(crate) fn foreground_program(&self) -> Option<&str> {
        self.foreground_pids
            .iter()
            .zip(&self.foreground_names)
            .find(|(pid, _)| Some(**pid) != self.shell_pid)
            .map(|(_, name)| name.as_str())
    }
}

fn process_name(process: &res::PaneProcessInfoProcess) -> String {
    let argv0 = process.argv0.as_deref().unwrap_or_default().trim();
    if argv0.is_empty() {
        process.name.trim().to_owned()
    } else {
        argv0.rsplit('/').next().unwrap_or_default().to_owned()
    }
}

pub(crate) fn tab_rename_params(tab_id: &str, label: &str) -> Result<Value, String> {
    params(req::TabRenameParams {
        tab_id: tab_id.into(),
        label: label.into(),
    })
}

pub(crate) fn pane_process_info_params(pane_id: &str) -> Result<Value, String> {
    params(req::PaneProcessInfoParams {
        pane_id: Some(pane_id.into()),
    })
}

pub(crate) fn foreground_process(value: Value, pane_id: &str) -> Result<Option<String>, String> {
    match response(value, "pane.process_info response is missing process_info")? {
        res::ResponseResult::PaneProcessInfo { process_info, .. }
            if process_info.pane_id == pane_id =>
        {
            let process = process_info
                .foreground_processes
                .iter()
                .find(|process| Some(process.pid) == process_info.foreground_process_group_id)
                .or_else(|| process_info.foreground_processes.last());
            Ok(process.and_then(|process| {
                let argv0 = process.argv0.as_deref().unwrap_or_default().trim();
                let name = if argv0.is_empty() {
                    process.name.trim()
                } else {
                    argv0.rsplit('/').next().unwrap_or_default()
                };
                (!name.is_empty()).then(|| name.to_owned())
            }))
        }
        _ => Err("pane.process_info response does not match requested pane".into()),
    }
}

pub(crate) fn pane_process_group(value: Value) -> Result<PaneProcessGroup, String> {
    let missing = "pane.process_info response is missing process_info";
    match response(value, missing)? {
        res::ResponseResult::PaneProcessInfo { process_info, .. } => Ok(PaneProcessGroup {
            shell_pid: process_info.shell_pid,
            foreground_process_group_id: process_info.foreground_process_group_id,
            foreground_pids: process_info
                .foreground_processes
                .iter()
                .map(|process| process.pid)
                .collect(),
            foreground_names: process_info
                .foreground_processes
                .iter()
                .map(process_name)
                .collect(),
        }),
        _ => Err(missing.into()),
    }
}

/// The terminal of the agent an `agent.start` answer started, for the start
/// settle's record of Hide's own starts (herdrdev/herdr#3853).
pub(crate) fn started_agent_terminal(value: &Value) -> Option<String> {
    match response(
        value.clone(),
        "agent.start response is missing agent terminal",
    )
    .ok()?
    {
        res::ResponseResult::AgentStarted { agent, .. } => {
            Some(agent.terminal_id).filter(|terminal| !terminal.is_empty())
        }
        _ => None,
    }
}

pub(crate) fn started_agent(value: Value) -> Result<String, String> {
    let missing = "agent.start response is missing agent pane";
    match response(value, missing)? {
        res::ResponseResult::AgentStarted { agent, .. } => nonempty_id(agent.pane_id, missing),
        _ => Err(missing.into()),
    }
}
pub(crate) fn pane_resize_params(
    pane: &str,
    direction: crate::live::PaneResizeDirection,
    amount: f32,
) -> Result<Value, String> {
    params(req::PaneResizeParams {
        pane_id: Some(pane.into()),
        direction: match direction {
            crate::live::PaneResizeDirection::Left => req::PaneDirection::Left,
            crate::live::PaneResizeDirection::Right => req::PaneDirection::Right,
            crate::live::PaneResizeDirection::Up => req::PaneDirection::Up,
            crate::live::PaneResizeDirection::Down => req::PaneDirection::Down,
        },
        amount: Some(amount),
    })
}
pub(crate) fn pane_layout_params(pane: &str) -> Result<Value, String> {
    params(req::PaneLayoutParams {
        pane_id: Some(pane.into()),
    })
}
pub(crate) fn pane_zoom_params(pane: &str) -> Result<Value, String> {
    params(req::PaneZoomParams {
        pane_id: Some(pane.into()),
        mode: req::PaneZoomMode::Toggle,
    })
}
pub(crate) fn pane_unzoom_params(pane: &str) -> Result<Value, String> {
    params(req::PaneZoomParams {
        pane_id: Some(pane.into()),
        mode: req::PaneZoomMode::Off,
    })
}
pub(crate) fn pane_read_params(pane: &str, source: &str, lines: u32) -> Result<Value, String> {
    params(req::PaneReadParams {
        pane_id: pane.into(),
        source: source
            .parse()
            .map_err(|e| format!("invalid read source: {e}"))?,
        lines: Some(lines),
        format: req::ReadFormat::Text,
        strip_ansi: true,
    })
}

fn response(value: Value, missing: &str) -> Result<res::ResponseResult, String> {
    serde_json::from_value(value).map_err(|_| missing.to_owned())
}
fn nonempty_id(id: String, missing: &str) -> Result<String, String> {
    if id.trim().is_empty() {
        Err(missing.into())
    } else {
        Ok(id)
    }
}
/// The active tab a `workspace.get` answer names. Herdr keeps a non-focused
/// workspace's active tab as that workspace's memory and emits no event when
/// a close moves it, so this read is how the replica learns the replacement.
pub(crate) fn workspace_active_tab(value: Value) -> Result<String, String> {
    let missing = "workspace.get response is missing workspace.active_tab_id";
    match response(value, missing)? {
        res::ResponseResult::WorkspaceInfo { workspace } => {
            nonempty_id(workspace.active_tab_id, missing)
        }
        _ => Err(missing.into()),
    }
}
pub(crate) fn created_tab(value: Value) -> Result<(String, String), String> {
    let tab_missing = "tab.create response is missing tab.tab_id";
    let pane_missing = "tab.create response is missing root_pane.pane_id";
    // Preserve missing-field priority before the generated record rejects it.
    for (pointer, message) in [
        ("/tab/tab_id", tab_missing),
        ("/root_pane/pane_id", pane_missing),
    ] {
        if !value
            .pointer(pointer)
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty())
        {
            return Err(message.into());
        }
    }
    match response(value, tab_missing)? {
        res::ResponseResult::TabCreated { tab, root_pane } => Ok((tab.tab_id, root_pane.pane_id)),
        _ => Err(tab_missing.into()),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CreatedWorktree {
    pub tab_label: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub path: String,
    pub branch: Option<String>,
}

pub(crate) fn created_worktree(value: Value) -> Result<CreatedWorktree, String> {
    let missing = "worktree.create response is missing its created worktree identity";
    match response(value, missing)? {
        res::ResponseResult::WorktreeCreated {
            workspace,
            tab,
            root_pane,
            worktree,
            ..
        } => Ok(CreatedWorktree {
            tab_label: tab.label,
            workspace_id: nonempty_id(workspace.workspace_id, missing)?,
            tab_id: nonempty_id(tab.tab_id, missing)?,
            pane_id: nonempty_id(root_pane.pane_id, missing)?,
            path: herdr_path(nonempty_id(worktree.path, missing)?),
            branch: worktree.branch,
        }),
        _ => Err(missing.into()),
    }
}

pub(crate) fn listed_worktree_path(value: Value, branch: &str) -> Result<Option<String>, String> {
    let missing = "worktree.list response is malformed";
    match response(value, missing)? {
        res::ResponseResult::WorktreeList { worktrees, .. } => Ok(worktrees
            .into_iter()
            .find(|row| row.branch.as_deref() == Some(branch))
            .map(|row| herdr_path(row.path))),
        _ => Err(missing.into()),
    }
}

pub(crate) fn moved_tabs(value: Value) -> Result<Vec<String>, String> {
    let missing = "tab.move response is missing tabs";
    let id_missing = "tab.move response has a tab without an id";
    let tabs = value.get("tabs").and_then(Value::as_array).ok_or(missing)?;
    if tabs.iter().any(|tab| {
        !tab.get("tab_id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.trim().is_empty())
    }) {
        return Err(id_missing.into());
    }
    match response(value, missing)? {
        res::ResponseResult::TabList { tabs } => {
            Ok(tabs.into_iter().map(|tab| tab.tab_id).collect())
        }
        _ => Err(missing.into()),
    }
}
/// Whether a `pane.zoom` answer changed the zoom. A one-pane tab answers
/// `zoom_changed: false` (`reason: single_pane`) while `changed` can still be
/// true because the zoom moved focus (Herdr 0.9.1), so the zoom bit is read.
pub(crate) fn pane_zoom_changed(value: Value) -> Result<bool, String> {
    let missing = "pane.zoom response is missing zoom";
    match response(value, missing)? {
        res::ResponseResult::PaneZoom { zoom } => Ok(zoom.zoom_changed),
        _ => Err(missing.into()),
    }
}
/// Whether a `pane.resize` answer moved a split.
pub(crate) fn pane_resize_changed(value: Value) -> Result<bool, String> {
    let missing = "pane.resize response is missing resize";
    match response(value, missing)? {
        res::ResponseResult::PaneResize { resize } => Ok(resize.changed),
        _ => Err(missing.into()),
    }
}
/// The value of the pane token `token` in a `pane.get` answer, `None` when
/// the pane carries none.
pub(crate) fn pane_token(value: Value, token: &str) -> Result<Option<String>, String> {
    let missing = "pane.get response is missing pane";
    match response(value, missing)? {
        res::ResponseResult::PaneInfo { pane } => Ok(pane
            .tokens
            .into_iter()
            .find(|(key, _)| String::from(key.clone()) == token)
            .map(|(_, value)| value)),
        _ => Err(missing.into()),
    }
}

/// The terminal a `pane.get` answer runs in its pane. Herdr gives every
/// terminal it starts a new id, a restored one included, while a pane id can
/// be handed out again, so this is what tells a pane from a later one.
pub(crate) fn pane_terminal(value: Value) -> Result<String, String> {
    let missing = "pane.get response is missing pane.terminal_id";
    match response(value, missing)? {
        res::ResponseResult::PaneInfo { pane } => nonempty_id(pane.terminal_id, missing),
        _ => Err(missing.into()),
    }
}
pub(crate) fn split_pane(value: Value) -> Result<String, String> {
    let missing = "pane.split response is missing pane.pane_id";
    match response(value, missing)? {
        res::ResponseResult::PaneInfo { pane } => nonempty_id(pane.pane_id, missing),
        _ => Err(missing.into()),
    }
}
pub(crate) fn pane_layout(value: Value) -> Result<SessionLayoutPayload, String> {
    if value.get("layout").is_none() {
        return Err("pane.layout response is missing layout".into());
    }
    let result: res::ResponseResult = serde_json::from_value(value)
        .map_err(|error| format!("pane.layout response is malformed: {error}"))?;
    match result {
        res::ResponseResult::PaneLayout { layout } => Ok(layout.into()),
        _ => Err("pane.layout response is missing layout".into()),
    }
}
pub(crate) fn pane_text(value: Value) -> Result<crate::live::PaneText, String> {
    if value.get("read").is_none() {
        return Err("pane.read returned no read section".into());
    }
    let result: res::ResponseResult = serde_json::from_value(value)
        .map_err(|error| format!("pane.read response is malformed: {error}"))?;
    match result {
        res::ResponseResult::PaneRead { read } => Ok(crate::live::PaneText {
            text: read.text,
            truncated: read.truncated,
        }),
        _ => Err("pane.read returned no read section".into()),
    }
}
/// The `events.subscribe` parameter object for the given filter kinds, checked
/// against the contract. A kind the pinned schema does not know, or one that
/// needs a pane id, is refused here rather than by Herdr.
pub(crate) fn subscription_params(kinds: &[&str]) -> Result<Value, String> {
    let subscriptions = kinds
        .iter()
        .map(|kind| {
            serde_json::from_value::<req::Subscription>(json!({"type": kind}))
                .map_err(|error| format!("invalid event subscription: {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    params(req::EventsSubscribeParams { subscriptions })
}

// What `hided`'s phone routes and `hide connect`'s server handoff need from
// Herdr: the only items of this module that are public, and the way a crate
// other than this one reaches the generated types. Each takes and returns
// plain values.

/// The recent rows of one pane, with escape sequences kept.
pub fn pane_rows_params(pane_id: &str, lines: u32) -> Result<Value, String> {
    params(req::PaneReadParams {
        pane_id: pane_id.into(),
        source: req::ReadSource::Recent,
        lines: Some(lines),
        format: req::ReadFormat::Ansi,
        strip_ansi: false,
    })
}

/// A `pane.read` answer, as the rows and whether Herdr cut them.
pub fn pane_rows(value: Value) -> Result<(String, bool), String> {
    pane_text(value).map(|read| (read.text, read.truncated))
}

/// A reply typed into a pane and submitted with Enter.
pub fn pane_reply_params(pane_id: &str, text: &str) -> Result<Value, String> {
    delivery_input_params(pane_id, text)
}

/// One Herdr key-combo string sent to a pane.
pub fn pane_key_params(pane_id: &str, key: &str) -> Result<Value, String> {
    params(req::PaneSendInputParams {
        pane_id: pane_id.into(),
        text: None,
        keys: vec![key.into()],
    })
}

/// How a pane's agent session is named: by id, or by the path of its file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneSessionKind {
    Id,
    Path,
}

/// The session Herdr reports for a pane's agent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaneSession {
    pub agent: String,
    pub kind: PaneSessionKind,
    pub value: String,
}

/// A `pane.get` answer, as the pane's folder and its agent session if any.
pub fn pane_session(value: Value) -> Result<(Option<String>, Option<PaneSession>), String> {
    let result: res::ResponseResult = serde_json::from_value(value)
        .map_err(|error| format!("pane.get response is malformed: {error}"))?;
    match result {
        res::ResponseResult::PaneInfo { pane } => Ok((
            pane.cwd,
            pane.agent_session.map(|session| PaneSession {
                agent: session.agent,
                kind: match session.kind {
                    res::AgentSessionRefKind::Id => PaneSessionKind::Id,
                    res::AgentSessionRefKind::Path => PaneSessionKind::Path,
                },
                value: session.value,
            }),
        )),
        _ => Err("pane.get returned no pane".into()),
    }
}

/// What a Herdr server says of itself when pinged.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerPong {
    pub version: String,
    pub protocol: u32,
    /// Whether it can hand its panes to another Herdr program
    /// (`server.live_handoff`); Herdr does so on Unix only.
    pub live_handoff: bool,
}

/// A `ping`, which takes no parameters.
pub fn ping_params() -> Result<Value, String> {
    params(req::PingParams(Default::default()))
}

/// A `ping` answer.
pub fn server_pong(value: Value) -> Result<ServerPong, String> {
    let result: res::ResponseResult = serde_json::from_value(value)
        .map_err(|error| format!("ping response is malformed: {error}"))?;
    match result {
        res::ResponseResult::Pong {
            version,
            protocol,
            capabilities,
        } => Ok(ServerPong {
            version,
            protocol,
            live_handoff: capabilities.is_some_and(|capabilities| capabilities.live_handoff),
        }),
        _ => Err("ping returned no pong".into()),
    }
}

/// A `server.live_handoff` that hands the server's panes to the Herdr at
/// `import_exe`, which refuses them unless it is `version` and speaks the
/// protocol this build pins.
pub fn live_handoff_params(import_exe: &str, version: &str) -> Result<Value, String> {
    params(req::ServerLiveHandoffParams {
        import_exe: Some(import_exe.into()),
        expected_version: Some(version.into()),
        expected_protocol: Some(HERDR_PROTOCOL_REVISION as u32),
    })
}

/// A `server.live_handoff` answer: the replacement holds the panes.
pub fn live_handoff_done(value: Value) -> Result<(), String> {
    let result: res::ResponseResult = serde_json::from_value(value)
        .map_err(|error| format!("server.live_handoff response is malformed: {error}"))?;
    match result {
        res::ResponseResult::Ok => Ok(()),
        _ => Err("server.live_handoff returned something other than ok".into()),
    }
}

pub(crate) fn live_session_response(
    value: Value,
) -> Result<crate::sidebar::SessionSnapshotPayload, SessionFetchError> {
    let snapshot = value
        .get("snapshot")
        .ok_or_else(|| malformed("response is missing snapshot"))?;
    crate::session_sync::project_snapshot(snapshot)
}

fn remote_protocol_error(operation: &str, reason: impl Into<String>) -> crate::remote::RemoteError {
    crate::remote::RemoteError::new(
        operation,
        "herdr",
        crate::remote::RemoteStage::Protocol,
        reason,
        false,
        true,
    )
}

/// Reads the identity envelope of a remote Herdr's snapshot. Herdr's stable
/// snapshot names no host and no sequence, so the caller stamps the host it
/// reached the socket through, and agents are identified by the pane they
/// run in.
pub(crate) fn remote_snapshot(
    value: &Value,
    operation: &str,
    host: &crate::domain::HostScope,
) -> crate::remote::RemoteResult<crate::remote::RemoteSnapshotEnvelope> {
    use crate::remote::{
        REMOTE_PROTOCOL_REVISION, RemoteError, RemoteSnapshotEnvelope, RemoteStage,
    };
    let snapshot = value
        .get("result")
        .and_then(|v| v.get("snapshot"))
        .or_else(|| value.get("snapshot"))
        .ok_or_else(|| {
            RemoteError::new(
                operation,
                "herdr",
                RemoteStage::Herdr,
                "response does not contain result.snapshot",
                true,
                false,
            )
        })?;
    let protocol = snapshot
        .get("protocol")
        .and_then(Value::as_u64)
        .ok_or_else(|| remote_protocol_error(operation, "snapshot.protocol is missing"))?;
    let protocol = u32::try_from(protocol)
        .map_err(|_| remote_protocol_error(operation, "snapshot.protocol exceeds u32"))?;
    if protocol != REMOTE_PROTOCOL_REVISION {
        return Err(remote_protocol_error(
            operation,
            format!("protocol mismatch expected={REMOTE_PROTOCOL_REVISION} received={protocol}"),
        ));
    }
    let snapshot: res::SessionSnapshot = serde_json::from_value(snapshot.clone())
        .map_err(|error| remote_protocol_error(operation, error.to_string()))?;
    let workspace_ids = remote_ids(
        snapshot.workspaces.into_iter().map(|v| v.workspace_id),
        "workspaces",
        "workspace_id",
        operation,
    )?;
    let pane_ids = remote_ids(
        snapshot.panes.into_iter().map(|v| v.pane_id),
        "panes",
        "pane_id",
        operation,
    )?;
    let agent_ids = remote_ids(
        snapshot.agents.into_iter().map(|v| v.pane_id),
        "agents",
        "pane_id",
        operation,
    )?;
    Ok(RemoteSnapshotEnvelope {
        host: host.clone(),
        protocol,
        workspace_ids,
        pane_ids,
        agent_ids,
    })
}
fn remote_ids(
    ids: impl Iterator<Item = String>,
    array: &str,
    key: &str,
    operation: &str,
) -> crate::remote::RemoteResult<Vec<String>> {
    let mut ids = ids.collect::<Vec<_>>();
    if ids.iter().any(String::is_empty) {
        return Err(remote_protocol_error(
            operation,
            format!("missing non-empty field {key}"),
        ));
    }
    ids.sort();
    if ids.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(remote_protocol_error(
            operation,
            format!("{array} contains duplicate {key}"),
        ));
    }
    Ok(ids)
}
#[cfg(test)]
pub(crate) fn checked_response_fixture(id: &Value, result: Value) -> Value {
    let value = json!({"id": id, "result": result});
    let _: res::SuccessResponse = serde_json::from_value(value.clone())
        .expect("fixture must match the pinned generated response");
    value
}

#[cfg(test)]
mod tests {

    #[test]
    fn delivery_readiness_refuses_only_what_herdr_reports_as_not_ready() {
        use super::Readiness;
        // The answer Herdr 0.9.3 gives for an agent it started and that is
        // active: launch_pending=false is omitted.
        let mut response = serde_json::json!({"type":"agent_info", "agent": {
            "pane_id":"w1:p1", "terminal_id":"terminal", "workspace_id":"w1",
            "tab_id":"w1:t1", "focused":false, "revision":1,
            "agent":"codex", "agent_status":"idle", "state_change_seq":1,
            "interactive_ready":true
        }});
        let readiness = |response: &serde_json::Value| {
            super::delivery_agent(response.clone()).unwrap().readiness
        };
        assert_eq!(readiness(&response), Readiness::Ready);
        response["agent"]["launch_pending"] = serde_json::json!(false);
        assert_eq!(readiness(&response), Readiness::Ready);
        // An agent typed into a shell by hand: Herdr reports neither flag.
        response["agent"]
            .as_object_mut()
            .unwrap()
            .remove("interactive_ready");
        assert_eq!(readiness(&response), Readiness::Unreported);
        response["agent"]["interactive_ready"] = serde_json::json!(false);
        assert_eq!(readiness(&response), Readiness::NotReady);
        // A start still settling refuses whatever else is reported.
        for interactive in [serde_json::json!(true), serde_json::json!(false)] {
            response["agent"]["interactive_ready"] = interactive;
            response["agent"]["launch_pending"] = serde_json::json!(true);
            assert_eq!(readiness(&response), Readiness::Launching);
        }
        response["agent"]
            .as_object_mut()
            .unwrap()
            .remove("interactive_ready");
        assert_eq!(readiness(&response), Readiness::Launching);
    }

    #[test]
    fn a_program_holding_the_terminal_is_named_apart_from_the_shell() {
        let group = |foreground: u32, processes: serde_json::Value| {
            super::pane_process_group(json!({"type": "pane_process_info", "process_info": {
                "pane_id": "w1:p1", "shell_pid": 42, "foreground_process_group_id": foreground,
                "foreground_processes": processes
            }}))
            .unwrap()
        };
        let idle = group(42, json!([{"pid": 42, "name": "zsh"}]));
        assert!(idle.shell_holds_terminal());
        let building = group(
            77,
            json!([{"pid": 77, "name": "cargo", "argv0": "/usr/bin/cargo"}, {"pid": 78, "name": "rustc"}]),
        );
        assert!(!building.shell_holds_terminal());
        assert_eq!(building.foreground_program(), Some("cargo"));
        assert_eq!(building.foreground_names, ["cargo", "rustc"]);
    }
    #[test]
    fn process_names_select_the_group_leader_then_last_and_prefer_argv0() {
        let mut value = serde_json::json!({"type":"pane_process_info", "process_info": {
            "pane_id":"w1:p1", "foreground_process_group_id":41085,
            "foreground_processes":[
                {"pid":10130,"name":"caffeinate","argv0":"/usr/bin/caffeinate"},
                {"pid":41085,"name":"2.1.283","argv0":"/usr/local/bin/claude"}
            ], "terminal_title":"never use this"
        }});
        assert_eq!(
            super::foreground_process(value.clone(), "w1:p1")
                .unwrap()
                .as_deref(),
            Some("claude")
        );
        value["process_info"]["foreground_processes"]
            .as_array_mut()
            .unwrap()
            .reverse();
        assert_eq!(
            super::foreground_process(value.clone(), "w1:p1")
                .unwrap()
                .as_deref(),
            Some("claude")
        );
        value["process_info"]["foreground_process_group_id"] = serde_json::json!(999);
        assert_eq!(
            super::foreground_process(value.clone(), "w1:p1")
                .unwrap()
                .as_deref(),
            Some("caffeinate")
        );
        value["process_info"]["foreground_processes"][1]["argv0"] = serde_json::json!("");
        assert_eq!(
            super::foreground_process(value.clone(), "w1:p1")
                .unwrap()
                .as_deref(),
            Some("caffeinate")
        );
        value["process_info"]["foreground_processes"][1]["name"] = serde_json::json!("");
        assert_eq!(super::foreground_process(value, "w1:p1").unwrap(), None);
    }

    #[test]
    fn process_names_use_the_contract_and_never_terminal_titles() {
        let value = serde_json::json!({"type":"pane_process_info", "process_info": {
            "pane_id":"w1:p1", "foreground_processes":[{"pid":12,"name":"cargo"}], "terminal_title":"guess"
        }});
        assert_eq!(
            super::foreground_process(value.clone(), "w1:p1")
                .unwrap()
                .as_deref(),
            Some("cargo")
        );
        assert!(super::foreground_process(value, "w2:p1").is_err());
        let empty = serde_json::json!({"type":"pane_process_info", "process_info":{"pane_id":"w1:p1","foreground_processes":[]}});
        assert_eq!(super::foreground_process(empty, "w1:p1").unwrap(), None);
        assert_eq!(
            super::tab_rename_params("w1:t1", "").unwrap(),
            serde_json::json!({"tab_id":"w1:t1","label":""})
        );
    }

    use super::*;
    use crate::herdr_contract::HERDR_API_SCHEMA_JSON;

    #[test]
    fn pane_interrupt_uses_the_generated_literal_text_contract() {
        assert_eq!(
            pane_send_text_params("w1:p1", "\u{3}").unwrap(),
            json!({"pane_id": "w1:p1", "text": "\u{3}"})
        );
    }

    #[test]
    fn a_fork_splits_without_focus() {
        assert_eq!(
            fork_split_params("w1:p1", Some("/fixture"), Default::default()).unwrap(),
            json!({"target_pane_id": "w1:p1", "direction": "right", "cwd": "/fixture",
                "focus": false, "right_click": "herdr"})
        );
        assert_eq!(
            fork_split_params("w1:p1", Some("  "), Default::default()).unwrap()["cwd"],
            Value::Null
        );
    }

    #[test]
    fn checkout_start_claim_is_created_on_the_pane_without_focus_or_replacement() {
        let claim = "hide-start:0123456789abcdef";
        let env = std::collections::BTreeMap::from([("SHELL_INTEGRATION".into(), "1".into())]);
        let command = vec!["/bin/zsh".into(), "-l".into()];
        let request = checkout_start_tab_params(
            "w1",
            None,
            "/fixture",
            "Terminal",
            env.clone(),
            Some(&command),
            claim,
        )
        .unwrap();
        assert_eq!(
            request,
            json!({
                "workspace_id":"w1", "tab_label":"Terminal", "focus":false,
                "root":{"type":"pane", "label":claim, "cwd":"/fixture",
                    "command":["/bin/zsh", "-l"], "env":{"SHELL_INTEGRATION":"1"}}
            })
        );
        let default_shell =
            checkout_start_tab_params("w1", None, "/fixture", "Terminal", env, None, claim)
                .unwrap();
        assert!(default_shell["root"].get("command").is_none());
        assert_eq!(default_shell["root"]["label"], claim);
        let seed = checkout_start_tab_params(
            "w1",
            Some("w1:t1"),
            "/fixture",
            "Terminal",
            Default::default(),
            None,
            claim,
        )
        .unwrap();
        assert_eq!(seed["tab_id"], "w1:t1");
        assert!(seed.get("workspace_id").is_none());
        assert_eq!(seed["root"]["label"], claim);

        // The pinned server exports the label but does not echo launch env.
        assert_eq!(launched_tab(json!({
            "type":"layout_apply", "layout":{
                "workspace_id":"w1", "tab_id":"w1:t2", "zoomed":false, "focused_pane_id":"w1:p3",
                "root":{"type":"pane", "pane_id":"w1:p3", "label":claim,
                    "cwd":"/fixture", "command":["/bin/zsh", "-l"]}
            }
        })).unwrap(), ("w1:t2".into(), "w1:p3".into()));
    }

    #[test]
    fn pane_join_targets_the_existing_tab_and_unzoom_is_explicit() {
        assert_eq!(
            pane_join_params("w1:p3", "w1:t1", "w1:p1").unwrap(),
            json!({
                "pane_id":"w1:p3", "focus":false,
                "destination":{"type":"tab", "tab_id":"w1:t1",
                    "target_pane_id":"w1:p1", "split":"right", "ratio":0.5}
            })
        );
        assert_eq!(
            pane_unzoom_params("w1:p1").unwrap(),
            json!({"pane_id":"w1:p1", "mode":"off"})
        );
    }

    // Shape captured from the isolated pinned server's successful move.
    // Workstation paths, the opaque claim and terminal identity are replaced.
    fn joined_pane_answer() -> Value {
        json!({"type":"pane_move", "move_result":{
            "changed":true, "previous_pane_id":"w1:p3",
            "previous_workspace_id":"w1", "previous_tab_id":"w1:t2",
            "pane":{"pane_id":"w1:p3", "terminal_id":"term_fixture",
                "workspace_id":"w1", "tab_id":"w1:t1", "focused":false,
                "cwd":"/fixture", "foreground_cwd":"/fixture", "label":"hide-start:fixture",
                "agent_status":"unknown", "tokens":{"hide_prompt":"1"},
                "scroll":{"offset_from_bottom":0, "max_offset_from_bottom":0, "viewport_rows":40},
                "revision":1},
            "target_layout":{"workspace_id":"w1", "tab_id":"w1:t1", "zoomed":false,
                "area":{"x":0,"y":0,"width":120,"height":40}, "focused_pane_id":"w1:p1",
                "panes":[
                    {"pane_id":"w1:p1","focused":true,"rect":{"x":0,"y":0,"width":30,"height":40}},
                    {"pane_id":"w1:p3","focused":false,"rect":{"x":30,"y":0,"width":30,"height":40}},
                    {"pane_id":"w1:p2","focused":false,"rect":{"x":60,"y":0,"width":60,"height":40}}
                ], "splits":[
                    {"id":"split_0_root","direction":"right","ratio":0.5,"rect":{"x":0,"y":0,"width":120,"height":40}},
                    {"id":"split_1_0","direction":"right","ratio":0.5,"rect":{"x":0,"y":0,"width":60,"height":40}}
                ]},
            "closed_tab_id":"w1:t2", "focused_pane_id":"w1:p1"
        }})
    }

    #[test]
    fn pane_join_decodes_actual_identity_and_source_tab_closure() {
        assert_eq!(
            joined_pane(joined_pane_answer()).unwrap(),
            PaneJoinResult {
                changed: true,
                reason: None,
                workspace_id: "w1".into(),
                tab_id: "w1:t1".into(),
                pane_id: "w1:p3".into(),
                terminal_id: "term_fixture".into(),
                previous_workspace_id: "w1".into(),
                previous_tab_id: "w1:t2".into(),
                previous_pane_id: "w1:p3".into(),
                closed_tab_id: Some("w1:t2".into()),
                closed_workspace_id: None,
            }
        );
        let mut missing_identity = joined_pane_answer();
        missing_identity["move_result"]["pane"]["terminal_id"] = json!("");
        assert!(joined_pane(missing_identity).is_err());
        assert!(joined_pane(json!({"type":"ok"})).is_err());
    }

    #[test]
    fn pane_join_preserves_refusal_without_claiming_the_requested_destination() {
        let mut answer = joined_pane_answer();
        let result = &mut answer["move_result"];
        result["changed"] = json!(false);
        result["reason"] = json!("zoomed_tab");
        result["pane"]["tab_id"] = json!("w1:t2");
        result.as_object_mut().unwrap().remove("closed_tab_id");
        // The real refusal supplies both layouts and leaves the pane in its source.
        result["source_layout"] = json!({"workspace_id":"w1", "tab_id":"w1:t2", "zoomed":false,
            "area":{"x":0,"y":0,"width":120,"height":40}, "focused_pane_id":"w1:p3",
            "panes":[{"pane_id":"w1:p3","focused":true,"rect":{"x":0,"y":0,"width":120,"height":40}}],
            "splits":[]});
        result["target_layout"]["zoomed"] = json!(true);
        result["target_layout"]["panes"] = json!([
            {"pane_id":"w1:p1","focused":true,"rect":{"x":0,"y":0,"width":60,"height":40}},
            {"pane_id":"w1:p2","focused":false,"rect":{"x":60,"y":0,"width":60,"height":40}}
        ]);
        result["target_layout"]["splits"]
            .as_array_mut()
            .unwrap()
            .truncate(1);
        let refused = joined_pane(answer.clone()).unwrap();
        assert!(!refused.changed);
        assert_eq!(refused.reason.as_deref(), Some("zoomed_tab"));
        assert_eq!(refused.tab_id, "w1:t2");
        assert_eq!(refused.closed_tab_id, None);

        // Schema-valid altered data proves same_tab remains a distinct unchanged result.
        answer["move_result"]["reason"] = json!("same_tab");
        let unchanged = joined_pane(answer).unwrap();
        assert!(!unchanged.changed);
        assert_eq!(unchanged.reason.as_deref(), Some("same_tab"));
    }

    #[test]
    #[ignore = "requires the owned pinned-server probe output"]
    fn isolated_live_remote_responses_decode_through_generated_types() {
        let directory = std::env::var("HERDR_TEST_TYPED_RESPONSES")
            .expect("check script supplies the owned probe directory");
        let load = |name: &str| -> Value {
            let bytes = std::fs::read(std::path::Path::new(&directory).join(name)).unwrap();
            let _: res::SuccessResponse = serde_json::from_slice(&bytes).unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()["result"].clone()
        };
        assert_eq!(
            created_workspace(load("workspace.create.json")).unwrap().2,
            "w1:p1"
        );
        assert_eq!(
            created_tab(load("tab.create.json")).unwrap(),
            ("w1:t2".into(), "w1:p2".into())
        );
        assert_eq!(
            moved_tabs(load("tab.move.json")).unwrap(),
            ["w1:t2", "w1:t1"]
        );
        assert_eq!(split_pane(load("pane.split.json")).unwrap(), "w1:p3");
        let layout = pane_layout(load("pane.layout.json")).unwrap();
        assert_eq!(layout.panes.len(), 2);
        pane_text(load("pane.read.json")).unwrap();
        let snapshot = load("session.snapshot.json");
        live_session_response(snapshot.clone()).unwrap();
        let remote = remote_snapshot(&snapshot, "probe", &probe_host()).unwrap();
        assert_eq!(remote.pane_ids, ["w1:p1", "w1:p2", "w1:p3"]);
    }

    fn probe_host() -> crate::domain::HostScope {
        crate::domain::HostScope {
            host_id: "probe".to_owned(),
            session_id: "probe".to_owned(),
        }
    }

    #[test]
    fn malformed_remote_snapshots_keep_protocol_priority_and_diagnostic_flags() {
        let value = json!({"snapshot": {"protocol": HERDR_PROTOCOL_REVISION + 1}});
        let error = remote_snapshot(&value, "fixture", &probe_host()).unwrap_err();
        assert_eq!(error.stage(), crate::remote::RemoteStage::Protocol);
        assert!(!error.diagnostic().retryable);
        assert!(error.diagnostic().action_required);
        assert_eq!(
            error.diagnostic().reason,
            format!(
                "protocol mismatch expected={HERDR_PROTOCOL_REVISION} received={}",
                HERDR_PROTOCOL_REVISION + 1
            )
        );
        let mut snapshot = empty_snapshot();
        snapshot.as_object_mut().unwrap().remove("version");
        let error =
            remote_snapshot(&json!({"snapshot":snapshot}), "fixture", &probe_host()).unwrap_err();
        assert_eq!(error.stage(), crate::remote::RemoteStage::Protocol);
        assert!(!error.diagnostic().retryable);
        assert!(error.diagnostic().action_required);
        assert_eq!(error.diagnostic().reason, "missing field `version`");
    }

    #[test]
    fn delete_manual_parameterless_and_terminal_requests_when_schema_declares_them() {
        let schema: Value = serde_json::from_str(HERDR_API_SCHEMA_JSON).unwrap();
        let definitions = schema["schemas"]["request"]["$defs"].as_object().unwrap();
        for name in [
            "SessionSnapshotParams",
            "TerminalInputParams",
            "TerminalScrollParams",
            "TerminalResizeParams",
            "TerminalReleaseParams",
        ] {
            assert!(
                !definitions.contains_key(name),
                "{name} is now generated; delete its manual request builder"
            );
        }
    }

    fn empty_snapshot() -> Value {
        json!({"protocol": HERDR_PROTOCOL_REVISION, "version": "fixture",
            "workspaces": [], "tabs": [], "panes": [], "layouts": [], "agents": []})
    }

    // Herdr's stable event stream has no sequence, so the replica has no cursor
    // and every reconnect reads a fresh snapshot. Should Herdr declare one, the
    // resume path is worth building back: this test names the day.
    #[test]
    fn a_sequenced_event_stream_would_earn_a_resume_cursor() {
        let schema: Value = serde_json::from_str(HERDR_API_SCHEMA_JSON).unwrap();
        let properties = schema["schemas"]["event"]["properties"]
            .as_object()
            .unwrap();
        assert!(
            !properties.contains_key("sequence"),
            "Herdr now sequences its events; resume the subscription from the replica's last sequence instead of re-reading a snapshot on every reconnect"
        );
        let params = schema["schemas"]["request"]["$defs"]["EventsSubscribeParams"]["properties"]
            .as_object()
            .unwrap();
        assert!(
            !params.contains_key("after_sequence"),
            "Herdr now takes a resume cursor; pass the replica's last sequence to events.subscribe"
        );
    }

    #[test]
    fn generated_snapshot_and_response_variants_feed_domain_inputs() {
        let value = empty_snapshot();
        let state = snapshot(value.clone()).unwrap();
        assert!(state.workspaces.is_empty());
        let state =
            snapshot_response(json!({"type": "session_snapshot", "snapshot": value})).unwrap();
        assert!(state.agents.is_empty());
        assert!(
            agents_response(json!({"type": "agent_list", "agents": []}))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn cleanup_preserves_launch_and_foreground_usage_and_unknown_paths() {
        let mut value = empty_snapshot();
        value["panes"] = json!([
            {"pane_id":"w1:p1", "terminal_id":"fixture-terminal", "workspace_id":"w1", "tab_id":"w1:t1", "focused":false, "agent_status":"idle", "revision":1,
             "cwd":"/fixture/main", "foreground_cwd":"/fixture/linked/subdir"},
            {"pane_id":"w1:p2", "terminal_id":"fixture-terminal", "workspace_id":"w1", "tab_id":"w1:t1", "focused":false, "agent_status":"idle", "revision":1}
        ]);
        let paths =
            cleanup_usage_paths(json!({"type":"session_snapshot", "snapshot": value})).unwrap();
        assert_eq!(
            paths,
            vec![
                ("w1:p1".into(), Some("/fixture/main".into())),
                ("w1:p1".into(), Some("/fixture/linked/subdir".into())),
                ("w1:p2".into(), None)
            ]
        );
    }

    #[test]
    fn missing_snapshot_fields_keep_their_diagnostics() {
        for field in crate::session_sync::SNAPSHOT_FIELDS_THE_REPLICA_READS {
            let mut value = empty_snapshot();
            value.as_object_mut().unwrap().remove(field);
            let error = snapshot(value).unwrap_err();
            assert_eq!(error.state(), "malformed");
            assert_eq!(error.message(), format!("snapshot is missing {field}"));
        }
    }

    #[test]
    fn incompatible_protocol_retains_typed_revisions_version_and_safe_remedy() {
        let mut value = empty_snapshot();
        let received = HERDR_PROTOCOL_REVISION + 1;
        value["protocol"] = json!(received);
        let error = snapshot(value.clone()).unwrap_err();
        assert_eq!(error.state(), "protocol_mismatch");
        assert_eq!(
            error.message(),
            format!(
                "The running Herdr uses protocol {received}, but this Hide supports protocol {HERDR_PROTOCOL_REVISION}. Update Hide to a compatible release, then try again. No workspace or agent was created."
            )
        );
        assert_eq!(
            error.protocol_details(),
            Some((HERDR_PROTOCOL_REVISION, received, Some("fixture")))
        );

        let received = HERDR_PROTOCOL_REVISION - 1;
        value["protocol"] = json!(received);
        let error = snapshot(value).unwrap_err();
        assert_eq!(
            error.message(),
            format!(
                "The running Herdr uses protocol {received}, but Hide requires protocol {HERDR_PROTOCOL_REVISION}. When your current work is safe, stop the Herdr session and reopen Hide. Hide will start its compatible bundled Herdr. No workspace or agent was created."
            )
        );
    }

    #[test]
    fn phone_pane_requests_keep_their_shape() {
        assert_eq!(
            pane_rows_params("w1:p1", 40).unwrap(),
            json!({"pane_id": "w1:p1", "source": "recent", "lines": 40, "format": "ansi", "strip_ansi": false})
        );
        assert_eq!(
            pane_reply_params("w1:p1", "ok").unwrap(),
            json!({"pane_id": "w1:p1", "text": "ok", "keys": ["enter"]})
        );
        assert_eq!(
            pane_key_params("w1:p1", "esc").unwrap(),
            json!({"pane_id": "w1:p1", "keys": ["esc"]})
        );
    }

    #[test]
    fn generated_subscriptions_keep_the_filter_shape() {
        assert_eq!(
            subscription_params(&["workspace.created", "pane.focused"]).unwrap(),
            json!({"subscriptions": [{"type": "workspace.created"}, {"type": "pane.focused"}]})
        );
        assert!(subscription_params(&["not.a.subscription"]).is_err());
        assert!(subscription_params(&["pane.agent_status_changed"]).is_err());
    }

    #[test]
    fn generated_event_payloads_feed_replica_events() {
        let value = json!({"event": "workspace_focused", "data": {"type": "workspace_focused", "workspace_id": "w1"}});
        let SubscriptionLine::Event(event) = parse_subscription_line(&value.to_string()).unwrap()
        else {
            panic!("event expected")
        };
        assert!(
            matches!(event, ReplicaEvent::WorkspaceFocused { workspace_id } if workspace_id == "w1")
        );
        let mut wrong = value;
        wrong["data"]["type"] = json!("workspace_closed");
        let error = parse_subscription_line(&wrong.to_string()).err().unwrap();
        assert_eq!(
            error.message(),
            "Herdr workspace_focused event event data type is \"workspace_closed\""
        );
    }

    #[test]
    fn subscription_error_diagnostics_survive_generated_deserialization() {
        let value = json!({"id": "herdr-core:events.subscribe", "error": {"code": "event_gap", "message": "cursor retired"}});
        let SubscriptionLine::Error { code, message } =
            parse_subscription_line(&value.to_string()).unwrap()
        else {
            panic!("error expected")
        };
        assert_eq!(
            (code.as_str(), message.as_str()),
            ("event_gap", "cursor retired")
        );
        for field in ["id", "code", "message"] {
            let mut value = value.clone();
            if field == "id" {
                value.as_object_mut().unwrap().remove(field);
            } else {
                value["error"].as_object_mut().unwrap().remove(field);
            }
            let error = parse_subscription_line(&value.to_string()).err().unwrap();
            assert_eq!(
                error.message(),
                format!("Herdr subscription error is missing {field}")
            );
        }
        assert_eq!(
            parse_subscription_line(" ").err().unwrap().message(),
            "Herdr event stream emitted an empty line"
        );
    }

    fn listed_agent(extra: Value) -> Value {
        let mut agent = json!({"pane_id": "w1:p2", "workspace_id": "w1", "tab_id": "w1:t2",
            "terminal_id": "fixture-terminal", "revision": 1, "focused": false, "agent_status": "working"});
        agent
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"type": "agent_list", "agents": [agent]})
    }

    fn declared_child(session: Option<(&str, &str)>, tokens: Value) -> Value {
        let mut extra = json!({"tokens": tokens});
        if let Some((kind, value)) = session {
            extra["agent_session"] =
                json!({"source": "herdr:claude", "agent": "claude", "kind": kind, "value": value});
        }
        listed_agent(extra)
    }

    fn token_set(child_session: &str, parent_session: &str) -> Value {
        json!({
            "parent_pane": "w1:p1",
            "parent_machine": "machine-parent",
            "child_session": session_digest(child_session).unwrap(),
            "parent_session": session_digest(parent_session).unwrap(),
        })
    }

    #[test]
    fn a_session_is_written_into_a_token_as_its_sha256() {
        // The expected value is `printf s-child | shasum -a 256`, which hcoord's
        // `sessionDigest` reproduces; the two sides agree on this one string.
        assert_eq!(
            session_digest("s-child").as_deref(),
            Some("e91e031561ec0bc9093101da407c3e78b99ebf139047f29f4ac6d5948e30cf0b")
        );
        assert_eq!(session_digest("  "), None);
        assert_eq!(session_digest(""), None);
        assert_ne!(session_digest("s-child"), session_digest("s-other"));
    }

    // sasu starts its implementor with `agent.start` (the only start that can
    // carry the role marker) and declares the Observer's pane as the parent
    // afterwards; without this the row was a root in another checkout and the
    // Observer had no child (2026-09-18).
    #[test]
    fn a_parent_declared_as_a_pane_token_is_the_lineage() {
        let declared = agents_response(declared_child(
            Some(("id", "s-child")),
            token_set("s-child", "s-parent"),
        ))
        .unwrap();
        assert_eq!(declared[0].spawned_from_pane_id.as_deref(), Some("w1:p1"));
        assert_eq!(
            declared[0].spawned_from_machine_id.as_deref(),
            Some("machine-parent")
        );
        assert_eq!(
            declared[0].declared_parent_session,
            session_digest("s-parent"),
            "the parent's session travels on for the pass that can see the parent"
        );
        // The token is still carried verbatim; the sidebar's own token readers are unaffected.
        assert_eq!(declared[0].tokens.get("parent_pane"), Some(&json!("w1:p1")));
        assert_eq!(
            declared[0].tokens.get("parent_machine"),
            Some(&json!("machine-parent"))
        );

        let mut cleared_tokens = token_set("s-child", "s-parent");
        cleared_tokens["parent_pane"] = json!("  ");
        let cleared =
            agents_response(declared_child(Some(("id", "s-child")), cleared_tokens)).unwrap();
        assert_eq!(
            cleared[0].spawned_from_pane_id, None,
            "an empty token is a cleared declaration, not a parent named \"\""
        );

        let silent = agents_response(listed_agent(json!({}))).unwrap();
        assert_eq!(silent[0].spawned_from_pane_id, None);
    }

    #[test]
    fn a_child_whose_pane_now_hosts_another_session_declares_no_parent() {
        let reused = agents_response(declared_child(
            Some(("id", "s-new-agent")),
            token_set("s-child", "s-parent"),
        ))
        .unwrap();
        assert_eq!(reused[0].spawned_from_pane_id, None);
        assert_eq!(reused[0].spawned_from_machine_id, None);
        assert_eq!(reused[0].declared_parent_session, None);
        // The agent itself is still listed, running under its own session.
        assert_eq!(
            reused[0].agent_session.as_ref().unwrap().value,
            "s-new-agent"
        );
    }

    #[test]
    fn a_pane_reported_without_a_session_cannot_prove_the_relationship() {
        let silent_child =
            agents_response(declared_child(None, token_set("s-child", "s-parent"))).unwrap();
        assert_eq!(
            silent_child[0].spawned_from_pane_id, None,
            "no session is no proof, so the pane is drawn as a root while the tokens stay for when it reports one"
        );
        let back = agents_response(declared_child(
            Some(("id", "s-child")),
            token_set("s-child", "s-parent"),
        ))
        .unwrap();
        assert_eq!(back[0].spawned_from_pane_id.as_deref(), Some("w1:p1"));
    }

    #[test]
    fn tokens_written_before_sessions_were_recorded_declare_no_parent() {
        let old = agents_response(declared_child(
            Some(("id", "s-child")),
            json!({"parent_pane": "w1:p1", "parent_machine": "machine-parent"}),
        ))
        .unwrap();
        assert_eq!(
            old[0].spawned_from_pane_id, None,
            "hcoord rewrites the full set within one reconcile interval; until then the child is a root"
        );
    }

    #[test]
    fn a_session_recorded_as_a_path_is_compared_like_an_id() {
        let long_path = format!("/sessions/{}/rollout.jsonl", "d".repeat(120));
        let declared = agents_response(declared_child(
            Some(("path", &long_path)),
            token_set(&long_path, "s-parent"),
        ))
        .unwrap();
        assert_eq!(declared[0].spawned_from_pane_id.as_deref(), Some("w1:p1"));
    }

    #[test]
    fn generated_types_reject_incomplete_records_and_invalid_token_names() {
        let value = json!({"type": "agent_list", "agents": [{"pane_id": "w1:p1"}]});
        assert!(agents_response(value).is_err());
        let value = json!({"type": "agent_list", "agents": [{"pane_id": "w1:p1", "workspace_id": "w1", "tab_id": "w1:t1",
            "terminal_id": "fixture-terminal", "revision": 1, "focused": false, "agent_status": "idle", "tokens": {"invalid key": "value"}}]});
        assert!(agents_response(value).is_err());
    }
}
