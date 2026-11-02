//! What `hide session` takes from the runtime (PRD agent-session-read
//! D-14): owned copies made under the lock, each bounded by what was asked.
//! The record and list reads themselves run off the lock
//! (`links::session_command`).

use super::*;
use crate::links::ledger::SessionIdentity;
use crate::links::session_command::{DeviceFacts, LiveState, SessionFacts, SessionScope};
use std::collections::HashMap;

/// The sessions one facts call answers for; a list asks at most its page and
/// a record its root and children.
pub(crate) const FACTS_LIMIT: usize = 512;

impl Runtime {
    /// The Projects, devices and nodes a `hide session` command reads, and
    /// whether Hide AI's summaries are on; none when the record has no home.
    pub(crate) fn session_scope(
        &self,
        context: &crate::workspace_control::Context,
    ) -> Option<SessionScope> {
        let links = self.links_scope(context)?;
        let remote = |id: &str| {
            self.snapshot
                .status
                .remote
                .iter()
                .find(|remote| remote.target_id == id)
        };
        let devices = self
            .snapshot
            .navigator
            .devices
            .iter()
            .map(|device| {
                let local = device.id == self.node.as_str();
                DeviceFacts {
                    id: device.id.clone(),
                    label: device.label.clone(),
                    local,
                    unreachable_since_unix_ms: if local {
                        None
                    } else {
                        remote(&device.id).and_then(|remote| {
                            (remote.state != "connected")
                                .then(|| remote.unreachable_since_unix_ms.unwrap_or(0))
                        })
                    },
                }
            })
            .collect();
        let mut nodes = vec![(self.node.to_string(), self.own_node())];
        nodes.extend(self.ready_device_channels());
        Some(SessionScope {
            store: links.store,
            local_device: links.local_device,
            caller: links.caller,
            projects: links.projects,
            devices,
            nodes,
            // The screen's gate, so the marks match what it shows.
            summaries: super::backlog::summaries_on(self.ai_settings.as_ref()),
            now_unix_ms: unix_milliseconds(),
        })
    }

    /// The native session the registered agent `id` runs.
    pub(crate) fn registered_agent_session(&self, id: &str) -> Option<SessionIdentity> {
        let ledger = self.delivery_ledger.as_ref().ok()?;
        let record = ledger.agents.iter().rev().find(|record| record.id == id)?;
        (!record.session.is_empty()).then(|| SessionIdentity {
            device_id: record.actor.device_id.clone(),
            agent: super::links::session_agent(&record.actor.kind)
                .unwrap_or(record.actor.kind.as_str())
                .to_owned(),
            session_id: record.session.clone(),
        })
    }

    /// Each asked session's agent id, live state and archive time, in the
    /// order asked; past [`FACTS_LIMIT`] the rest answer nothing.
    pub(crate) fn session_facts(&self, sessions: &[SessionIdentity]) -> Vec<SessionFacts> {
        let asked = &sessions[..sessions.len().min(FACTS_LIMIT)];
        let wanted: std::collections::HashSet<&SessionIdentity> = asked.iter().collect();
        let mut agents: HashMap<SessionIdentity, String> = HashMap::new();
        if let Ok(ledger) = self.delivery_ledger.as_ref() {
            for record in &ledger.agents {
                let Some(kind) = super::links::session_agent(&record.actor.kind) else {
                    continue;
                };
                let identity = SessionIdentity {
                    device_id: record.actor.device_id.clone(),
                    agent: kind.to_owned(),
                    session_id: record.session.clone(),
                };
                if wanted.contains(&identity) {
                    // The newest registration of a session names it.
                    agents.insert(identity, record.id.clone());
                }
            }
        }
        let mut live: HashMap<SessionIdentity, LiveState> = HashMap::new();
        for (row, identity) in self.session_rows().zip(self.session_row_identities()) {
            let Some(identity) = identity.filter(|identity| wanted.contains(identity)) else {
                continue;
            };
            let kind = if row.state.needs_you {
                Some("needs_you")
            } else if row.state.working {
                Some("working")
            } else {
                None
            };
            let now = if row.state.needs_you {
                row.expected_reply.clone().or_else(|| row.detail.clone())
            } else {
                row.progress.clone().or_else(|| row.detail.clone())
            };
            live.insert(
                identity,
                LiveState {
                    kind,
                    request: row.record_summary.clone(),
                    now,
                },
            );
        }
        let archived: HashMap<SessionIdentity, u64> = self
            .archived_sessions()
            .filter(|(session, _)| wanted.contains(session))
            .collect();
        sessions
            .iter()
            .map(|session| SessionFacts {
                agent_id: agents.get(session).cloned(),
                live: live.get(session).cloned(),
                archived_at_unix_ms: archived.get(session).copied(),
            })
            .collect()
    }
}
