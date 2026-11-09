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
//!
//! A row is one agent item (PRD agent-profiles-phase2 B11): the agent's
//! built-in default, `profile` `default`, then each of the machine's
//! profiles of it a start may take, `profile` its name, which is the
//! `--profile` value that starts it. A profile's models are the ones its
//! own account, variables and wrapper list, or its agent's default list
//! when that could not be read (`models_from_default`); its weekly figure
//! is its own account's last reading.

use serde_json::{Value, json};

use super::Runtime;
use crate::model::{BackgroundAiProviderSnapshot, ProfileSnapshot};

/// One agent item of a catalog: an agent, and its profile or none for the
/// built-in default.
struct Item<'a> {
    adapter: &'static hide_agent_adapter::AgentAdapter,
    profile: Option<&'a ProfileSnapshot>,
}

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
        let machine = match machine {
            Some(name) => self.machine_named(name)?,
            None => device.to_owned(),
        };
        let machine = machine.as_str();
        let on_core = machine == self.node.as_str();
        if !self.device_answers(machine) {
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
        let profiles = self.machine_profiles(machine);
        let catalog_items: Vec<Item<'_>> = hide_agent_adapter::ADAPTERS
            .iter()
            .filter(|adapter| adapter.start.is_some())
            .flat_map(|adapter| {
                std::iter::once(Item {
                    adapter,
                    profile: None,
                })
                .chain(
                    profiles
                        .iter()
                        .flat_map(|profiles| &profiles.profiles)
                        .filter(|profile| profile.kind == adapter.id && profile.startable)
                        .map(|profile| Item {
                            adapter,
                            profile: Some(profile),
                        }),
                )
            })
            .collect();
        // One read is asked for while a model list is unread on the core's
        // own machine; a device's lists are never read here.
        let pending = on_core
            && !timed_out
            && catalog_items.iter().any(|item| {
                item.adapter
                    .start
                    .is_some_and(|dialect| !dialect.options().model.is_empty())
                    && self.item_unread(item)
            });
        let mut items: Vec<Value> = catalog_items
            .iter()
            .map(|item| self.catalog_row(item, machine, on_core, timed_out))
            .collect();
        drop(catalog_items);
        if pending {
            self.ask_ai_catalog_read();
        }
        let kit = self.kit_states.get(machine);
        // With the development switch on, the core's own machine also lists
        // every agent definition: where each came from and which screen
        // rules it reads (PRD engine-bundle-3-core-owns B21). Off, there is
        // no state and the answer is the seven agents as before (B1).
        let definitions = self.agent_definitions.as_ref().filter(|_| on_core);
        let Some(definitions) = definitions else {
            return Ok(json!({
                "machine": machine,
                // A machine whose kit has not reported yet cannot say which
                // agents it has; `installed` is then null on every row.
                "installed_known": kit.is_some(),
                // A read of the model lists is on its way; the answer after it
                // lands names the models.
                "models_read_pending": pending,
                "items": items,
            }));
        };
        for item in &mut items {
            let id = item["id"].as_str().unwrap_or_default().to_owned();
            let definition = definitions
                .catalog
                .definitions
                .iter()
                .find(|definition| definition.id == id);
            if let (Some(row), Some(definition)) = (item.as_object_mut(), definition) {
                row.extend(definition_fields(definition, true));
            }
        }
        let started: Vec<&str> = hide_agent_adapter::ADAPTERS
            .iter()
            .filter(|adapter| adapter.start.is_some())
            .map(|adapter| adapter.id)
            .collect();
        for definition in &definitions.catalog.definitions {
            if started.contains(&definition.id.as_str()) {
                continue;
            }
            // Hide's own features and a spawn come with the agents node of
            // the same bundle; until then such an agent is listed only.
            let mut row = json!({
                "agent": definition.id,
                "id": definition.id,
                "label": definition.name,
                "profile": "default",
                "installed": Value::Null,
                "enabled": Value::Null,
                "takes_model": false,
                "models": Value::Null,
                "model_labels": Value::Null,
                "models_unavailable_reason": Value::Null,
                "models_from_default": false,
                "cli_default_model": Value::Null,
                "efforts": [],
                "signed_in": Value::Null,
                "weekly_used_percent": Value::Null,
                "weekly_resets_at_ms": Value::Null,
                "past_usage_limit": false,
            });
            if let Some(row) = row.as_object_mut() {
                row.extend(definition_fields(definition, false));
            }
            items.push(row);
        }
        Ok(json!({
            "machine": machine,
            "installed_known": kit.is_some(),
            "models_read_pending": pending,
            "items": items,
            // Each of the operator's files left out, with why in one line.
            "definitions_rejected": definitions
                .catalog
                .rejected
                .iter()
                .map(|rejected| json!({ "file": rejected.file, "reason": rejected.reason }))
                .collect::<Vec<_>>(),
            "definitions_unreadable": definitions.unreadable(),
        }))
    }

    /// Whether the model list of `item` on the core's own machine is still
    /// to be read: its agent's row unread, or a profile the last read did
    /// not list.
    fn item_unread(&self, item: &Item<'_>) -> bool {
        let Some(provider) = self
            .catalog_provider(item.adapter.id)
            .filter(|provider| provider.state != "unread")
        else {
            return true;
        };
        item.profile.is_some_and(|profile| {
            !provider
                .profile_models
                .iter()
                .any(|models| models.profile == profile.id)
        })
    }

    /// The catalog's row of one item on `machine`.
    fn catalog_row(&self, item: &Item<'_>, machine: &str, on_core: bool, timed_out: bool) -> Value {
        let adapter = item.adapter;
        let options = adapter
            .start
            .map(|dialect| dialect.options())
            .expect("a catalog item is an agent Hide starts");
        let reported = self.catalog_kit_agent(machine, adapter.id);
        let installed =
            reported.map(|agent| agent.availability == hide_kit::Availability::Available);
        let provider = on_core.then(|| self.catalog_provider(adapter.id)).flatten();
        let takes_model = !options.model.is_empty();
        // Hide AI reads model lists on the core's own machine only; a
        // device's agents are started with a model the caller names or the
        // CLI's default.
        let models = if !takes_model {
            Models::none()
        } else if !on_core {
            Models::missing("models_read_on_this_machine_only")
        } else if timed_out && self.item_unread(item) {
            Models::missing("models_read_timeout")
        } else {
            match (provider, item.profile) {
                (None, _) => Models::missing("models_not_read_by_hide_ai"),
                (Some(provider), None) => Models::of_agent(provider),
                (Some(provider), Some(profile)) => Models::of_profile(provider, &profile.id),
            }
        };
        // The model the agent runs with when asked for none, as Hide AI
        // measured it (the snapshot's `model` before any read or choice), so
        // a plan can name one while the list is unread; null for an agent
        // whose own CLI keeps that default.
        let cli_default_model = takes_model
            .then(|| hide_ai::ProviderId::for_agent(adapter.id))
            .flatten()
            .map(|provider| provider.default_model())
            .filter(|model| !model.is_empty());
        let (signed_in, weekly) = match item.profile {
            None => (
                provider.map(|provider| provider.state != "needs_login"),
                provider.and_then(|provider| {
                    provider
                        .weekly
                        .map(|weekly| (weekly.used_percent, weekly.resets_at_ms))
                }),
            ),
            Some(profile) => (
                profile.login.and_then(|login| match login {
                    hide_node_link::profiles::ProfileLogin::SignedIn => Some(true),
                    hide_node_link::profiles::ProfileLogin::SignedOut => Some(false),
                    hide_node_link::profiles::ProfileLogin::Unknown => None,
                }),
                on_core
                    .then(|| self.profile_weekly(adapter, profile))
                    .flatten(),
            ),
        };
        let past_usage_limit = match item.profile {
            None => provider.is_some_and(|provider| provider.past_limit),
            Some(_) => weekly.is_some_and(|(used, _)| {
                self.ai_settings
                    .as_ref()
                    .and_then(|settings| settings.usage_limit.threshold())
                    .is_some_and(|limit| used >= f64::from(limit))
            }),
        };
        json!({
            "agent": adapter.engine_kind,
            "id": adapter.id,
            "label": adapter.label,
            "profile": item.profile.map_or("default", |profile| profile.name.as_str()),
            "installed": installed,
            "enabled": reported.map(|agent| agent.enabled),
            "takes_model": takes_model,
            "models": models.models,
            "model_labels": models.labels,
            "models_unavailable_reason": models.reason,
            "models_from_default": models.from_default,
            "cli_default_model": cli_default_model,
            "efforts": options.efforts,
            "signed_in": signed_in,
            "weekly_used_percent": weekly.map(|(used, _)| used),
            "weekly_resets_at_ms": weekly.and_then(|(_, resets)| resets),
            "past_usage_limit": past_usage_limit,
        })
    }

    /// The weekly figure of `profile`'s own account, as the usage read last
    /// answered it: a profile of an account of its own reads its row, one
    /// sharing another profile's account reads that one's, and one in the
    /// agent's default account (a wrapper or variables alone, which may run
    /// it elsewhere, as a gateway does) has none.
    pub(super) fn profile_weekly(
        &self,
        adapter: &hide_agent_adapter::AgentAdapter,
        profile: &ProfileSnapshot,
    ) -> Option<(f64, Option<u64>)> {
        let key = profile
            .account_key
            .as_ref()
            .filter(|key| !key.is_default())?;
        let now_s = super::unix_milliseconds() / 1000;
        self.machine_profiles(self.node.as_str())?
            .profiles
            .iter()
            .filter(|other| other.kind == profile.kind && other.account_key.as_ref() == Some(key))
            .find_map(|other| {
                self.snapshot
                    .usage
                    .profile_weekly_reading(adapter.engine_kind, &other.id, now_s)
            })
            .map(|(used, resets_s)| (used, Some(resets_s.saturating_mul(1000))))
    }

    /// What `machine`'s kit last reported of the agent `id` (its switch and
    /// whether its program is there), which the catalog, Settings › Agents
    /// and Desk's bar read alike; `None` before the kit reported it.
    pub(super) fn catalog_kit_agent(
        &self,
        machine: &str,
        id: &str,
    ) -> Option<&crate::model::KitAgentSnapshot> {
        self.kit_states
            .get(machine)
            .and_then(|kit| kit.agents.iter().find(|agent| agent.id == id))
    }

    /// Hide AI's last read of the agent `id` on the core's own machine,
    /// which says whether the operator is signed in to it.
    pub(super) fn catalog_provider(
        &self,
        id: &str,
    ) -> Option<&crate::model::BackgroundAiProviderSnapshot> {
        self.snapshot
            .status
            .background_ai
            .providers
            .iter()
            .find(|provider| provider.agent == id)
    }
}

