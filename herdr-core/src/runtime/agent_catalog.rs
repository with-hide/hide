//! `hide agent catalog [--machine <device>]` (PRD software-factory-conductor
//! D-35, B10): the agents a machine has, the models each offers and the
//! efforts each takes, for any agent that chooses who to start. It reads
//! what the core already holds, the machine's install kit report and Hide
//! AI's model list, and asks nothing of a machine itself.
//!
//! Hide AI reads the model lists only while someone looks at them, so a
//! catalog asked before Settings › Agents or a start panel was ever opened
//! would answer `models_unread` for every agent. The catalog therefore asks
//! for one read the way the start panel does (`ask_ai_catalog_read`) and
//! says `models_read_pending` while it is on its way; the daemon asks again
//! until the read landed or its wait ran out, and then the answer carries
//! `models_read_timeout` with the CLI's default model, so a plan made from
//! it is never empty.

use serde_json::{Value, json};

use super::Runtime;

impl Runtime {
    /// The catalog of `machine`, or of the caller's own machine `device`
    /// when it names none. A model list that was not read says why instead
    /// of answering an empty one; `timed_out` says the caller's wait for a
    /// read is over, so an unread list is answered as such.
    pub(crate) fn agent_catalog(
        &mut self,
        device: &str,
        machine: Option<&str>,
        timed_out: bool,
    ) -> Result<Value, String> {
        let machine = machine.unwrap_or(device);
        let on_core = machine == self.node.as_str();
        if !on_core
            && !self
                .snapshot
                .status
                .remote
                .iter()
                .any(|remote| remote.target_id == machine && remote.state == "connected")
        {
            return Err(
                if self
                    .snapshot
                    .status
                    .remote
                    .iter()
                    .any(|remote| remote.target_id == machine)
                {
                    "machine_unavailable".into()
                } else {
                    "machine_unknown".into()
                },
            );
        }
        let unread = |provider: Option<&crate::model::BackgroundAiProviderSnapshot>| {
            provider.is_none_or(|provider| provider.state == "unread")
        };
        // One read is asked for while a model list is unread on the core's
        // own machine; a device's lists are never read here.
        let pending = on_core
            && !timed_out
            && hide_agent_adapter::ADAPTERS
                .iter()
                .filter_map(|adapter| Some((adapter, adapter.start?)))
                .filter(|(_, dialect)| !dialect.options().model.is_empty())
                .any(|(adapter, _)| {
                    unread(
                        self.snapshot
                            .status
                            .background_ai
                            .providers
                            .iter()
                            .find(|provider| provider.agent == adapter.id),
                    )
                });
        if pending {
            self.ask_ai_catalog_read();
        }
        let kit = self.kit_states.get(machine);
        let ai = &self.snapshot.status.background_ai;
        let items: Vec<Value> = hide_agent_adapter::ADAPTERS
            .iter()
            .filter_map(|adapter| Some((adapter, adapter.start?)))
            .map(|(adapter, dialect)| {
                let options = dialect.options();
                let reported = kit.and_then(|kit| kit.agents.iter().find(|agent| agent.id == adapter.id));
                let installed = reported.map(|agent| agent.availability == hide_kit::Availability::Available);
                let provider = on_core
                    .then(|| ai.providers.iter().find(|provider| provider.agent == adapter.id))
                    .flatten();
                let takes_model = !options.model.is_empty();
                // Hide AI reads model lists on the core's own machine only;
                // a device's agents are started with a model the caller
                // names or the CLI's default.
                let (models, models_reason) = if !takes_model {
                    (Value::Null, Value::Null)
                } else if !on_core {
                    (Value::Null, json!("models_read_on_this_machine_only"))
                } else if timed_out && unread(provider) {
                    (Value::Null, json!("models_read_timeout"))
                } else {
                    match provider {
                        Some(provider) if !provider.models.is_empty() => (json!(provider.models), Value::Null),
                        Some(provider) => (
                            Value::Null,
                            json!(provider
                                .models_unavailable_reason
                                .clone()
                                .unwrap_or_else(|| format!("models_{}", provider.state))),
                        ),
                        None => (Value::Null, json!("models_not_read_by_hide_ai")),
                    }
                };
                // The model the agent runs with when asked for none, as Hide
                // AI measured it (the snapshot's `model` before any read or
                // choice), so a plan can name one while the list is unread;
                // null for an agent whose own CLI keeps that default.
                let cli_default_model = takes_model
                    .then(|| hide_ai::ProviderId::for_agent(adapter.id))
                    .flatten()
                    .map(|provider| provider.default_model())
                    .filter(|model| !model.is_empty());
                json!({
                    "agent": adapter.herdr.name,
                    "id": adapter.id,
                    "label": adapter.label,
                    "installed": installed,
                    "enabled": reported.map(|agent| agent.enabled),
                    "takes_model": takes_model,
                    "models": models,
                    "models_unavailable_reason": models_reason,
                    "cli_default_model": cli_default_model,
                    "efforts": options.efforts,
                    "signed_in": provider.map(|provider| provider.state != "needs_login"),
                    "weekly_used_percent": provider.and_then(|provider| provider.weekly.map(|weekly| weekly.used_percent)),
                    "weekly_resets_at_ms": provider.and_then(|provider| provider.weekly.and_then(|weekly| weekly.resets_at_ms)),
                    "past_usage_limit": provider.is_some_and(|provider| provider.past_limit),
                })
            })
            .collect();
        Ok(json!({
            "machine": machine,
            // A machine whose kit has not reported yet cannot say which
            // agents it has; `installed` is then null on every row.
            "installed_known": kit.is_some(),
            // A read of the model lists is on its way; the answer after it
            // lands names the models.
            "models_read_pending": pending,
            "items": items,
        }))
    }
}
