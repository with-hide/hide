//! The operator's provider and model choice, and the file it lives in.
//!
//! Two processes read this: Hide writes it from Settings, and the plugin that
//! consumes the router reads it back. Both link this crate, so the path, the
//! schema and the priority rule live here once rather than being agreed twice.
//!
//! The file is a sibling of Hide's own `state.json`, not a plugin's
//! configuration directory: a path keyed to a plugin id would tie Hide to
//! whichever plugin happened to be first.
//!
//! Every choice is of an agent item (PRD agent-profiles-phase2 D-07, B20): an
//! agent and the profile it runs as. The built-in default item is written as
//! every file before items wrote an agent, and a profile is a field an older
//! Hide does not know beside it, so an older Hide reads a newer file agent by
//! agent and a newer one reads an older file as built-in default items.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hide_agent_adapter::ProfileId;
use serde::{Deserialize, Serialize};

use crate::router::RouterConfig;
use crate::usage_limit::UsageLimit;
use crate::{AiError, Availability, Item, PROVIDERS, ProviderId};

/// What the operator chose. A field that is absent from the file takes the
/// default rather than making the whole file unreadable, and a field this
/// version does not know is ignored, so an older Hide reads a newer file.
///
/// An agent id this build does not know is ignored the same way: a `provider`
/// naming one reads as the default, a `models` entry for it is kept untouched
/// and a `fallback` entry for it is dropped.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawSettings", into = "RawSettingsOut")]
pub struct AiSettings {
    /// The item that answers: Settings › Hide AI › Runs on. Only meaningful
    /// while `chosen`; before then it is the first agent of the fixed order,
    /// which nothing asks anything of.
    pub runs_on: Item,
    /// Whether `runs_on` was chosen, by the operator or by the first-run
    /// rule (D-18), and so is stored. An unchosen settings value runs no
    /// background request: Hide features work without a model until an agent
    /// can answer (B47). It is the `provider` key being in the file.
    pub chosen: bool,
    /// The model each built-in default item is asked for, keyed by the
    /// provider's id. A provider with no entry is asked for its own default;
    /// an empty value is the CLI's default (no `--model`). The key is a
    /// string so an entry for an agent a newer Hide knows survives this
    /// build's rewrite.
    pub models: BTreeMap<String, String>,
    /// The model of each profile item, by the provider's id and then the
    /// profile's: `models` for items of a profile. Written only with an entry.
    pub profile_models: BTreeMap<String, BTreeMap<String, String>>,
    /// Whether agent labels are asked of a provider (Settings › Hide AI ›
    /// Features, Agent summaries). On unless the operator turned it off.
    pub agent_summary: bool,
    /// Use Hide AI: off, no background feature asks any model anything
    /// (Settings › Hide AI, D-14). The rest of the choice is kept so turning
    /// it back on resumes exactly where it was.
    pub enabled: bool,
    /// The items asked, in this order, when `runs_on` cannot answer (D-16).
    /// Empty by default, for an existing file too: nothing moves to another
    /// vendor unless the operator added it here.
    pub fallback: Vec<FallbackEntry>,
    /// How much of an account's week Hide AI may use before it moves to the
    /// next item of the order (Settings › Hide AI › Usage limit). One value
    /// for every feature Hide AI starts on its own; absent from the file it
    /// is 90%, and it is written only when the operator chose another.
    pub usage_limit: UsageLimit,
    /// The connected device each built-in default item runs on, by the
    /// agent's id: an item with no entry runs on the core's own machine (PRD
    /// wsl-device B11). One machine per item, so Runs on and a fallback entry
    /// never name the same item twice. The key is a string so an entry for
    /// an agent a newer Hide knows survives this build's rewrite; written
    /// only when an item runs on a device.
    pub machines: BTreeMap<String, String>,
    /// The device of each profile item placed on one, by the provider's id
    /// and then the profile's: `machines` for items of a profile, whose
    /// profile is that device's.
    pub profile_machines: BTreeMap<String, BTreeMap<String, String>>,
}

/// One item the operator added under "If <Runs on> can't answer".
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FallbackEntry {
    pub item: Item,
    /// The model it is asked for; the provider's default when absent.
    pub model: Option<String>,
}

/// Why a fallback entry was not added.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackRefusal {
    /// The item already answers first, so asking it again is no fallback.
    IsRunsOn,
    AlreadyListed,
}

/// The file as it is written: `provider` only once it was chosen, so a file
/// that only turned a switch does not make a choice for the operator, and a
/// built-in default item exactly as a file before items wrote it.
#[derive(Serialize)]
struct RawSettingsOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    models: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    profile_models: BTreeMap<String, BTreeMap<String, String>>,
    agent_summary: bool,
    enabled: bool,
    fallback: Vec<RawFallbackOut>,
    #[serde(skip_serializing_if = "UsageLimit::is_default")]
    usage_limit: UsageLimit,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    machines: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    profile_machines: BTreeMap<String, BTreeMap<String, String>>,
}

/// A fallback entry as it is written: `profile` only for a profile item,
/// beside the agent an older Hide reads it as.
#[derive(Serialize)]
struct RawFallbackOut {
    provider: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

impl From<AiSettings> for RawSettingsOut {
    fn from(settings: AiSettings) -> Self {
        let chosen = settings.chosen;
        Self {
            provider: chosen.then(|| settings.runs_on.provider.as_str()),
            profile: chosen
                .then(|| settings.runs_on.profile_id().map(str::to_owned))
                .flatten(),
            models: settings.models,
            profile_models: settings.profile_models,
            agent_summary: settings.agent_summary,
            enabled: settings.enabled,
            fallback: settings
                .fallback
                .into_iter()
                .map(|entry| RawFallbackOut {
                    provider: entry.item.provider.as_str(),
                    profile: entry.item.profile_id().map(str::to_owned),
                    model: entry.model,
                })
                .collect(),
            usage_limit: settings.usage_limit,
            machines: settings.machines,
            profile_machines: settings.profile_machines,
        }
    }
}

/// The file as it is read, before the registry judges the ids in it.
#[derive(Deserialize)]
struct RawSettings {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, String>,
    #[serde(default)]
    profile_models: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default = "on")]
    agent_summary: bool,
    #[serde(default = "on")]
    enabled: bool,
    #[serde(default)]
    fallback: Vec<RawFallback>,
    #[serde(default)]
    usage_limit: UsageLimit,
    #[serde(default)]
    machines: BTreeMap<String, String>,
    #[serde(default)]
    profile_machines: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Deserialize)]
