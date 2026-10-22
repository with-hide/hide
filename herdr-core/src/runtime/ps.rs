//! `hide ps`: every live pane by project, checkout and tab, the agent each
//! holds, and the registered agents no pane holds. It reads what the core
//! already publishes (the navigator, its agent rows, the mission map) and
//! the delivery ledger, and asks Herdr nothing. Only owned rows are taken
//! under the lock (`Runtime::ps_rows`); the ledger join runs after it
//! (`PsRows::answer`).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::delivery::answer::{Ps, PsAgent, PsMission, PsPane, PsState};
use crate::delivery::ledger::Ledger;
use crate::missions::MissionStatus;
use crate::model::SidebarAgentSnapshot;

use super::Runtime;

/// What `hide ps` takes from the runtime under its lock.
pub(crate) struct PsRows {
    panes: Vec<PsPane>,
    /// Agent rows whose pane is in no tab: a sleeping agent's, or one whose
    /// pane the navigator has not placed.
    loose: Vec<PsAgent>,
    /// The lineage session each pane's agent runs, which tells its
    /// registration from an older one on the same pane.
    sessions: HashMap<(String, String), String>,
    disconnected: HashSet<String>,
    /// Open missions' workers, by registration id.
    workers: HashMap<String, PsMission>,
    ledger: Arc<Ledger>,
}

/// A pane id as the navigator spells it, as its machine and Herdr's own id.
fn split_pane(node: &str, scoped: &str) -> (String, String) {
    scoped
        .strip_prefix("remote:")
        .and_then(|rest| rest.split_once(":pane:"))
        .map_or_else(
            || (node.to_owned(), scoped.to_owned()),
            |(machine, pane)| (machine.to_owned(), pane.to_owned()),
        )
}

fn open(status: MissionStatus) -> bool {
    !matches!(status, MissionStatus::Done | MissionStatus::Cancelled)
}

impl Runtime {
    /// The rows `hide ps` answers from, for a caller on `device` in `pane`.
    pub(crate) fn ps_rows(&self, device: &str, pane: &str) -> Result<PsRows, String> {
        let ledger = self.delivery_state()?;
        let node = self.node.as_str();
        let navigator = &self.snapshot.navigator;
        let disconnected: HashSet<String> = self
            .snapshot
            .status
            .remote
            .iter()
            .filter(|remote| remote.state != "connected")
            .map(|remote| remote.target_id.clone())
            .collect();
        let you = split_pane(device, pane);
        let mut conductors = HashMap::new();
        let mut workers = HashMap::new();
        for mission in self
            .snapshot
            .missions
            .items
            .iter()
            .filter(|mission| open(mission.status))
        {
            let role = |role: &str| PsMission {
                id: mission.id.clone(),
                title: mission.title.clone(),
                role: role.to_owned(),
            };
            conductors.insert(
                split_pane(&mission.device_id, &mission.conductor.pane_id),
                role("conductor"),
            );
            for node in &mission.nodes {
                if let Some(agent) = &node.agent_id {
                    workers.insert(agent.clone(), role(&node.key));
                }
            }
        }
        let row = |row: &SidebarAgentSnapshot, machine: &str, pane: &str| PsAgent {
            id: None,
            name: row.herdr_name.clone(),
            kind: row.agent_kind.clone(),
            state: if disconnected.contains(machine) {
                PsState::Disconnected
            } else {
                row.mark.into()
            },
            label: Some(row.identity_label.clone()).filter(|label| !label.is_empty()),
            line: row.detail.clone(),
            machine: machine.to_owned(),
            pane: pane.to_owned(),
            parent_machine: None,
            parent_pane: None,
            parent: None,
            watched_by: None,
            mission: conductors
                .get(&(machine.to_owned(), pane.to_owned()))
                .cloned(),
            project: None,
        };
        // This machine's projects and agent rows are the navigator's; each
        // device's are its remote session's, kept while it does not answer.
        let label = |device: &str| {
            navigator
                .devices
                .iter()
                .find(|row| row.id == device)
                .and_then(|row| row.label.clone())
                .unwrap_or_else(|| device.to_owned())
        };
        let sources: Vec<_> = std::iter::once((None, &navigator.workspaces, &navigator.agents))
            .chain(self.snapshot.status.remote.iter().filter_map(|remote| {
                let session = remote.session.as_ref()?;
                Some((
                    Some(label(&remote.target_id)),
                    &session.workspaces,
                    &session.agents,
                ))
            }))
            .collect();
        let mut panes = Vec::new();
        let mut loose = Vec::new();
        let mut sessions = HashMap::new();
        for (machine_label, workspaces, agents) in sources {
            let rows: HashMap<&str, &SidebarAgentSnapshot> = agents
                .iter()
                .map(|agent| (agent.pane_id.as_str(), agent))
                .collect();
            let mut placed = HashSet::new();
            for workspace in workspaces {
                for checkout in &workspace.checkouts {
                    for tab in &checkout.tabs {
                        for pane in &tab.panes {
                            let (machine, raw) = split_pane(node, &pane.id);
                            let agent = rows.get(pane.id.as_str()).map(|agent| {
                                placed.insert(pane.id.as_str());
                                let mut found = row(agent, &machine, &raw);
                                if let Some(parent) = &agent.lineage_parent_pane_id {
                                    let (machine, pane) = split_pane(node, parent);
                                    found.parent_machine = Some(machine);
                                    found.parent_pane = Some(pane);
                                }
                                found
                            });
                            panes.push(PsPane {
                                you: (machine.as_str(), raw.as_str())
                                    == (you.0.as_str(), you.1.as_str()),
                                machine,
                                machine_label: machine_label.clone(),
                                project: workspace.label.clone(),
                                project_path: workspace.path.clone(),
                                checkout: checkout
                                    .branch
                                    .clone()
                                    .unwrap_or_else(|| checkout.label.clone()),
                                checkout_path: checkout.path.clone(),
                                worktree: checkout.is_worktree,
                                tab: tab.label.clone(),
                                tab_id: tab.id.clone(),
                                pane: raw,
                                title: agent
                                    .is_none()
                                    .then(|| {
                                        pane.herdr_label
                                            .clone()
                                            .or_else(|| pane.terminal_title.clone())
                                    })
                                    .flatten(),
                                agent,
                            });
                        }
                    }
                }
            }
            for agent in agents {
                let key = split_pane(node, &agent.pane_id);
                if let Some(session) = &agent.lineage_session {
                    sessions.insert(key.clone(), session.clone());
                }
                if !placed.contains(agent.pane_id.as_str()) {
                    loose.push(row(agent, &key.0, &key.1));
                }
            }
        }
        Ok(PsRows {
            panes,
            loose,
            sessions,
            disconnected,
            workers,
            ledger,
        })
    }
}

