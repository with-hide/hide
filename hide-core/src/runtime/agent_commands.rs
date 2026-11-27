//! Agent command lists shared by every session input. Desk remains their producer.
use super::*;
use hide_desk::AdapterId;

impl Runtime {
    /// The command lists the last run kept, read before the lock by the
    /// caller, and the file the lists sent from now on are saved to.
    pub(crate) fn install_agent_commands(
        &mut self,
        store: crate::agent_commands::AgentCommandsStore,
        saved: crate::agent_commands::SavedCommands,
    ) {
        self.agent_commands.store = Some(store);
        self.agent_commands.lists = saved.lists;
        self.agent_commands.read_marks = saved.read;
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

    /// Saves sent lists and their read marks off the runtime lock.
    pub(super) fn save_agent_commands(&self) {
        if let Some(store) = &self.agent_commands.store {
            let mut lists = self.agent_commands.lists.clone();
            lists.retain(|_, list| !list.is_empty());
            store.save(crate::agent_commands::SavedCommands {
                lists,
                read: self.agent_commands.read_marks.clone(),
            });
        }
    }

    /// The shared command section, sent again only when lists or reasons move.
    pub(super) fn compose_agent_commands(&mut self) {
        let unread = self.desk_commands_unread();
        if self
            .snapshot
            .desk_commands
            .as_deref()
            .is_some_and(|shown| shown.lists == self.agent_commands.lists && shown.unread == unread)
        {
            return;
        }
        self.snapshot.desk_commands = Some(Arc::new(crate::model::DeskCommandsSnapshot {
            lists: self.agent_commands.lists.clone(),
            unread,
        }));
    }
}
