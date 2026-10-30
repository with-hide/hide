//! Checkout removal keeps the last known use, including disconnected descendants.
use super::super::turn::AgentUse;
use crate::model::{Snapshot, WorkspaceSnapshot};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

pub(crate) fn cleanup_facts(
    snapshot: &Snapshot,
    workspace: &WorkspaceSnapshot,
) -> Vec<crate::worktree_cleanup::CheckoutFacts> {
    // A delegated agent can run on a connected device, so its state is
    // read from the device's own rows too.
    let agents: HashMap<&str, &crate::model::SidebarAgentSnapshot> = snapshot
        .navigator
        .agents
        .iter()
        .chain(
            snapshot
                .status
                .remote
                .iter()
                .filter_map(|status| status.session.as_ref())
                .flat_map(|session| session.agents.iter()),
        )
        .map(|agent| (agent.pane_id.as_str(), agent))
        .collect();
    let state = |agent: &crate::model::SidebarAgentSnapshot| {
        AgentUse::of(&agent.demand, agent.blocked, &agent.activity)
    };
    workspace
        .checkouts
        .iter()
        .map(|checkout| {
            let panes: Vec<String> = checkout
                .tabs
                .iter()
                .flat_map(|tab| tab.panes.iter())
                .map(|pane| pane.id.clone())
                .collect();
            let mut facts = crate::worktree_cleanup::CheckoutFacts {
                path: PathBuf::from(&checkout.path),
                ..Default::default()
            };
            for pane in &panes {
                let Some(agent) = agents.get(pane.as_str()) else {
                    facts.terminal_panes.push(pane.clone());
                    continue;
                };
                match state(agent) {
                    AgentUse::Quiet => {}
                    AgentUse::Working => facts.agent_working += 1,
                    AgentUse::Waiting => facts.agent_waiting += 1,
                    AgentUse::Unknown => facts.agent_unknown += 1,
                }
                // A delegated agent keeps its parent's checkout in use
                // wherever it runs, and a device that went away leaves its
                // last state, not an idle one. Ones in this checkout are
                // counted above, and an unknown one is not vouched for
                // either way. The children are walked rather than the
                // close list, which leaves out unreachable devices.
                let mut seen: HashSet<&str> = HashSet::new();
                let mut pending: Vec<&str> = agent
                    .lineage_child_pane_ids
                    .iter()
                    .map(String::as_str)
                    .collect();
                while let Some(id) = pending.pop() {
                    if !seen.insert(id) {
                        continue;
                    }
                    let Some(descendant) = agents.get(id) else {
                        continue;
                    };
                    pending.extend(descendant.lineage_child_pane_ids.iter().map(String::as_str));
                    if !panes.iter().any(|own| own == id)
                        && matches!(state(descendant), AgentUse::Working | AgentUse::Waiting)
                    {
                        facts.descendants_busy += 1;
                    }
                }
            }
            facts.panes = panes;
            facts
        })
        .collect()
}