impl PsRows {
    /// The answer, joined with the delivery ledger off the runtime's lock;
    /// `all` adds the ended registrations.
    pub(crate) fn answer(self, all: bool) -> Ps {
        let Self {
            mut panes,
            loose,
            sessions,
            disconnected,
            workers,
            ledger,
        } = self;
        let mut used = HashSet::new();
        let mut join = |agent: &mut PsAgent| {
            let key = (agent.machine.clone(), agent.pane.clone());
            let session = sessions.get(&key);
            // The newest live registration of the agent the pane runs now.
            let Some(record) = ledger.agents.iter().rev().find(|record| {
                !record.ended
                    && record.machine == agent.machine
                    && record.pane == agent.pane
                    && session.is_none_or(|session| record.actor.session.as_ref() == Some(session))
            }) else {
                return;
            };
            used.insert(record.id.clone());
            agent.id = Some(record.id.clone());
            agent.name = Some(record.wire_name().to_owned());
            agent.parent = record.parent.clone();
            agent.project = record.project.clone();
            agent.watched_by = watcher(&ledger, record);
            if agent.mission.is_none() {
                agent.mission = workers.get(&record.id).cloned();
            }
        };
        for pane in &mut panes {
            if let Some(agent) = &mut pane.agent {
                join(agent);
            }
        }
        let mut agents: Vec<PsAgent> = loose
            .into_iter()
            .map(|mut agent| {
                join(&mut agent);
                agent
            })
            .collect();
        agents.extend(
            ledger
                .agents
                .iter()
                .filter(|record| !used.contains(&record.id) && (all || !record.ended))
                .map(|record| PsAgent {
                    id: Some(record.id.clone()),
                    name: Some(record.wire_name().to_owned()),
                    kind: record.actor.kind.clone(),
                    state: if record.ended {
                        PsState::Ended
                    } else if disconnected.contains(&record.machine) {
                        PsState::Disconnected
                    } else {
                        PsState::Unknown
                    },
                    label: None,
                    line: None,
                    machine: record.machine.clone(),
                    pane: record.pane.clone(),
                    parent_machine: None,
                    parent_pane: None,
                    parent: record.parent.clone(),
                    watched_by: (!record.ended).then(|| watcher(&ledger, record)).flatten(),
                    mission: workers.get(&record.id).cloned(),
                    project: record.project.clone(),
                }),
        );
        Ps { panes, agents }
    }
}

fn watcher(ledger: &Ledger, record: &crate::coordination::AgentRecord) -> Option<String> {
    ledger
        .watches
        .iter()
        .find(|watch| watch.target.same_identity(&record.actor))
        .map(|watch| watch.parent.name.clone())
}
