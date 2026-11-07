//! Taking an agent hook's report (`HookCommand::Status`) into the core.
//!
//! The report is recorded in the shared hook facts under their own small
//! lock, inside this short runtime admission: no I/O, subprocess or
//! serialization happens here (PRD engine-bundle-1 B18). A report that
//! changes what a projection would lay wakes the coordinator of the pane's
//! machine, which publishes its session again with the facts laid on; one
//! that changes nothing wakes nothing, and the coordinator's publish
//! announces only a projection that moved.

use hide_node_link::panes::HookStatusReport;

use super::Runtime;

impl Runtime {
    /// Replaces the hook facts with the ones the last run saved; done once,
    /// before any coordinator starts.
    pub(crate) fn install_hook_facts(&mut self, facts: crate::agent_hooks::facts::HookFacts) {
        *self
            .hook_facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = facts;
    }

    /// The engine that keeps the session a hook in `caller`'s pane on
    /// `device` reported, with the id that engine gives the pane: the engine
    /// of the machine the pane runs on, this machine's or a dialed device's
    /// own (`own_devices`), which binds a spawned agent's identity by it and
    /// names the pane bare, as that machine's holders do, where the core
    /// names a device's pane with its device. The engine keeps that session
    /// with the pane, so a reboot resumes that conversation.
    pub(crate) fn hook_session_target(
        &self,
        device: &str,
        caller: &str,
        report: &HookStatusReport,
    ) -> Option<(crate::engine::SharedEngine, String)> {
        report.session.as_ref()?;
        let (machine, pane) = crate::address::split_pane(device, caller);
        let engine = self.delivery_engine(machine)?;
        Some((engine, pane.to_owned()))
    }

    /// Records what the hook in `caller`'s pane on `device` reported, unless
    /// the hook made it before the newest report the pane applied
    /// (`HookFacts::record`); answers whether it was applied.
    pub(crate) fn ingest_hook_status(
        &mut self,
        device: &str,
        caller: &str,
        report: &HookStatusReport,
    ) -> Result<bool, &'static str> {
        self.ingest_hook(device, caller, report, false)
    }

    /// Records a report `device`'s node replayed from its outbox for
    /// `caller`'s pane, when the hook made it after the newest report the
    /// pane applied (`HookFacts::replay`); answers whether it was applied.
    pub(crate) fn ingest_hook_replay(
        &mut self,
        device: &str,
        caller: &str,
        report: &HookStatusReport,
    ) -> Result<bool, &'static str> {
        self.ingest_hook(device, caller, report, true)
    }

    fn ingest_hook(
        &mut self,
        device: &str,
        caller: &str,
        report: &HookStatusReport,
        replayed: bool,
    ) -> Result<bool, &'static str> {
        // A machine's hooks speak for that machine's panes: a caller id that
        // names another device is no pane `device` may report on, whatever
        // carried it (a node's replayed outbox names its panes itself).
        if crate::address::device_of(caller).is_some_and(|named| named != device) {
            return Err("hook_pane_foreign");
        }
        let local = device == self.node.as_str();
        let pane = crate::address::named_pane_id(self.node.as_str(), device, caller);
        let recorded = {
            let mut facts = self
                .hook_facts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if replayed {
                facts.replay(&pane, report)?
            } else {
                facts.record(&pane, report)?
            }
        };
        if !recorded.changed {
            return Ok(!recorded.stale);
        }
        if local {
            // A turn the hook reports moved the agent where the engine may
            // not have: an agent with no screen rules (omp) reads at rest on
            // the core's own engine, so no state change of the engine's asks
            // the label reader to read its session again. The turn is when
            // its file was written; reported at the session's start, before
            // the file existed, the session stayed unconfirmed, and with it
            // the native id a fork needs (journey 41904789052).
            if report.turn.is_some() {
                self.queue_session_reread(pane.clone());
            }
            self.request_republish();
        } else if let Some(sync) = self
            .remote_connections
            .get(device)
            .and_then(|connection| connection.sync.as_ref())
        {
            sync.republish_waker().wake();
        }
        Ok(true)
    }
}