/// Where a catalog row's definition came from, and the screen rules it reads
/// with their source and version (`built-in`, or the operator's file under
/// `~/.hide/agents`).
fn definition_fields(
    definition: &hide_agent_adapter::definition::Definition,
    spawn: bool,
) -> serde_json::Map<String, Value> {
    use hide_agent_adapter::definition::Source;
    let source = |source: &Source| match source {
        Source::Builtin => json!({ "kind": "built-in", "file": Value::Null }),
        Source::User(file) => json!({ "kind": "user", "file": format!("~/.hide/agents/{file}") }),
    };
    let mut fields = serde_json::Map::new();
    fields.insert("source".into(), source(&definition.source));
    fields.insert("based_on".into(), json!(definition.based_on));
    fields.insert(
        "hooks".into(),
        json!(format!("{:?}", definition.hooks).to_lowercase()),
    );
    fields.insert(
        "screen".into(),
        definition.screen.as_ref().map_or(Value::Null, |screen| {
            json!({ "source": source(&screen.source), "version": screen.rules.version().as_str() })
        }),
    );
    fields.insert("spawn".into(), json!(spawn));
    fields
}

/// A catalog row's models: the list by the names a start passes, the names
/// the agent shows for them, or why there is none.
struct Models {
    models: Value,
    labels: Value,
    reason: Value,
    from_default: bool,
}

