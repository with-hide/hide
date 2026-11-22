//! The runtime's side of external tools (PRD hide-external-control): the
//! connection list behind the runtime lock, its file saved off it, the
//! calls hided makes for a tool (request, status, leave, a key's proof and
//! each command's check), the operator's events from a window, and the
//! `access` snapshot section.
//!
//! Every step is one diagnostic record with the connection id, this
//! machine and the reason, never a key or a key file's path (B33).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};

use super::Runtime;
use crate::access::{Access, Call, Ended, Invite, Mode, Standing, Stored};
use crate::latest_file::{LatestFile, read_capped};
use crate::model::{
    AccessConnectionSnapshot, AccessInviteSnapshot, AccessPendingSnapshot, AccessSnapshot,
};

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

/// The operator's answer to a bot, from a window's Settings (D-31); a
/// pane's scoped connection and the CLI cannot send one (B8).
#[derive(Debug)]
pub(crate) enum AccessEvent {
    Approve {
        id: String,
        mode: Mode,
    },
    Refuse {
        id: String,
    },
    Mode {
        id: String,
        mode: Mode,
    },
    Revoke {
        id: String,
    },
    /// A bot the operator made and takes back before a program joined it.
    Cancel {
        id: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WithMode {
    id: String,
    mode: Mode,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ById {
    id: String,
}

impl AccessEvent {
    pub(super) const KINDS: [&'static str; 5] = [
        "access_approve",
        "access_refuse",
        "access_mode",
        "access_revoke",
        "access_invite_cancel",
    ];

    pub(super) fn decode(kind: &str, payload: Value) -> Option<Self> {
        Some(match kind {
            "access_approve" => {
                let WithMode { id, mode } = serde_json::from_value(payload).ok()?;
                Self::Approve { id, mode }
            }
            "access_refuse" => Self::Refuse {
                id: serde_json::from_value::<ById>(payload).ok()?.id,
            },
            "access_mode" => {
                let WithMode { id, mode } = serde_json::from_value(payload).ok()?;
                Self::Mode { id, mode }
            }
            "access_revoke" => Self::Revoke {
                id: serde_json::from_value::<ById>(payload).ok()?.id,
            },
            "access_invite_cancel" => Self::Cancel {
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
            "mode": connection.mode,
        })),
        Standing::Ended(ended) => Err(ended.reason().to_owned()),
        Standing::Unknown => Ok(json!({"state": "unknown"})),
    }
}

/// A made bot as its window reads it, code included: the answer to that
/// window's call alone, never a snapshot.
fn invite_answer(invite: &Invite) -> Value {
    json!({
        "id": invite.id,
        "name": invite.name,
        "mode": invite.mode,
        "code": crate::access::spaced_code(invite.code()),
        "expires_ms": invite.expires_ms,
    })
}

impl Runtime {
    pub(crate) fn install_access_store(&mut self, store: LatestFile<Stored>, stored: Stored) {
        self.access.store = Some(store);
        self.access.state = Access::restored(stored);
        self.access.edit += 1;
        self.refresh_access();
    }

    /// The ids the access list holds, which the delivery store's upkeep
    /// ends every other tool's records, watches and answer waits by
    /// (`coordination::end_gone_tools`, D-23); `None` until the list is
    /// restored at start, so a start ends nothing before it knows the list.
    pub(crate) fn delivery_tool_connections(&self) -> Option<Vec<String>> {
        self.access.store.as_ref()?;
        Some(
            self.access
                .state
                .connections()
                .iter()
                .map(|connection| connection.id.clone())
                .collect(),
        )
    }

    /// The connections the core holds in own mode, whose watches on work
    /// not their own the delivery store's upkeep ends
    /// (`coordination::end_unowned_watches`, mission-87).
    pub(crate) fn delivery_own_connections(&self) -> Vec<String> {
        self.access
            .state
            .connections()
            .iter()
            .filter(|connection| connection.mode == Mode::Own)
            .map(|connection| connection.id.clone())
            .collect()
    }

    fn note_access(&self, kind: &str, id: Option<&str>, mode: Option<Mode>, reason: Option<&str>) {
        crate::diagnostic!(json!({
            "component": "access",
            "kind": kind,
            "connection_id": id,
            "machine": self.node.as_str(),
            "mode": mode.map(Mode::as_str),
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
            Call::Request { id, .. } if !crate::access::valid_id(&id) => {
                Err("invalid_request".to_owned())
            }
            Call::Request {
                name,
                key_sha256,
                program,
                id,
            } => match self
                .access
                .state
                .request(&name, &key_sha256, program, now, id)
            {
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
                                Some(connection.mode),
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
            Call::Join {
                code,
                key_sha256,
                program,
            } => match self.access.state.join(&code, &key_sha256, program, now) {
                Ok(joined) => {
                    if let Standing::Connected(connection) = &joined.standing {
                        let kind = if joined.put_up {
                            "access.joined"
                        } else {
                            "access.key_issued"
                        };
                        self.note_access(kind, Some(&connection.id), Some(connection.mode), None);
                    }
                    if joined.put_up {
                        self.access_changed(true);
                    }
                    standing_answer(&joined.standing)
                }
                Err(reason) => {
                    self.note_access("access.join_refused", None, None, Some(reason));
                    Err(reason.to_owned())
                }
            },
            Call::Invite {
                id,
                code,
                name,
                mode,
            } => match self.access.state.invite(id, code, &name, mode, now) {
                Ok(invite) => {
                    let answer = invite_answer(invite);
                    let id = invite.id.clone();
                    self.note_access("access.invited", Some(&id), Some(mode), None);
                    self.access_changed(false);
                    Ok(answer)
                }
                Err(reason) => {
                    self.note_access("access.invite_refused", None, None, Some(reason));
                    Err(reason.to_owned())
                }
            },
            Call::Renew { id, code } => match self.access.state.renew(&id, code, now) {
                Ok(invite) => {
                    let answer = invite_answer(invite);
                    self.note_access("access.invite_renewed", Some(&id), None, None);
                    self.access_changed(false);
                    Ok(answer)
                }
                Err(reason) => {
                    self.note_access("access.invite_refused", Some(&id), None, Some(reason));
                    Err(reason.to_owned())
                }
            },
            Call::Code { id } => self
                .access
                .state
                .invite_code(&id, now)
                .map(invite_answer)
                .map_err(str::to_owned),
            Call::Status { key_sha256 } => {
                standing_answer(&self.access.state.standing(&key_sha256))
            }
            Call::Prove { key_sha256 } => match self.access.state.standing(&key_sha256) {
                Standing::Connected(connection) => Ok(json!({
                    "id": connection.id,
                    "name": connection.name,
                    "mode": connection.mode,
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
                Ok(json!({"id": connection.id, "name": connection.name, "mode": connection.mode}))
            }
            Call::Leave { key_sha256 } => match self.access.state.standing(&key_sha256) {
                Standing::Connected(connection) => {
                    self.access
                        .state
                        .revoke(&connection.id)
                        .map_err(str::to_owned)?;
                    self.note_access("access.left", Some(&connection.id), None, None);
                    self.access_changed(true);
                    self.tool_work_moved(&connection.id);
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
            AccessEvent::Approve { id, mode } => self
                .access
                .state
                .approve(id, *mode, now)
                .map(|_| ("access.approved", id, Some(*mode), true)),
            AccessEvent::Refuse { id } => self
                .access
                .state
                .refuse(id)
                .map(|_| ("access.refused", id, None, false)),
            AccessEvent::Mode { id, mode } => self
                .access
                .state
                .set_mode(id, *mode)
                .map(|changed| ("access.mode", id, Some(*mode), changed)),
            AccessEvent::Revoke { id } => self
                .access
                .state
                .revoke(id)
                .map(|_| ("access.revoked", id, None, true)),
            AccessEvent::Cancel { id } => self
                .access
                .state
                .cancel(id)
                .map(|_| ("access.invite_cancelled", id, None, false)),
        };
        match outcome {
            Ok((kind, id, mode, save)) => {
                self.note_access(kind, Some(id), mode, None);
                self.access_changed(save);
                // Who keeps the bot's work follows the event that moved it:
                // a bot of the whole Hide works as the operator, so all mode
                // or a disconnect hands it to the operator at once, and own
                // mode takes it back (mission-87).
                if matches!(event, AccessEvent::Mode { .. } | AccessEvent::Revoke { .. }) {
                    let id = id.clone();
                    self.tool_work_moved(&id);
                }
                true
            }
            Err(reason) => {
                // Two windows may answer one request; the second finds it
                // gone, which nothing on the screen needs to say.
                let id = match &event {
                    AccessEvent::Approve { id, .. }
                    | AccessEvent::Refuse { id }
                    | AccessEvent::Mode { id, .. }
                    | AccessEvent::Revoke { id }
                    | AccessEvent::Cancel { id } => id,
                };
                self.note_access("access.event_refused", Some(id), None, Some(reason));
                false
            }
        }
    }

    /// The connection `id` names, while it stands: the mode a command of
    /// it runs in, and the identity it acts as.
    pub(crate) fn access_actor(&self, id: &str) -> Option<(Mode, crate::delivery::Actor)> {
        self.access.state.connection(id).map(|connection| {
            (
                connection.mode,
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

    /// Whether the bot of `connection` keeps its work now: the connection
    /// stands in own mode (PRD hide-external-control D-14, mission-87). A
    /// bot of the whole Hide works as the operator, so what it starts is the
    /// operator's and only names it as its starter; a disconnected or a left
    /// connection keeps none, and a new connection of the same name is
    /// another id, so it takes none of this (D-40).
    pub(crate) fn access_keeps_work(&self, connection: &str) -> bool {
        self.access
            .state
            .connection(connection)
            .is_some_and(|connection| connection.mode == Mode::Own)
    }

    /// The external tool `work` is, as a row or a mission names it: whether
    /// the tool keeps it is read from its connection now (D-14, D-40).
    pub(crate) fn tool_work_snapshot(
        &self,
        work: &crate::coordination::started::ToolWork,
    ) -> crate::model::ToolWorkSnapshot {
        crate::model::ToolWorkSnapshot {
            connection: work.connection.clone(),
            name: work.name.clone(),
            owns: self.access_keeps_work(&work.connection),
        }
    }

    /// The external tool whose work `mission`'s Conductor is, by its
    /// registration (or, for a record that names none, by its pane and
    /// session) (PRD hide-external-control B30, D-36). While the tool keeps
    /// it (`owns`) the mission is in the tool's cell and off the operator's
    /// mission screen, counts, asks and notices; once the tool lets go it is
    /// the operator's and still names the tool. Hide AI's and the
    /// operator's missions name none (B37).
    pub(crate) fn mission_tool(
        &self,
        mission: &crate::missions::Mission,
    ) -> Option<crate::model::ToolWorkSnapshot> {
        self.conductor_tool(&mission.conductor)
            .map(|work| self.tool_work_snapshot(work))
    }

    /// The bot's work `conductor` is, by its registration or, for a record
    /// that names none, by its pane and session.
    fn conductor_tool(
        &self,
        conductor: &crate::missions::Conductor,
    ) -> Option<&crate::coordination::started::ToolWork> {
        self.hide_started
            .tool_work
            .iter()
            .find(|work| match conductor.agent_id.as_deref() {
                Some(id) => work.agent_id == id,
                None => {
                    work.pane_id == conductor.actor.pane_id
                        && work.session.is_some()
                        && work.session == conductor.actor.session
                }
            })
    }

    /// Who keeps connection `id`'s work moved (a mode, a disconnect, a
    /// leave): the rows and their lists are judged again on this event, so
    /// the work is in or out of the operator's lists at once (B16). Nothing
    /// is read for a tool that started nothing.
    fn tool_work_moved(&mut self, id: &str) -> bool {
        if !self
            .hide_started
            .tool_work
            .iter()
            .any(|work| work.connection == id)
        {
            return false;
        }
        let lineage = self.refresh_agent_lineage();
        let agents = self.snapshot.navigator.agents.clone();
        lineage
            | self.sync_pane_status_from_agents(&agents)
            | self.sync_session_state()
            | self.refresh_agent_scopes()
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
                let conducted = missions
                    .iter()
                    .filter(|mission| {
                        mission.status.open()
                            && agents.iter().any(|agent| {
                                mission.conductor.actor.pane_id == agent.pane_id
                                    && mission.conductor.actor.session == agent.session
                            })
                    })
                    .count();
                AccessConnectionSnapshot {
                    id: connection.id.clone(),
                    name: connection.name.clone(),
                    mode: connection.mode,
                    program: connection.program.clone(),
                    created_ms: connection.created_ms,
                    last_used_ms: connection.last_used_ms,
                    agent_count: agents.len(),
                    mission_count: conducted,
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
            invites: self
                .access
                .state
                .invites()
                .iter()
                .map(|invite| AccessInviteSnapshot {
                    id: invite.id.clone(),
                    name: invite.name.clone(),
                    mode: invite.mode,
                    created_ms: invite.created_ms,
                    expires_ms: invite.expires_ms,
                })
                .collect(),
            connections,
        };
        if self.snapshot.access.as_deref() != Some(&next) {
            self.snapshot.access = Some(Arc::new(next));
        }
    }
}

impl Runtime {
    /// The identity a bot's command acts as: its connection's, on this
    /// machine, while the connection's mode allows `need` (B20-B23,
    /// `crate::access::admit`), with the mode the command's own reads hold
    /// it to (D3).
    pub(super) fn access_authority(
        &self,
        device: &str,
        connection: &str,
        need: crate::access::Need,
    ) -> Result<(crate::delivery::Actor, Mode), String> {
        let (mode, actor) = self
            .access_actor(connection)
            .filter(|_| device == self.node.as_str())
            .ok_or(Ended::Revoked.reason())?;
        crate::access::admit(mode, need)?;
        Ok((actor, mode))
    }
}

/// What an own-mode bot's command reaches, judged against its own work
/// (mission-87): the agents its connection started and every agent started
/// under them, as the started records hold them (`hide_started.tool_work`,
/// the set that keeps them out of the operator's lists), and the missions
/// those agents conduct. What none of them is, is not its own.
impl Runtime {
    /// The own work an own-mode bot's command is held to: `None` for a
    /// caller that is no bot, or a bot of the whole Hide.
    pub(super) fn own_work(&self, caller: &str) -> Option<OwnWork> {
        match crate::workspace_control::Caller::parse(caller) {
            crate::workspace_control::Caller::Access { connection, .. }
                if self
                    .access
                    .state
                    .connection(connection)
                    .is_some_and(|connection| connection.mode == Mode::Own) =>
            {
                Some(self.own_work_of(connection))
            }
            _ => None,
        }
    }

    /// Connection `id`'s own work now, read from the ledger the core holds:
    /// the registrations `hide_started.tool_work` gives it, the spawns it or
    /// they admitted, and the panes those run in now.
    pub(super) fn own_work_of(&self, id: &str) -> OwnWork {
        let agents: HashSet<String> = self
            .hide_started
            .tool_work
            .iter()
            .filter(|work| work.connection == id)
            .map(|work| work.agent_id.clone())
            .collect();
        let mut own = OwnWork {
            connection: id.to_owned(),
            agents,
            origins: HashSet::new(),
            spawns: HashSet::new(),
            panes: Vec::new(),
        };
        let Ok(ledger) = self.delivery_ledger.as_ref() else {
            return own;
        };
        own.origins = ledger
            .agents
            .iter()
            .filter(|record| record.actor.access_connection() == Some(id))
            .map(|record| record.id.clone())
            .collect();
        for record in &ledger.agents {
            if !record.ended && own.agents.contains(&record.id) {
                own.panes.push(record.actor.pane_id.clone());
            }
        }
        for spawn in &ledger.spawns {
            if !own.origins.contains(&spawn.parent) && !own.agents.contains(&spawn.parent) {
                continue;
            }
            own.spawns.insert(spawn.id.clone());
            // A spawn still starting has no registration yet: its pane is
            // the bot's while the pane runs the terminal the spawn recorded,
            // never a pane that took the id over later.
            if spawn.child.is_none()
                && let (Some(pane), Some(terminal)) = (self.spawn_pane_id(spawn), &spawn.terminal)
                && self.pane_runs(&pane, terminal)
            {
                own.panes.push(pane);
            }
        }
        own
    }

    /// Whether the pane the core names `pane` runs terminal `terminal` now.
    fn pane_runs(&self, pane: &str, terminal: &str) -> bool {
        let node = self.node.as_str();
        self.snapshot
            .navigator
            .workspaces
            .iter()
            .flat_map(|workspace| &workspace.checkouts)
            .flat_map(|checkout| &checkout.tabs)
            .flat_map(|tab| &tab.panes)
            .any(|row| {
                crate::address::same_pane(node, node, &row.id, pane)
                    && row.terminal_id.as_deref() == Some(terminal)
            })
    }

    /// The agent an own-mode bot's command names, read once by the one
    /// resolver every command shares (`Runtime::resolve_target`), held to
    /// `own`, and answered as the exact id the command then runs on: a
    /// registration's, or a spawn's that has none yet (mission-87). A key
    /// that names nothing is refused as one that is not its own, so a bot
    /// cannot tell the two apart.
    pub(super) fn own_target(
        &self,
        own: &OwnWork,
        device: &str,
        key: &str,
    ) -> Result<String, String> {
        let bot = self.access_actor(&own.connection).map(|(_, actor)| actor);
        match self.resolve_target(bot.as_ref(), device, key)? {
            Some(super::delivery::Target::Agent(record)) if own.agents.contains(&record.id) => {
                Ok(record.id.clone())
            }
            Some(super::delivery::Target::Spawn(spawn)) if own.spawns.contains(&spawn.id) => {
                Ok(spawn.id.clone())
            }
            _ => Err(crate::access::NOT_OWN_WORK.to_owned()),
        }
    }

    /// Whether the pane `key` names, as a pane command reads it on `device`,
    /// is one `own` runs in now: a live registration's of its own, or its
    /// own spawn's that is still starting there.
    pub(super) fn own_pane(&self, own: &OwnWork, device: &str, key: &str) -> bool {
        let node = self.node.as_str();
        let pane = crate::address::named_pane_id(node, device, key);
        own.panes
            .iter()
            .any(|own| crate::address::same_pane(node, node, own, &pane))
    }

    /// Whether `conductor` conducts as `own`'s work.
    pub(super) fn conducts_own(
        &self,
        own: &OwnWork,
        conductor: &crate::missions::Conductor,
    ) -> bool {
        self.conductor_tool(conductor)
            .is_some_and(|work| work.connection == own.connection)
    }
}

/// An own-mode bot's work, by the exact identities its commands are held
/// to.
pub(super) struct OwnWork {
    pub connection: String,
    /// The registrations it started and those started under them, ended or
    /// not, by id.
    pub agents: HashSet<String>,
    /// Its own origin registrations, which its spawns name as their parent.
    pub origins: HashSet<String>,
    /// The spawns it or its agents admitted, by id.
    pub spawns: HashSet<String>,
    /// The panes its work runs in now, by the id the core names them with.
    pub panes: Vec<String>,
}

/// Holds what an own-mode bot names to its own work.
pub(super) fn hold(own: bool) -> Result<(), String> {
    if own {
        Ok(())
    } else {
        Err(crate::access::NOT_OWN_WORK.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::{Value, json};

    use super::AccessEvent;
    use crate::access::Mode;
    use crate::model::{
        AccessConnectionSnapshot, AccessInviteSnapshot, AccessPendingSnapshot, AccessSnapshot,
    };

    fn schema() -> Value {
        serde_json::from_str(include_str!("../../../contracts/hided-ws.schema.json")).unwrap()
    }

    fn keys(value: &Value) -> BTreeSet<String> {
        value.as_object().unwrap().keys().cloned().collect()
    }

    /// Whether `value` has every key `def` requires and none it does not
    /// name, as `additionalProperties: false` says.
    fn fits(def: &Value, value: &Value) -> bool {
        let named = keys(&def["properties"]);
        let required: BTreeSet<String> = def["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|key| key.as_str().unwrap().to_owned())
            .collect();
        let have = keys(value);
        required.is_subset(&have) && have.is_subset(&named)
    }

    fn mode_fits(defs: &Value, mode: &Value) -> bool {
        defs["accessMode"]["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|shape| {
                fits(shape, mode) && shape["properties"]["kind"]["const"] == mode["kind"]
            })
            .count()
            == 1
    }

    #[test]
    fn the_access_section_and_its_events_are_the_shapes_the_wire_contract_names() {
        let schema = schema();
        let defs = &schema["$defs"];
        let section = serde_json::to_value(AccessSnapshot {
            machine: "this-mac".into(),
            limit: crate::access::MAX_CONNECTIONS,
            pending: vec![AccessPendingSnapshot {
                id: "00000000000000a1".into(),
                name: "orca".into(),
                program: None,
                requested_ms: 1,
            }],
            invites: vec![AccessInviteSnapshot {
                id: "00000000000000c1".into(),
                name: "hermes".into(),
                mode: Mode::Own,
                created_ms: 1,
                expires_ms: 600_001,
            }],
            connections: [Mode::All, Mode::Own]
                .into_iter()
                .enumerate()
                .map(|(n, mode)| AccessConnectionSnapshot {
                    id: format!("00000000000000b{n}"),
                    name: format!("bot-{n}"),
                    mode,
                    program: Some("python3".into()),
                    created_ms: 1,
                    last_used_ms: 2,
                    agent_count: 0,
                    mission_count: 0,
                })
                .collect(),
        })
        .unwrap();
        assert!(fits(&defs["accessSnapshot"], &section), "{section}");
        assert!(fits(&defs["accessPending"], &section["pending"][0]));
        assert!(fits(&defs["accessInvite"], &section["invites"][0]));
        assert!(mode_fits(defs, &section["invites"][0]["mode"]));
        for connection in section["connections"].as_array().unwrap() {
            assert!(fits(&defs["accessConnection"], connection), "{connection}");
            assert!(mode_fits(defs, &connection["mode"]), "{connection}");
        }

        // Each event the contract declares decodes from its payload, and a
        // level or a workspace, earlier wires, decodes into nothing.
        for mode in [json!({"kind": "all"}), json!({"kind": "own"})] {
            let payload = json!({"id": "00000000000000a1", "mode": mode});
            assert!(fits(&defs["accessApprovePayload"], &payload));
            assert!(fits(&defs["accessModePayload"], &payload));
            assert!(matches!(
                AccessEvent::decode("access_approve", payload.clone()),
                Some(AccessEvent::Approve { .. })
            ));
            assert!(matches!(
                AccessEvent::decode("access_mode", payload),
                Some(AccessEvent::Mode { .. })
            ));
        }
        let by_id = json!({"id": "00000000000000a1"});
        assert!(fits(&defs["accessIdPayload"], &by_id));
        assert!(AccessEvent::decode("access_refuse", by_id.clone()).is_some());
        assert!(AccessEvent::decode("access_revoke", by_id.clone()).is_some());
        assert!(matches!(
            AccessEvent::decode("access_invite_cancel", by_id),
            Some(AccessEvent::Cancel { .. })
        ));
        for refused in [
            ("access_approve", json!({"id": "a", "level": "act"})),
            ("access_level", json!({"id": "a", "level": "act"})),
            (
                "access_mode",
                json!({"id": "a", "mode": {"kind": "workspace"}}),
            ),
        ] {
            assert!(
                AccessEvent::decode(refused.0, refused.1.clone()).is_none(),
                "{refused:?}"
            );
        }
        // The starter a row and a mission carry names the bot either way.
        let starter = serde_json::to_value(crate::model::ToolWorkSnapshot {
            connection: "00000000000000b1".into(),
            name: "bot-1".into(),
            owns: false,
        })
        .unwrap();
        assert!(fits(&defs["botStarter"], &starter), "{starter}");
        assert_eq!(
            AccessEvent::KINDS,
            [
                "access_approve",
                "access_refuse",
                "access_mode",
                "access_revoke",
                "access_invite_cancel"
            ]
        );
    }
}
