//! Session proof and launch provenance from the already-held canonical rows.
use super::*;

impl Runtime {
    pub(super) fn refresh_overview_graph_sessions(&mut self) -> bool {
        let launches = &self.snapshot.ui_state.pane_launches;
        let mut sessions = BTreeMap::new();
        let mut add = |agent: &SidebarAgentSnapshot, device: &str| {
            let Some(digest) = agent.lineage_session.as_ref() else {
                return;
            };
            let launch_model = launches
                .get(&agent.pane_id)
                .filter(|launch| {
                    hide_agent_adapter::canonical_kind(&agent.agent_kind) == launch.kind
                        && launch.session.as_ref().is_some_and(|session| {
                            crate::wire::session_digest(session).as_ref() == Some(digest)
                        })
                })
                .and_then(|launch| launch.choice.model.clone());
            sessions.insert(
                (device.to_owned(), agent.pane_id.clone()),
                crate::model::GraphSession {
                    device_id: device.to_owned(),
                    pane_id: agent.pane_id.clone(),
                    session_digest: digest.clone(),
                    launch_model,
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