struct RawFallback {
    provider: String,
    #[serde(default)]
    profile: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

/// The item a file names: its agent, and the profile beside it, the built-in
/// default without one (or with the default's own id).
fn item_named(provider: ProviderId, profile: Option<String>) -> Item {
    Item::of(provider, profile.map(ProfileId))
}

/// A map of devices with the entries that name no device left out: an empty
/// device id is the core's machine.
fn placed(machines: BTreeMap<String, String>) -> BTreeMap<String, String> {
    machines
        .into_iter()
        .filter(|(_, device)| !device.is_empty())
        .collect()
}

impl From<RawSettings> for AiSettings {
    fn from(raw: RawSettings) -> Self {
        let named = raw.provider.as_deref().and_then(ProviderId::from_id);
        let mut settings = Self {
            runs_on: match named {
                Some(provider) => item_named(provider, raw.profile),
                None => Item::agent(default_provider()),
            },
            chosen: named.is_some(),
            models: raw.models,
            profile_models: raw.profile_models,
            agent_summary: raw.agent_summary,
            enabled: raw.enabled,
            fallback: Vec::new(),
            usage_limit: raw.usage_limit,
            machines: placed(raw.machines),
            profile_machines: raw
                .profile_machines
                .into_iter()
                .map(|(agent, profiles)| (agent, placed(profiles)))
                .filter(|(_, profiles)| !profiles.is_empty())
                .collect(),
        };
        for entry in raw.fallback {
            if let Some(entry_provider) = ProviderId::from_id(&entry.provider) {
                // A file edited by hand can name the same item twice or the
                // one that already answers first; both read as the list the
                // operator can build in Settings.
                let _ =
                    settings.add_fallback(item_named(entry_provider, entry.profile), entry.model);
            }
        }
        settings
    }
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            runs_on: Item::agent(default_provider()),
            chosen: false,
            models: PROVIDERS
                .iter()
                .filter(|provider| !provider.default_model().is_empty())
                .map(|provider| {
                    (
                        provider.as_str().to_owned(),
                        provider.default_model().to_owned(),
                    )
                })
                .collect(),
            profile_models: BTreeMap::new(),
            agent_summary: on(),
            enabled: on(),
            fallback: Vec::new(),
            usage_limit: UsageLimit::default(),
            machines: BTreeMap::new(),
            profile_machines: BTreeMap::new(),
        }
    }
}

fn on() -> bool {
    true
}

/// The agent a first run starts on when none can answer yet: the first in the
/// fixed order (D-18).
fn default_provider() -> ProviderId {
    PROVIDERS[0]
}

/// The model a provider is asked for when the operator has not chosen one.
/// It is the backend's own constant, never a second list.
pub fn default_model(provider: ProviderId) -> &'static str {
    provider.default_model()
}

/// The entry of `item` in a map keyed as the file keys one: by the agent for
/// a built-in default item, by the agent and then the profile for a profile
/// item.
fn entry<'a>(
    agents: &'a BTreeMap<String, String>,
    profiles: &'a BTreeMap<String, BTreeMap<String, String>>,
    item: &Item,
) -> Option<&'a String> {
    match item.profile_id() {
        None => agents.get(item.provider.as_str()),
        Some(profile) => profiles.get(item.provider.as_str())?.get(profile),
    }
}

/// Sets or, with `None`, removes the entry of `item` in such a map, leaving
/// no empty agent behind.
fn set_entry(
    agents: &mut BTreeMap<String, String>,
    profiles: &mut BTreeMap<String, BTreeMap<String, String>>,
    item: &Item,
    value: Option<String>,
) {
    let agent = item.provider.as_str().to_owned();
    match (item.profile_id(), value) {
        (None, Some(value)) => {
            agents.insert(agent, value);
        }
        (None, None) => {
            agents.remove(&agent);
        }
        (Some(profile), Some(value)) => {
            profiles
                .entry(agent)
                .or_default()
                .insert(profile.to_owned(), value);
        }
        (Some(profile), None) => {
            if let Some(listed) = profiles.get_mut(&agent) {
                listed.remove(profile);
                if listed.is_empty() {
                    profiles.remove(&agent);
                }
            }
        }
    }
}

impl AiSettings {
    /// The model this item will be asked for, whether or not the file named
    /// one: a profile item with none takes its agent's registry default,
    /// never the built-in default item's choice, since a profile can run
    /// another provider's models.
    pub fn model(&self, item: &Item) -> &str {
        entry(&self.models, &self.profile_models, item)
            .map(String::as_str)
            .unwrap_or_else(|| default_model(item.provider))
    }

    pub fn set_model(&mut self, item: &Item, model: impl Into<String>) {
        set_entry(
            &mut self.models,
            &mut self.profile_models,
            item,
            Some(model.into()),
        );
    }

    /// The device `item` runs on; `None` is the core's own machine.
    pub fn machine(&self, item: &Item) -> Option<&str> {
        entry(&self.machines, &self.profile_machines, item).map(String::as_str)
    }

    /// Places `item` on a connected device, or with `None` on the core's own
    /// machine, wherever it is in the order.
    pub fn place(&mut self, item: &Item, device: Option<String>) {
        let device = device.filter(|device| !device.is_empty());
        set_entry(&mut self.machines, &mut self.profile_machines, item, device);
    }

    /// Every item of the order: Runs on, then the fallback list.
    pub fn order(&self) -> Vec<Item> {
        self.router_config().priority
    }

