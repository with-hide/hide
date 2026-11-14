//! The runtime's side of external tools (PRD hide-external-control): the
//! connection list behind the runtime lock, its file saved off it, the
//! calls hided makes for a tool (request, status, leave, a key's proof and
//! each command's check), the operator's events from a window, and the
//! `access` snapshot section.
//!
//! Every step is one diagnostic record with the connection id, this
//! machine and the reason, never a key or a key file's path (B33).

use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use super::Runtime;
use crate::access::{Access, Call, Ended, Level, Standing, Stored};
use crate::latest_file::{LatestFile, read_capped};
use crate::model::{AccessConnectionSnapshot, AccessPendingSnapshot, AccessSnapshot};

#[derive(Default)]
pub(crate) struct AccessRuntime {
    state: Access,
    store: Option<LatestFile<Stored>>,
    /// Counts what moves the section: the list, the tools' starts.
    edit: u64,
    /// The inputs the section was made from: this edit and the mission
    /// book's, so a snapshot read whose inputs did not move makes nothing.
    composed: Option<(u64, u64)>,
}

/// The connection list's file, read before the runtime lock is taken. A
/// file that cannot be read starts with no tool connected and says why;
/// it is replaced at the next change, which an approve or a disconnect is.
pub(crate) fn restore(path: PathBuf) -> (LatestFile<Stored>, Stored) {
    let stored = match read_capped(&path, crate::access::READ_CAP)
        .and_then(|bytes| bytes.map(|bytes| crate::access::decode(&bytes)).transpose())
    {
        Ok(stored) => stored.unwrap_or_default(),
        Err(message) => {
            crate::diagnostic!(json!({
                "component": "access",
                "kind": "access_store.restore_failed",
                "message": message,
            }));
            Stored::default()
        }
    };
    let file = LatestFile::new(
        path,
        "hide-access-save",
        "access",
        "access_store",
        crate::access::encode,
    );
    (file, stored)
}

