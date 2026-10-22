//! `hide agent catalog [--machine <device>]` (PRD software-factory-conductor
//! D-35, B10): the agents a machine has, the models each offers and the
//! efforts each takes, for any agent that chooses who to start. It reads
//! what the core already holds, the machine's install kit report and Hide
//! AI's model list, and asks nothing of a machine itself.

use serde_json::{Value, json};

use super::Runtime;

impl Runtime {
    /// The catalog of `machine`, or of the caller's own machine `device`
    /// when it names none. A model list that was not read says why instead
    /// of answering an empty one.
    pub(crate) fn agent_catalog(
        &self,
        device: &str,
        machine: Option<&str>,
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
                json!({
                    "agent": adapter.herdr.name,
                    "id": adapter.id,
                    "label": adapter.label,
                    "installed": installed,
                    "enabled": reported.map(|agent| agent.enabled),
                    "takes_model": takes_model,
                    "models": models,
                    "models_unavailable_reason": models_reason,
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
            "items": items,
        }))
    }
}