    /// The device of each item placed on one: each agent this build knows,
    /// and each profile item of the order; every other item runs on the
    /// core's own machine.
    pub fn machines_by_item(&self) -> BTreeMap<Item, String> {
        PROVIDERS
            .iter()
            .map(|provider| Item::agent(*provider))
            .chain(self.order().into_iter().filter(|item| !item.is_default()))
            .filter_map(|item| {
                let device = self.machine(&item)?.to_owned();
                Some((item, device))
            })
            .collect()
    }

    /// The model each item's backend is built with: a fallback entry with
    /// its own model uses that model, everything else the model chosen for
    /// the item. One value for every agent's built-in default item, so a
    /// router built from it can also be asked about agents that are not in
    /// use, and one for each profile item of the order.
    pub fn models_by_item(&self) -> BTreeMap<Item, String> {
        PROVIDERS
            .iter()
            .map(|provider| Item::agent(*provider))
            .chain(self.order().into_iter().filter(|item| !item.is_default()))
            .map(|item| {
                let model = self.effective_model(&item).to_owned();
                (item, model)
            })
            .collect()
    }

    /// [`Self::models_by_item`] of the agents' built-in default items alone:
    /// what the Settings read of every agent is built with.
    pub fn default_models(&self) -> BTreeMap<ProviderId, String> {
        PROVIDERS
            .iter()
            .map(|provider| {
                let model = self.effective_model(&Item::agent(*provider)).to_owned();
                (*provider, model)
            })
            .collect()
    }

    /// The model `item` is asked for where it stands in the order.
    pub fn effective_model(&self, item: &Item) -> &str {
        if *item != self.runs_on
            && let Some(model) = self
                .fallback
                .iter()
                .find(|entry| entry.item == *item)
                .and_then(|entry| entry.model.as_deref())
        {
            return model;
        }
        self.model(item)
    }

    /// Makes `item` the one that answers (Runs on). An item that was in the
    /// fallback list leaves it, and the model chosen for it there becomes
    /// its model, because asking the same item twice is no fallback and the
    /// operator's choice of model should not be lost with the entry (D-27,
    /// B44).
    pub fn set_runs_on(&mut self, item: Item) {
        if let Some(position) = self.fallback.iter().position(|entry| entry.item == item) {
            let entry = self.fallback.remove(position);
            if let Some(model) = entry.model {
                self.set_model(&item, model);
            }
        }
        self.runs_on = item;
        self.chosen = true;
    }

    /// Adds an item at the end of the fallback list; order is the order of
    /// adding (D-16). Another item of the Runs on agent is a fallback like
    /// any other (PRD agent-profiles-phase2 B16): only the same item is
    /// refused.
    pub fn add_fallback(
        &mut self,
        item: Item,
        model: Option<String>,
    ) -> Result<(), FallbackRefusal> {
        if item == self.runs_on {
            return Err(FallbackRefusal::IsRunsOn);
        }
        if self.fallback.iter().any(|entry| entry.item == item) {
            return Err(FallbackRefusal::AlreadyListed);
        }
        self.fallback.push(FallbackEntry { item, model });
        Ok(())
    }

    pub fn remove_fallback(&mut self, item: &Item) {
        self.fallback.retain(|entry| entry.item != *item);
    }

    /// Sets the model of one fallback entry; `None` returns it to the
    /// provider's default. An item that is not listed is left alone.
    pub fn set_fallback_model(&mut self, item: &Item, model: Option<String>) {
        if let Some(entry) = self.fallback.iter_mut().find(|entry| entry.item == *item) {
            entry.model = model;
        }
    }

    /// The router policy this choice produces: the chosen item first, then
    /// the fallback list in the order it was added, and nothing else. An item
    /// that is in neither is never asked (D-16), however many are installed.
    ///
    /// Everything else about the policy stays the default: a chosen item
    /// that cannot answer still steps aside to the next one, and that move is
    /// still sticky, because the retry and cooldown constants are the
    /// defaults.
    pub fn router_config(&self) -> RouterConfig {
        let mut priority = vec![self.runs_on.clone()];
        priority.extend(self.fallback.iter().map(|entry| entry.item.clone()));
        RouterConfig {
            priority,
            // An unchosen settings value has no agent to ask (B47).
            enabled: self.enabled && self.chosen,
            ..RouterConfig::default()
        }
    }

    /// Whether a router built for `other` asks the same items, in the same
    /// order, for the same models, on the same machines as one built for this
    /// value. Whoever holds a router across a settings change compares with
    /// this, so a removed fallback item stops receiving text and an added one
    /// is tried.
    pub fn routes_like(&self, other: &Self) -> bool {
        self.order() == other.order()
            && self.models_by_item() == other.models_by_item()
            && self.machines_by_item() == other.machines_by_item()
    }

    /// The agent a first run should start on: the first of the fixed order
    /// that can answer, `None` when none can (D-18). `availability` is what
    /// the router reported; an agent missing from it cannot answer. A first
    /// run chooses a built-in default item only.
    ///
    /// This is only consulted while nobody has chosen. Once the operator has
    /// chosen, their choice stands even while it is degraded, because the
    /// router already reports the degradation and returns to the choice on
    /// its own.
    pub fn provider_for_first_run(
        availability: &[(ProviderId, Availability)],
    ) -> Option<ProviderId> {
        PROVIDERS.iter().copied().find(|provider| {
            availability
                .iter()
                .any(|(id, state)| id == provider && state.is_ready())
        })
    }
}

/// Where the settings file lives, given a home directory: hide's folder in
/// the state folder the system keeps under that home.
pub fn settings_path(home: &Path) -> PathBuf {
    hide_platform::host::state_dir_under(home)
        .join("hide")
        .join("ai.json")
}

/// Reads the operator's choice.
///
/// A file that is not there is not a failure: nobody has chosen yet, and the
/// defaults are the answer. Anything else is reported, because a file that
/// exists and cannot be read is a fact the caller has to state before it
/// falls back; see `AI_PROVIDERS.md`.
pub fn load(home: &Path) -> Result<AiSettings, AiError> {
    load_choice(home).map(Option::unwrap_or_default)
}

/// [`load`], with `None` when nobody has chosen yet.
pub fn load_choice(home: &Path) -> Result<Option<AiSettings>, AiError> {
    load_choice_at(&settings_path(home))
}

