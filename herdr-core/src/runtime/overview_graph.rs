//! Session proof and launch provenance from the already-held canonical rows,
//! and the operator's remembered choices on the map (PRD
//! overview-graph-remembered), changed one action per event.
use super::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OverviewGraphChangePayload {
    pub(super) change: crate::model::OverviewGraphChange,
}

impl Runtime {
    pub(super) fn refresh_overview_graph_sessions(&mut self) -> bool {
        let launches = &self.snapshot.ui_state.pane_launches;
        let mut sessions = BTreeMap::new();
        let mut add = |agent: &SidebarAgentSnapshot, device: &str| {
            let Some(digest) = agent.lineage_session.as_ref() else {
                return;
            };
            let choice = launches
                .get(&agent.pane_id)
                .filter(|launch| {
                    hide_agent_adapter::canonical_kind(&agent.agent_kind) == launch.kind
                        && launch.session.as_ref().is_some_and(|session| {
                            crate::wire::session_digest(session).as_ref() == Some(digest)
                        })
                })
                .map(|launch| &launch.choice);
            sessions.insert(
                (device.to_owned(), agent.pane_id.clone()),
                crate::model::GraphSession {
                    device_id: device.to_owned(),
                    pane_id: agent.pane_id.clone(),
                    session_digest: digest.clone(),
                    launch_model: choice.and_then(|choice| choice.model.clone()),
                    launch_effort: choice.and_then(|choice| choice.effort.clone()),
                },
            );
        };
        for agent in &self.snapshot.navigator.agents {
            // Navigator rows include device-qualified cross-device descendants.
            let device = agent
                .pane_id
                .strip_prefix("remote:")
                .and_then(|rest| rest.split_once(":pane:"))
                .map(|(device, _)| device)
                .unwrap_or(self.node.as_str());
            add(agent, device);
        }
        for remote in &self.snapshot.status.remote {
            if let Some(session) = &remote.session {
                for agent in &session.agents {
                    add(agent, &remote.target_id);
                }
            }
        }
        let after: Vec<_> = sessions.into_values().collect();
        if after == self.snapshot.overview_graph.sessions {
            return false;
        }
        self.snapshot.overview_graph.sessions = after;
        true
    }
}

impl Runtime {
    /// One operator action on the map: a filter, a hide, a fold, a drop or
    /// Reset layout (D-10). Whether anything changed.
    pub(super) fn change_overview_graph(&mut self, payload: OverviewGraphChangePayload) -> bool {
        match self.snapshot.ui_state.overview_graph.apply(payload.change) {
            Err(message) => {
                self.set_error("overview_graph.invalid", message, false);
                true
            }
            Ok(applied) => {
                if applied.forgotten > 0 {
                    crate::diagnostic!(serde_json::json!({
                        "component": "overview_graph",
                        "kind": "overview_graph.forgotten",
                        "records": applied.forgotten,
                        "limit": crate::model::OVERVIEW_GRAPH_LIST_LIMIT,
                    }));
                }
                if applied.changed {
                    self.persist_ui_state();
                }
                applied.changed
            }
        }
    }
}