/// The operator's answer to a tool, from a window's Settings › External
/// tools (D-31); a pane's scoped connection and the CLI cannot send one
/// (B8).
#[derive(Debug)]
pub(crate) enum AccessEvent {
    Approve { id: String, level: Level },
    Refuse { id: String },
    Level { id: String, level: Level },
    Revoke { id: String },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WithLevel {
    id: String,
    level: Level,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ById {
    id: String,
}

impl AccessEvent {
    pub(super) const KINDS: [&'static str; 4] = [
        "access_approve",
        "access_refuse",
        "access_level",
        "access_revoke",
    ];

    pub(super) fn decode(kind: &str, payload: Value) -> Option<Self> {
        Some(match kind {
            "access_approve" => {
                let WithLevel { id, level } = serde_json::from_value(payload).ok()?;
                Self::Approve { id, level }
            }
            "access_refuse" => Self::Refuse {
                id: serde_json::from_value::<ById>(payload).ok()?.id,
            },
            "access_level" => {
                let WithLevel { id, level } = serde_json::from_value(payload).ok()?;
                Self::Level { id, level }
            }
            "access_revoke" => Self::Revoke {
                id: serde_json::from_value::<ById>(payload).ok()?.id,
            },
            _ => return None,
        })
    }
}

fn standing_answer(standing: &Standing) -> Result<Value, String> {
    match standing {
        Standing::Pending { id, name } => Ok(json!({"state": "pending", "id": id, "name": name})),
        Standing::Connected(connection) => Ok(json!({
            "state": "connected",
            "id": connection.id,
            "name": connection.name,
            "level": connection.level,
        })),
        Standing::Ended(ended) => Err(ended.reason().to_owned()),
        Standing::Unknown => Ok(json!({"state": "unknown"})),
    }
}

impl Runtime {
    pub(crate) fn install_access_store(&mut self, store: LatestFile<Stored>, stored: Stored) {
        self.access.store = Some(store);
        self.access.state = Access::restored(stored);
        self.access.edit += 1;
        self.refresh_access();
    }

    fn note_access(
        &self,
        kind: &str,
        id: Option<&str>,
        level: Option<Level>,
        reason: Option<&str>,
    ) {
        crate::diagnostic!(json!({
            "component": "access",
            "kind": kind,
            "connection_id": id,
            "machine": self.node.as_str(),
            "level": level.map(Level::as_str),
            "reason": reason,
        }));
    }

    /// The list changed: saved off the lock, and the section made again.
    fn access_changed(&mut self, save: bool) {
        if save && let Some(store) = &self.access.store {
            store.save(self.access.state.stored());
        }
        self.access.edit += 1;
        self.refresh_access();
    }

    /// One call hided makes for a tool. Whether the section moved goes
    /// with the answer, so the caller tells the screens.
    pub(crate) fn access_call(&mut self, call: Call) -> (Result<Value, String>, bool) {
        let before = self.access.edit;
        let answer = self.access_answer(call);
        (answer, self.access.edit != before)
    }

    fn access_answer(&mut self, call: Call) -> Result<Value, String> {
        let now = super::unix_milliseconds();
        match call {
            Call::Request {
                name,
                key_sha256,
                program,
            } => match self.access.state.request(&name, &key_sha256, program, now) {
                Ok(requested) => {
                    if let Some(expired) = &requested.expired {
                        self.note_access(
                            "access.request_expired",
                            Some(expired),
                            None,
                            Some("pending_limit"),
                        );
                    }
                    match &requested.standing {
                        Standing::Pending { id, .. } if requested.put_up => {
                            self.note_access("access.requested", Some(id), None, None);
                            self.access_changed(false);
                        }
                        Standing::Connected(connection) => {
                            self.note_access(
                                "access.key_issued",
                                Some(&connection.id),
                                Some(connection.level),
                                None,
                            );
                        }
                        _ => {}
                    }
                    standing_answer(&requested.standing)
                }
                Err(refusal) => {
                    self.note_access("access.request_refused", None, None, Some(refusal.reason()));
                    Err(refusal.reason().to_owned())
                }
            },
            Call::Status { key_sha256 } => {
                standing_answer(&self.access.state.standing(&key_sha256))
            }
            Call::Prove { key_sha256 } => match self.access.state.standing(&key_sha256) {
                Standing::Connected(connection) => Ok(json!({
                    "id": connection.id,
                    "name": connection.name,
                    "level": connection.level,
                })),
                standing => {
                    let reason = match standing {
                        Standing::Pending { .. } => "access_pending",
                        Standing::Ended(ended) => ended.reason(),
                        // A key this core never approved, or one it let go
                        // of: connections outlive a restart, so it is gone.
                        _ => Ended::Revoked.reason(),
                    };
                    self.note_access("access.key_refused", None, None, Some(reason));
                    Err(reason.to_owned())
                }
            },
            Call::Check { id } => {
                let Some(connection) = self.access.state.connection(&id).cloned() else {
                    return Err(Ended::Revoked.reason().to_owned());
                };
                if self.access.state.used(&id, now) {
                    self.access_changed(true);
                }
                Ok(json!({"id": connection.id, "name": connection.name, "level": connection.level}))
            }
            Call::Leave { key_sha256 } => match self.access.state.standing(&key_sha256) {
                Standing::Connected(connection) => {
                    self.access
                        .state
                        .revoke(&connection.id)
                        .map_err(str::to_owned)?;
                    self.note_access("access.left", Some(&connection.id), None, None);
                    self.access_changed(true);
                    Ok(json!({"state": "left", "id": connection.id}))
                }
                Standing::Pending { id, .. } => {
                    self.access.state.refuse(&id).map_err(str::to_owned)?;
                    self.note_access("access.left", Some(&id), None, Some("pending"));
                    self.access_changed(false);
                    Ok(json!({"state": "left", "id": id}))
                }
                Standing::Ended(_) | Standing::Unknown => Ok(json!({"state": "left"})),
            },
            Call::List => {
                self.refresh_access();
                serde_json::to_value(self.snapshot.access.as_deref())
                    .map_err(|_| "access_unavailable".to_owned())
            }
        }
    }

    pub(super) fn access_event(&mut self, event: AccessEvent) -> bool {
        let now = super::unix_milliseconds();
        let outcome = match &event {
            AccessEvent::Approve { id, level } => self
                .access
                .state
                .approve(id, *level, now)
                .map(|_| ("access.approved", id, Some(*level), true)),
            AccessEvent::Refuse { id } => self
                .access
                .state
                .refuse(id)
                .map(|_| ("access.refused", id, None, false)),
            AccessEvent::Level { id, level } => self
                .access
                .state
                .set_level(id, *level)
                .map(|changed| ("access.level", id, Some(*level), changed)),
            AccessEvent::Revoke { id } => self
                .access
                .state
                .revoke(id)
                .map(|_| ("access.revoked", id, None, true)),
        };
        match outcome {
            Ok((kind, id, level, save)) => {
                self.note_access(kind, Some(id), level, None);
                self.access_changed(save);
                true
            }
            Err(reason) => {
                // Two windows may answer one request; the second finds it
                // gone, which nothing on the screen needs to say.
                let id = match &event {
                    AccessEvent::Approve { id, .. }
                    | AccessEvent::Refuse { id }
                    | AccessEvent::Level { id, .. }
                    | AccessEvent::Revoke { id } => id,
                };
                self.note_access("access.event_refused", Some(id), None, Some(reason));
                false
            }
        }
    }

    /// The connection `id` names, while it stands: the level a command of
    /// it runs at, and the identity it acts as.
    pub(crate) fn access_actor(&self, id: &str) -> Option<(Level, crate::delivery::Actor)> {
        self.access.state.connection(id).map(|connection| {
            (
                connection.level,
                crate::delivery::Actor::access(
                    &connection.id,
                    &connection.name,
                    self.node.as_str(),
                ),
            )
        })
    }

    /// The tools' starts moved in the ledger: the section's counts follow.
    pub(super) fn access_starts_moved(&mut self) {
        self.access.edit += 1;
    }

    /// Makes the `access` section what the list, the tools' starts and the
    /// mission book say now, when one of them moved; sent again only when
    /// it differs.
    pub(super) fn refresh_access(&mut self) {
        let inputs = (self.access.edit, self.missions.book_edit);
        if self.access.composed == Some(inputs) {
            return;
        }
        self.access.composed = Some(inputs);
        let started = &self.hide_started.access;
        let missions = &self.missions.book.missions;
        let connections = self
            .access
            .state
            .connections()
            .iter()
            .map(|connection| {
                let agents: Vec<&crate::coordination::started::StartedAgent> = started
                    .iter()
                    .filter(|start| start.connection == connection.id)
                    .map(|start| &start.agent)
                    .collect();
                let conducted: Vec<String> = missions
                    .iter()
                    .filter(|mission| {
                        mission.status.open()
                            && agents.iter().any(|agent| {
                                mission.conductor.actor.pane_id == agent.pane_id
                                    && mission.conductor.actor.session == agent.session
                            })
                    })
                    .map(|mission| mission.id.clone())
                    .collect();
                AccessConnectionSnapshot {
                    id: connection.id.clone(),
                    name: connection.name.clone(),
                    level: connection.level,
                    program: connection.program.clone(),
                    created_ms: connection.created_ms,
                    last_used_ms: connection.last_used_ms,
                    agent_count: agents.len(),
                    agents: agents.iter().map(|agent| agent.agent_id.clone()).collect(),
                    mission_count: conducted.len(),
                    missions: conducted,
                }
            })
            .collect();
        let next = AccessSnapshot {
            machine: self.node.as_str().to_owned(),
            limit: crate::access::MAX_CONNECTIONS,
            pending: self
                .access
                .state
                .pending()
                .iter()
                .map(|pending| AccessPendingSnapshot {
                    id: pending.id.clone(),
                    name: pending.name.clone(),
                    program: pending.program.clone(),
                    requested_ms: pending.requested_ms,
                })
                .collect(),
            connections,
        };
        if self.snapshot.access.as_deref() != Some(&next) {
            self.snapshot.access = Some(Arc::new(next));
        }
    }
}

/// What a tool may not ask even at Act, refused before anything runs: a
/// spawn under a parent (B23: a tool is never a parent, as Desk is not),
/// and what only an agent in a pane can mean, which a tool asks of an
/// agent it starts instead (B22). Any other actor passes untouched.
pub(crate) fn admit_tool_command(
    actor: &crate::delivery::Actor,
    command: &crate::delivery::Command,
) -> Result<(), String> {
    use crate::coordination::Command as Agents;
    if actor.access_connection().is_none() {
        return Ok(());
    }
    let crate::delivery::Command::Agents { command } = command else {
        return match command {
            crate::delivery::Command::Pull { .. } | crate::delivery::Command::Confirm { .. } => {
                Err(NEEDS_AGENT.into())
            }
            _ => Ok(()),
        };
    };
    match command {
        Agents::Spawn {
            parent: Some(_), ..
        } => Err(SPAWN_PARENT.into()),
        Agents::Spawn {
            beside, plan_node, ..
        } if *beside || plan_node.is_some() => Err(NEEDS_AGENT.into()),
        Agents::Show { id } if id == crate::coordination::HERE => Err(NEEDS_AGENT.into()),
        Agents::RegisterSelf { .. }
        | Agents::Register { .. }
        | Agents::AskHuman { .. }
        | Agents::AskHumanClear => Err(NEEDS_AGENT.into()),
        Agents::Spawn { .. } | Agents::List { .. } | Agents::Show { .. } | Agents::End { .. } => {
            Ok(())
        }
    }
}

/// A command only an agent in a Hide pane can mean (B22).
pub(crate) const NEEDS_AGENT: &str = "access_needs_agent";
/// A tool's spawn named a parent (B23).
pub(crate) const SPAWN_PARENT: &str = "access_spawn_parent";

impl Runtime {
    /// The identity a tool's delivery command acts as: its connection's,
    /// on this machine, while the connection stands at Act (B20, B21).
    pub(super) fn access_authority(
        &self,
        device: &str,
        connection: &str,
    ) -> Result<crate::delivery::Actor, String> {
        let (level, actor) = self
            .access_actor(connection)
            .filter(|_| device == self.node.as_str())
            .ok_or(Ended::Revoked.reason())?;
        if level != Level::Act {
            return Err(VIEW_ONLY.into());
        }
        Ok(actor)
    }
}

/// A View only tool asked for what only Act may (B20).
pub(crate) const VIEW_ONLY: &str = "access_view_only";
