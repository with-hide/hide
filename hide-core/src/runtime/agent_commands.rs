//! Agent command lists shared by every session input. Desk remains their producer.
use super::*;
use hide_desk::AdapterId;

impl Runtime {
    /// The command lists the last run kept, read before the lock by the
    /// caller, and the file the lists sent from now on are saved to.
    pub(crate) fn install_agent_commands(
        &mut self,
        store: crate::agent_commands::AgentCommandsStore,
        lists: crate::agent_commands::CommandLists,
    ) {
        self.agent_commands.store = Some(store);
        self.agent_commands.lists = lists;
        self.compose_agent_commands();
    }

    /// The commands `agent` listed last, while hided runs; none before it
    /// sent a list.
    pub(super) fn agent_commands_of(&self, agent: AdapterId) -> &[hide_desk::commands::Command] {
        self.agent_commands
            .lists
            .get(&agent)
            .map_or(&[], Vec::as_slice)
    }

    /// The `desk_commands` section, every agent's last list, sent again
    /// only when a list moved.
    pub(super) fn compose_agent_commands(&mut self) {
        if self
            .snapshot
            .desk_commands
            .as_deref()
            .is_some_and(|shown| shown.lists == self.agent_commands.lists)
        {
            return;
        }
        self.snapshot.desk_commands = Some(Arc::new(crate::model::DeskCommandsSnapshot {
            lists: self.agent_commands.lists.clone(),
        }));
    }
}