/// [`load_choice`] of the settings file at `path`, wherever it is kept (a
/// core move carries it in its copy).
pub fn load_choice_at(path: &Path) -> Result<Option<AiSettings>, AiError> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(error) => {
            return Err(AiError::Internal(format!(
                "ai_settings_unreadable:{}",
                error.kind()
            )));
        }
    };
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        AiError::InvalidOutput(format!("ai_settings_malformed:line={}", error.line()))
    })
}

/// Writes the operator's choice, creating the directory if it is not there.
///
/// The write goes to a temporary file in the same directory and is renamed
/// over the target, so a reader never sees a half-written file.
pub fn save(home: &Path, settings: &AiSettings) -> Result<(), AiError> {
    let path = settings_path(home);
    let parent = path
        .parent()
        .ok_or_else(|| AiError::Internal("ai_settings_path_without_parent".to_owned()))?;
    hide_platform::fs::private::create_dir_all(parent)
        .map_err(|error| AiError::Internal(format!("ai_settings_dir:{}", error.kind())))?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|_| AiError::Internal("ai_settings_not_encodable".to_owned()))?;
    let temporary = parent.join(format!("ai.json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, &bytes)
        .map_err(|error| AiError::Internal(format!("ai_settings_write:{}", error.kind())))?;
    std::fs::rename(&temporary, &path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        AiError::Internal(format!("ai_settings_rename:{}", error.kind()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The built-in default items of `providers`, in their order.
    fn agents(providers: &[ProviderId]) -> Vec<Item> {
        providers.iter().copied().map(Item::agent).collect()
    }

    fn home(name: &str) -> PathBuf {
        tempfile::Builder::new()
            .prefix(&format!("hide-ai-settings-{name}-"))
            .tempdir()
            .expect("a temporary home is creatable")
            .keep()
    }

    fn write_settings(home: &Path, body: &str) {
        let path = settings_path(home);
        std::fs::create_dir_all(path.parent().expect("the settings path has a parent"))
            .expect("the settings directory is creatable");
        std::fs::write(path, body).expect("the settings file is writable");
    }

    #[test]
    fn the_settings_file_sits_beside_hides_own_state() {
        let path = settings_path(Path::new("/Users/example"));
        assert!(path.starts_with("/Users/example"), "{}", path.display());
        assert!(path.ends_with("hide/ai.json"), "{}", path.display());
        #[cfg(target_os = "macos")]
        assert_eq!(
            path,
            PathBuf::from("/Users/example/Library/Application Support/hide/ai.json")
        );
    }

    #[test]
    fn no_file_means_the_defaults_rather_than_a_failure() {
        let home = home("absent");
        let settings = load(&home).expect("a missing file is not a failure");
        assert_eq!(settings, AiSettings::default());
        assert_eq!(settings.runs_on, Item::agent(ProviderId::CLAUDE));
        assert!(
            settings.enabled,
            "Hide AI is on until the operator turns it off"
        );
        assert!(
            settings.fallback.is_empty(),
            "nothing falls back by default"
        );
        assert_eq!(
            settings.model(&Item::agent(ProviderId::CODEX)),
            crate::codex::DEFAULT_MODEL
        );
        assert_eq!(
            settings.model(&Item::agent(ProviderId::CLAUDE)),
            crate::claude::DEFAULT_MODEL
        );
        assert_eq!(
            settings.model(&Item::agent(ProviderId::GROK)),
            "",
            "an agent measured against nothing yet is asked for its CLI default"
        );
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    #[test]
    fn a_missing_field_takes_its_default_and_an_unknown_field_is_ignored() {
        let home = home("partial");
        write_settings(
            &home,
            r#"{"provider":"claude","spending_cap":{"weekly":10},"schema_version":9}"#,
        );
        let settings = load(&home).expect("an unknown field does not make the file unreadable");
        assert_eq!(settings.runs_on, Item::agent(ProviderId::CLAUDE));
        assert!(
            settings.agent_summary,
            "a file written before the switch existed leaves summaries on"
        );
        assert!(
            settings.enabled,
            "a file written before the switch existed leaves Hide AI on"
        );
        assert_eq!(
            settings.model(&Item::agent(ProviderId::CLAUDE)),
            crate::claude::DEFAULT_MODEL,
            "a provider the file named no model for takes the backend's own default"
        );
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    /// B48: a file an earlier Hide wrote keeps its agent and models and gains
    /// no fallback, so nothing starts going to another vendor on upgrade.
    #[test]
    fn a_file_from_the_two_provider_build_loads_unchanged_with_an_empty_fallback() {
        let home = home("old-build");
        write_settings(
            &home,
            r#"{"provider":"codex","models":{"codex":"gpt-5.6-luna","claude":"haiku"},"agent_summary":false}"#,
        );
        let settings = load(&home).expect("the old file is readable");
        assert_eq!(settings.runs_on, Item::agent(ProviderId::CODEX));
        assert_eq!(settings.model(&Item::agent(ProviderId::CLAUDE)), "haiku");
        assert!(!settings.agent_summary);
        assert!(settings.enabled);
        assert!(settings.fallback.is_empty());
        assert_eq!(
            settings.router_config().priority,
            agents(&[ProviderId::CODEX]),
            "the other provider is no longer appended behind the choice"
        );
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    /// An agent only a newer Hide knows is ignored, not fatal, and its model
    /// entry is carried through a rewrite untouched.
    #[test]
    fn an_agent_this_build_does_not_know_is_ignored_and_its_model_is_kept() {
        let home = home("future");
        write_settings(
            &home,
            r#"{"provider":"some-future-agent","models":{"some-future-agent":"x-1"},"fallback":[{"provider":"some-future-agent"},{"provider":"codex","model":"m"}]}"#,
        );
        let settings = load(&home).expect("an unknown agent does not make the file unreadable");
        assert_eq!(settings.runs_on, Item::agent(ProviderId::CLAUDE));
        assert!(
            !settings.chosen,
            "a choice naming an agent this build does not know is no choice"
        );
        assert_eq!(
            settings.fallback,
            vec![FallbackEntry {
                item: Item::agent(ProviderId::CODEX),
                model: Some("m".to_owned())
            }],
            "only the agent this build knows stays in the list"
        );
        save(&home, &settings).expect("the file is writable");
        let rewritten: serde_json::Value =
            serde_json::from_slice(&std::fs::read(settings_path(&home)).unwrap()).unwrap();
        assert_eq!(rewritten["models"]["some-future-agent"], "x-1");
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    /// B38/D-15: a retired Gemini choice uses the same policy as any unknown
    /// provider, without silently enabling requests on the internal default.
    #[test]
    fn retired_gemini_choices_cannot_run_or_fall_back_and_keep_their_model_data() {
        for retired in ["gemini-cli", "gemini"] {
            let home = home("retired-gemini");
            write_settings(
                &home,
                &serde_json::json!({
                    "provider": retired,
                    "models": {(retired): "saved-model", "antigravity": "gemini-3.8-flash-high"},
                    "fallback": [{"provider": retired}, {"provider": "codex", "model": "saved-codex"}]
                })
                .to_string(),
            );
            let settings = load(&home).expect("retired settings remain readable");
            assert_eq!(settings.runs_on, Item::agent(ProviderId::CLAUDE));
            assert!(!settings.chosen);
            assert!(!settings.router_config().enabled);
            assert_eq!(
                settings.router_config().priority,
                agents(&[ProviderId::CLAUDE, ProviderId::CODEX])
            );
            assert_eq!(settings.fallback.len(), 1);
            assert_eq!(settings.fallback[0].item, Item::agent(ProviderId::CODEX));
            assert_eq!(settings.models[retired], "saved-model");
            assert_eq!(settings.models["antigravity"], "gemini-3.8-flash-high");
            save(&home, &settings).expect("the file is writable");
            let rewritten: serde_json::Value =
                serde_json::from_slice(&std::fs::read(settings_path(&home)).unwrap()).unwrap();
            assert!(rewritten.get("provider").is_none());
            assert_eq!(rewritten["models"][retired], "saved-model");
            assert_eq!(rewritten["fallback"][0]["provider"], "codex");
            assert_eq!(rewritten["fallback"].as_array().unwrap().len(), 1);
            assert!(!load(&home).unwrap().chosen);
            std::fs::remove_dir_all(home).expect("the temporary home is removable");
        }
    }

    /// B1, B7: a file with no limit reads 90, an explicit limit survives a
    /// rewrite, "none" is a choice and not an absence, and the default is not
    /// written so a machine that never chose one keeps the file it had.
    #[test]
    fn the_usage_limit_defaults_to_90_and_only_a_chosen_one_is_written() {
        let home = home("usage-limit");
        write_settings(&home, r#"{"provider":"claude"}"#);
        let mut settings = load(&home).expect("a file with no limit is readable");
        assert_eq!(settings.usage_limit, UsageLimit::Percent(90));
        save(&home, &settings).expect("the file is writable");
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(settings_path(&home)).unwrap()).unwrap();
        assert!(
            written.get("usage_limit").is_none(),
            "the default is not written: {written}"
        );

        for chosen in [
            UsageLimit::Percent(80),
            UsageLimit::Percent(95),
            UsageLimit::None,
        ] {
            settings.usage_limit = chosen;
            save(&home, &settings).expect("the file is writable");
            assert_eq!(load(&home).unwrap().usage_limit, chosen);
        }
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    /// A limit that is not a percent or "none" is stated, never read as 90.
    #[test]
    fn a_limit_nobody_can_mean_is_reported_rather_than_read_as_the_default() {
        let home = home("usage-limit-bad");
        write_settings(&home, r#"{"provider":"claude","usage_limit":150}"#);
        let error = load(&home).expect_err("150 percent is not a limit");
        assert_eq!(error.class(), "invalid_output");
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    /// The limit is read at each request through the shared cell, so changing
    /// it must never rebuild a router or be mistaken for a change of agents.
    #[test]
    fn changing_only_the_usage_limit_leaves_the_routing_alone() {
        let before = AiSettings::default();
        let mut after = before.clone();
        after.usage_limit = UsageLimit::None;
        assert!(before.routes_like(&after));
        assert_ne!(before, after);
    }

    #[test]
    fn malformed_json_is_reported_rather_than_read_as_the_defaults() {
        let home = home("malformed");
        write_settings(&home, "{\"provider\": ");
        let error = load(&home).expect_err("a file that cannot be parsed is a failure");
        assert_eq!(error.class(), "invalid_output");
        assert!(
            error.to_string().contains("ai_settings_malformed"),
            "the failure names what could not be read: {error}"
        );
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    #[test]
    fn a_saved_choice_is_what_the_next_load_returns() {
        let home = home("roundtrip");
        let mut settings = AiSettings {
            runs_on: Item::agent(ProviderId::CLAUDE),
            ..AiSettings::default()
        };
        settings.set_model(&Item::agent(ProviderId::CLAUDE), "sonnet");
        settings.agent_summary = false;
        settings.enabled = false;
        settings
            .add_fallback(
                Item::agent(ProviderId::CODEX),
                Some("gpt-5.6-luna".to_owned()),
            )
            .unwrap();
        settings
            .add_fallback(Item::agent(ProviderId::PI), None)
            .unwrap();
        save(&home, &settings).expect("the settings file is writable");
        assert_eq!(load(&home).expect("the saved file is readable"), settings);
        assert!(
            !settings_path(&home)
                .parent()
                .expect("the settings path has a parent")
                .join(format!("ai.json.{}.tmp", std::process::id()))
                .exists(),
            "the temporary file does not outlive the rename"
        );
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    #[test]
    fn the_chosen_agent_leads_and_only_the_fallback_list_follows_it() {
        let mut settings = AiSettings {
            runs_on: Item::agent(ProviderId::CLAUDE),
            chosen: true,
            ..AiSettings::default()
        };
        assert_eq!(
            settings.router_config().priority,
            agents(&[ProviderId::CLAUDE]),
            "an empty fallback list means no fallback, however many agents are installed"
        );
        settings
            .add_fallback(Item::agent(ProviderId::PI), None)
            .unwrap();
        settings
            .add_fallback(Item::agent(ProviderId::CODEX), None)
            .unwrap();
        let config = settings.router_config();
        assert_eq!(
            config.priority,
            agents(&[ProviderId::CLAUDE, ProviderId::PI, ProviderId::CODEX]),
            "the order of adding is the order of trying"
        );
        assert!(config.enabled);
        let defaults = RouterConfig::default();
        assert_eq!(config.availability_ttl, defaults.availability_ttl);
        assert_eq!(config.backoff_base, defaults.backoff_base);
        assert_eq!(
            config.max_transient_attempts,
            defaults.max_transient_attempts
        );
        assert_eq!(
            config.max_invalid_output_attempts,
            defaults.max_invalid_output_attempts
        );
        assert_eq!(config.default_cooldown, defaults.default_cooldown);
        assert_eq!(config.overall_deadline_cap, defaults.overall_deadline_cap);
    }

    #[test]
    fn turning_hide_ai_off_reaches_the_router_and_keeps_the_rest_of_the_choice() {
        let mut settings = AiSettings::default();
        settings
            .add_fallback(Item::agent(ProviderId::CODEX), None)
            .unwrap();
        settings.enabled = false;
        let config = settings.router_config();
        assert!(!config.enabled);
        assert_eq!(
            config.priority,
            agents(&[ProviderId::CLAUDE, ProviderId::CODEX])
        );
    }

    #[test]
    fn the_agent_that_answers_first_is_never_its_own_fallback_and_one_agent_is_listed_once() {
        let mut settings = AiSettings::default();
        assert_eq!(
            settings.add_fallback(Item::agent(ProviderId::CLAUDE), None),
            Err(FallbackRefusal::IsRunsOn)
        );
        settings
            .add_fallback(Item::agent(ProviderId::CODEX), None)
            .unwrap();
        assert_eq!(
            settings.add_fallback(Item::agent(ProviderId::CODEX), None),
            Err(FallbackRefusal::AlreadyListed)
        );
        settings.remove_fallback(&Item::agent(ProviderId::CODEX));
        assert!(settings.fallback.is_empty());
    }

    /// B44: choosing a fallback agent as Runs on takes it out of the list, and
    /// the model the operator picked for it there is not lost.
    #[test]
    fn choosing_a_listed_agent_as_runs_on_takes_it_out_of_the_list() {
        let mut settings = AiSettings::default();
        settings
            .add_fallback(Item::agent(ProviderId::CODEX), Some("gpt-x".to_owned()))
            .unwrap();
        settings
            .add_fallback(Item::agent(ProviderId::PI), None)
            .unwrap();
        settings.set_runs_on(Item::agent(ProviderId::CODEX));
        assert_eq!(settings.runs_on, Item::agent(ProviderId::CODEX));
        assert_eq!(
            settings.fallback,
            vec![FallbackEntry {
                item: Item::agent(ProviderId::PI),
                model: None
            }]
        );
        assert_eq!(settings.model(&Item::agent(ProviderId::CODEX)), "gpt-x");
    }

    #[test]
    fn a_hand_edited_file_with_a_repeated_or_leading_agent_reads_as_a_list_settings_could_build() {
        let home = home("hand-edited");
        write_settings(
            &home,
            r#"{"provider":"codex","fallback":[{"provider":"codex"},{"provider":"pi"},{"provider":"pi","model":"x"}]}"#,
        );
        let settings = load(&home).unwrap();
        assert_eq!(
            settings.fallback,
            vec![FallbackEntry {
                item: Item::agent(ProviderId::PI),
                model: None
            }]
        );
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    #[test]
    fn a_fallback_agent_is_built_with_its_own_model() {
        let mut settings = AiSettings::default();
        settings.set_model(&Item::agent(ProviderId::CODEX), "gpt-chosen");
        settings
            .add_fallback(
                Item::agent(ProviderId::CODEX),
                Some("gpt-fallback".to_owned()),
            )
            .unwrap();
        let models = settings.models_by_item();
        assert_eq!(models[&Item::agent(ProviderId::CODEX)], "gpt-fallback");
        assert_eq!(
            models[&Item::agent(ProviderId::CLAUDE)],
            crate::claude::DEFAULT_MODEL
        );
        assert_eq!(models[&Item::agent(ProviderId::PI)], "");
    }

    #[test]
    fn a_first_run_starts_on_the_first_agent_of_the_fixed_order_that_can_answer() {
        let one_ready = [
            (ProviderId::CODEX, Availability::NeedsLogin),
            (ProviderId::CLAUDE, Availability::NeedsLogin),
            (ProviderId::PI, Availability::Ready),
        ];
        assert_eq!(
            AiSettings::provider_for_first_run(&one_ready),
            Some(ProviderId::PI)
        );

        let both_ready = [
            (ProviderId::PI, Availability::Ready),
            (ProviderId::CODEX, Availability::Ready),
            (ProviderId::CLAUDE, Availability::Ready),
        ];
        assert_eq!(
            AiSettings::provider_for_first_run(&both_ready),
            Some(ProviderId::CLAUDE),
            "the fixed order decides, not the order the router reported"
        );

        let neither = [
            (ProviderId::CODEX, Availability::NotInstalled),
            (ProviderId::CLAUDE, Availability::NeedsLogin),
        ];
        assert_eq!(AiSettings::provider_for_first_run(&neither), None);
    }

    #[test]
    fn a_file_that_only_turned_a_switch_makes_no_choice_and_asks_no_agent() {
        let home = home("unchosen");
        let mut settings = AiSettings::default();
        assert!(!settings.chosen, "nothing is chosen until someone chooses");
        settings.agent_summary = false;
        save(&home, &settings).expect("the file is writable");
        let written: serde_json::Value =
            serde_json::from_slice(&std::fs::read(settings_path(&home)).unwrap()).unwrap();
        assert!(
            written.get("provider").is_none(),
            "the file does not name an agent nobody chose: {written}"
        );
        let reread = load(&home).expect("the file is readable");
        assert!(!reread.chosen && !reread.agent_summary);
        assert!(
            !reread.router_config().enabled,
            "with no agent chosen no request is asked of any model (B47)"
        );
        let mut chosen = reread;
        chosen.set_runs_on(Item::agent(ProviderId::CODEX));
        assert!(chosen.chosen && chosen.router_config().enabled);
        save(&home, &chosen).expect("the file is writable");
        assert!(
            load(&home).unwrap().chosen,
            "a saved choice reads as chosen"
        );
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    /// PRD wsl-device B11: an agent placed on a device keeps that place
    /// through a save, an older file reads every agent on the core's machine,
    /// and a router held across a move of an agent to another machine is
    /// rebuilt.
    #[test]
    fn an_agent_placed_on_a_device_stays_there_and_an_older_file_runs_everything_here() {
        let older: AiSettings =
            serde_json::from_str(r#"{"provider":"claude","models":{}}"#).unwrap();
        assert!(older.machines_by_item().is_empty());
        let written = serde_json::to_string(&older).unwrap();
        assert!(!written.contains("machines"), "{written}");

        let mut placed = older.clone();
        placed.set_runs_on(Item::agent(ProviderId::CLAUDE));
        placed.place(
            &Item::agent(ProviderId::CLAUDE),
            Some("wsl-ubuntu".to_owned()),
        );
        assert_eq!(
            placed.machine(&Item::agent(ProviderId::CLAUDE)),
            Some("wsl-ubuntu")
        );
        assert!(
            !placed.routes_like(&older),
            "the agent moved to another machine"
        );
        let reread: AiSettings =
            serde_json::from_str(&serde_json::to_string(&placed).unwrap()).unwrap();
        assert_eq!(reread, placed);

        placed.place(&Item::agent(ProviderId::CLAUDE), None);
        assert_eq!(placed.machine(&Item::agent(ProviderId::CLAUDE)), None);
        assert!(placed.routes_like(&older));
    }

    /// The PR summary feature was removed (PRD pr-card-and-session-record-redesign
    /// D-19): a file that still carries its switch reads, and the next write
    /// drops the key.
    #[test]
    fn a_retired_pr_summary_switch_in_the_file_is_read_past_and_not_written_back() {
        let settings = AiSettings::from(
            serde_json::from_str::<RawSettings>(r#"{"provider":"codex","pr_summary":false}"#)
                .unwrap(),
        );
        assert!(settings.chosen);
        let written = serde_json::to_string(&RawSettingsOut::from(settings)).unwrap();
        assert!(!written.contains("pr_summary"));
    }

    fn profile(provider: ProviderId, id: &str) -> Item {
        Item::of(provider, Some(ProfileId(id.to_owned())))
    }

    /// PRD agent-profiles-phase2 B20, D-12: a file that names agents only
    /// reads each as its built-in default item, and a choice of built-in
    /// default items is written byte for byte as a file before items was
    /// (the shape `docs/AI_PROVIDERS.md` shows), so nothing a machine
    /// already has changes on upgrade.
    #[test]
    fn a_file_of_agents_reads_as_their_default_items_and_writes_back_the_same_bytes() {
        let before = r#"{
  "provider": "claude",
  "models": {
    "claude": "sonnet",
    "codex": "gpt-5.6-luna"
  },
  "agent_summary": true,
  "enabled": true,
  "fallback": [
    {
      "provider": "codex",
      "model": "gpt-5.6-luna"
    },
    {
      "provider": "pi"
    }
  ],
  "usage_limit": 95,
  "machines": {
    "pi": "wsl-ubuntu"
  }
}"#;
        let settings: AiSettings = serde_json::from_str(before).unwrap();
        assert_eq!(settings.runs_on, Item::agent(ProviderId::CLAUDE));
        assert_eq!(
            settings.order(),
            agents(&[ProviderId::CLAUDE, ProviderId::CODEX, ProviderId::PI])
        );
        assert_eq!(
            settings.machine(&Item::agent(ProviderId::PI)),
            Some("wsl-ubuntu")
        );
        assert_eq!(serde_json::to_string_pretty(&settings).unwrap(), before);
    }

    /// B16, B20: Runs on, a fallback entry, a model and a machine of a
    /// profile item are kept beside the agent they belong to and read back
    /// as that item; the built-in default item of the same agent stays an
    /// item of its own.
    #[test]
    fn a_profile_item_is_written_beside_its_agent_and_read_back_as_that_item() {
        let work = profile(ProviderId::CLAUDE, "p-work");
        let gateway = profile(ProviderId::CLAUDE, "p-gw");
        let mut settings = AiSettings::default();
        settings.set_runs_on(work.clone());
        settings.set_model(&work, "opus");
        settings
            .add_fallback(gateway.clone(), Some("glm-4.6".to_owned()))
            .unwrap();
        settings
            .add_fallback(Item::agent(ProviderId::CLAUDE), None)
            .unwrap();
        settings.place(&gateway, Some("wsl-ubuntu".to_owned()));

        let written = serde_json::to_value(&settings).unwrap();
        assert_eq!(written["provider"], "claude");
        assert_eq!(written["profile"], "p-work");
        assert_eq!(written["profile_models"]["claude"]["p-work"], "opus");
        assert_eq!(
            written["fallback"],
            serde_json::json!([
                { "provider": "claude", "profile": "p-gw", "model": "glm-4.6" },
                { "provider": "claude" },
            ])
        );
        assert_eq!(written["profile_machines"]["claude"]["p-gw"], "wsl-ubuntu");
        assert!(written.get("machines").is_none(), "{written}");

        let reread: AiSettings = serde_json::from_value(written).unwrap();
        assert_eq!(reread, settings);
        assert_eq!(reread.model(&work), "opus");
        assert_eq!(
            reread.model(&Item::agent(ProviderId::CLAUDE)),
            crate::claude::DEFAULT_MODEL,
            "the built-in default item keeps its own model"
        );
        assert_eq!(reread.effective_model(&gateway), "glm-4.6");
        assert_eq!(reread.machine(&gateway), Some("wsl-ubuntu"));
        assert_eq!(reread.machine(&Item::agent(ProviderId::CLAUDE)), None);
        assert_eq!(
            reread.order(),
            vec![
                work.clone(),
                gateway.clone(),
                Item::agent(ProviderId::CLAUDE)
            ]
        );
        // The default profile's own id names the built-in default item.
        assert_eq!(
            Item::of(ProviderId::CLAUDE, Some(ProfileId::default_profile())),
            Item::agent(ProviderId::CLAUDE)
        );
    }

    /// B16: another item of the Runs on agent is a fallback like any other;
    /// the same item, as Runs on or listed already, is refused.
    #[test]
    fn another_item_of_the_runs_on_agent_is_a_fallback_and_the_same_item_is_refused() {
        let work = profile(ProviderId::CLAUDE, "p-work");
        let mut settings = AiSettings::default();
        settings.set_runs_on(work.clone());
        assert_eq!(
            settings.add_fallback(work.clone(), None),
            Err(FallbackRefusal::IsRunsOn)
        );
        settings
            .add_fallback(Item::agent(ProviderId::CLAUDE), None)
            .unwrap();
        settings
            .add_fallback(profile(ProviderId::CLAUDE, "p-gw"), None)
            .unwrap();
        assert_eq!(
            settings.add_fallback(profile(ProviderId::CLAUDE, "p-gw"), None),
            Err(FallbackRefusal::AlreadyListed)
        );
        // Choosing a listed item as Runs on takes it off the list with its model.
        settings.set_fallback_model(&Item::agent(ProviderId::CLAUDE), Some("haiku".to_owned()));
        settings.set_runs_on(Item::agent(ProviderId::CLAUDE));
        assert_eq!(
            settings.order(),
            vec![
                Item::agent(ProviderId::CLAUDE),
                profile(ProviderId::CLAUDE, "p-gw")
            ]
        );
        assert_eq!(settings.model(&Item::agent(ProviderId::CLAUDE)), "haiku");
        assert!(
            !settings.routes_like(&AiSettings {
                runs_on: work,
                ..settings.clone()
            }),
            "another item of the same agent is another route"
        );
    }

    /// The reading of `hide-ai/src/settings.rs` at tag `v1.2.5`, copied as it
    /// is there: the expected answer of an older Hide reading a newer file
    /// comes from that build's code, not from this one's.
    mod v1_2_5 {
        use std::collections::BTreeMap;

        use serde::Deserialize;

        use crate::ProviderId;
        use crate::usage_limit::UsageLimit;

        fn on() -> bool {
            true
        }

        /// The file as it is read, before the registry judges the ids in it.
        /// Every field is kept, though this reader uses only the order: each
        /// one's type is part of whether v1.2.5 can read the file at all.
        #[derive(Deserialize)]
        #[expect(
            dead_code,
            reason = "copied whole from v1.2.5, whose types decide the parse"
        )]
        pub(super) struct RawSettings {
            #[serde(default)]
            pub(super) provider: Option<String>,
            #[serde(default)]
            pub(super) models: BTreeMap<String, String>,
            #[serde(default = "on")]
            pub(super) agent_summary: bool,
            #[serde(default = "on")]
            pub(super) enabled: bool,
            #[serde(default)]
            pub(super) fallback: Vec<RawFallback>,
            #[serde(default)]
            pub(super) usage_limit: UsageLimit,
            #[serde(default)]
            pub(super) machines: BTreeMap<String, String>,
        }

        #[derive(Deserialize)]
        pub(super) struct RawFallback {
            pub(super) provider: String,
            #[serde(default)]
            pub(super) model: Option<String>,
        }

        /// What v1.2.5's `From<RawSettings>` makes of it: Runs on, and the
        /// fallback list its `add_fallback` builds (the Runs on agent and a
        /// repeated agent dropped).
        /// Runs on as the file named it, and the fallback list with each
        /// entry's model.
        pub(super) type Order = (Option<ProviderId>, Vec<(ProviderId, Option<String>)>);

        pub(super) fn read(bytes: &str) -> serde_json::Result<Order> {
            let raw: RawSettings = serde_json::from_str(bytes)?;
            let named = raw.provider.as_deref().and_then(ProviderId::from_id);
            let provider = named.unwrap_or(crate::PROVIDERS[0]);
            let mut fallback: Vec<(ProviderId, Option<String>)> = Vec::new();
            for entry in raw.fallback {
                if let Some(entry_provider) = ProviderId::from_id(&entry.provider)
                    && entry_provider != provider
                    && !fallback.iter().any(|(listed, _)| *listed == entry_provider)
                {
                    fallback.push((entry_provider, entry.model));
                }
            }
            Ok((named, fallback))
        }
    }

    /// B20, D-12: a file with profile items, read as v1.2.5 reads it, is read
    /// agent by agent and nothing fails: Runs on is its agent, a fallback
    /// entry of the Runs on agent or of an agent already listed is dropped,
    /// and the rest of the order is kept.
    #[test]
    fn a_file_of_profile_items_reads_agent_by_agent_in_the_v1_2_5_reader() {
        let mut settings = AiSettings::default();
        settings.set_runs_on(profile(ProviderId::CLAUDE, "p-work"));
        settings.set_model(&profile(ProviderId::CLAUDE, "p-work"), "opus");
        settings
            .add_fallback(profile(ProviderId::CLAUDE, "p-gw"), Some("opus".to_owned()))
            .unwrap();
        settings
            .add_fallback(
                Item::agent(ProviderId::CODEX),
                Some("gpt-5.6-luna".to_owned()),
            )
            .unwrap();
        settings
            .add_fallback(profile(ProviderId::CODEX, "p-team"), None)
            .unwrap();
        settings.place(
            &profile(ProviderId::CODEX, "p-team"),
            Some("wsl".to_owned()),
        );
        let bytes = serde_json::to_string_pretty(&settings).unwrap();

        let (provider, fallback) = v1_2_5::read(&bytes).expect("v1.2.5 reads the file");
        assert_eq!(provider, Some(ProviderId::CLAUDE));
        assert_eq!(
            fallback,
            [(ProviderId::CODEX, Some("gpt-5.6-luna".to_owned()))]
        );
    }
}