impl Models {
    fn none() -> Self {
        Self {
            models: Value::Null,
            labels: Value::Null,
            reason: Value::Null,
            from_default: false,
        }
    }

    fn missing(reason: &str) -> Self {
        Self {
            reason: json!(reason),
            ..Self::none()
        }
    }

    /// The agent's built-in default list as its row holds it.
    fn of_agent(provider: &BackgroundAiProviderSnapshot) -> Self {
        if provider.models.is_empty() {
            return Self::missing(
                &provider
                    .models_unavailable_reason
                    .clone()
                    .unwrap_or_else(|| format!("models_{}", provider.state)),
            );
        }
        Self {
            models: json!(provider.models),
            labels: json!(provider.model_labels),
            ..Self::none()
        }
    }

    /// The list `profile`'s own environment gave, or the agent's when it gave
    /// none (B9): the reason is in the log, and the row says it is the
    /// agent's list. A profile the last read did not reach is unread.
    fn of_profile(provider: &BackgroundAiProviderSnapshot, profile: &str) -> Self {
        match provider
            .profile_models
            .iter()
            .find(|models| models.profile == profile)
        {
            None => Self::missing("models_unread"),
            Some(models) if !models.models.is_empty() => Self {
                models: json!(models.models),
                labels: json!(models.model_labels),
                ..Self::none()
            },
            Some(_) => Self {
                from_default: true,
                ..Self::of_agent(provider)
            },
        }
    }
}
