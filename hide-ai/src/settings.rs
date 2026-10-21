//! The operator's provider and model choice, and the file it lives in.
//!
//! Two processes read this: Hide writes it from Settings, and the plugin that
//! consumes the router reads it back. Both link this crate, so the path, the
//! schema and the priority rule live here once rather than being agreed twice.
//!
//! The file is a sibling of Hide's own `state.json`, not a plugin's
//! configuration directory: a path keyed to a plugin id would tie Hide to
//! whichever plugin happened to be first.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::router::RouterConfig;
use crate::usage_limit::UsageLimit;
use crate::{AiError, Availability, PROVIDERS, ProviderId};

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
    /// The agent that answers: Settings › Hide AI › Runs on. Only meaningful
    /// while `chosen`; before then it is the first agent of the fixed order,
    /// which nothing asks anything of.
    pub provider: ProviderId,
    /// Whether `provider` was chosen, by the operator or by the first-run
    /// rule (D-18), and so is stored. An unchosen settings value runs no
    /// background request: Hide features work without a model until an agent
    /// can answer (B47). It is the `provider` key being in the file.
    pub chosen: bool,
    /// The model each provider is asked for, keyed by the provider's id. A
    /// provider with no entry is asked for its own default; an empty value is
    /// the CLI's default (no `--model`). The key is a string so an entry for
    /// an agent a newer Hide knows survives this build's rewrite.
    pub models: BTreeMap<String, String>,
    /// Whether agent labels are asked of a provider (Settings › Hide AI ›
    /// Features, Agent summaries). On unless the operator turned it off.
    pub agent_summary: bool,
    /// Use Hide AI: off, no background feature asks any model anything
    /// (Settings › Hide AI, D-14). The rest of the choice is kept so turning
    /// it back on resumes exactly where it was.
    pub enabled: bool,
    /// The agents asked, in this order, when `provider` cannot answer (D-16).
    /// Empty by default, for an existing file too: nothing moves to another
    /// vendor unless the operator added it here.
    pub fallback: Vec<FallbackEntry>,
    /// How much of an agent's week Hide AI may use before it moves to the
    /// next agent of the order (Settings › Hide AI › Usage limit). One value
    /// for every feature Hide AI starts on its own; absent from the file it
    /// is 90%, and it is written only when the operator chose another.
    pub usage_limit: UsageLimit,
}

/// One agent the operator added under "If <Runs on> can't answer".
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FallbackEntry {
    pub provider: ProviderId,
    /// The model it is asked for; the provider's default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Why a fallback entry was not added.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackRefusal {
    /// The agent already answers first, so asking it again is no fallback.
    IsRunsOn,
    AlreadyListed,
}

/// The file as it is written: `provider` only once it was chosen, so a file
/// that only turned a switch does not make a choice for the operator.
#[derive(Serialize)]
struct RawSettingsOut {
    #[serde(skip_serializing_if = "Option::is_none")]
    provider: Option<&'static str>,
    models: BTreeMap<String, String>,
    agent_summary: bool,
    enabled: bool,
    fallback: Vec<FallbackEntry>,
    #[serde(skip_serializing_if = "UsageLimit::is_default")]
    usage_limit: UsageLimit,
}

impl From<AiSettings> for RawSettingsOut {
    fn from(settings: AiSettings) -> Self {
        Self {
            provider: settings.chosen.then(|| settings.provider.as_str()),
            models: settings.models,
            agent_summary: settings.agent_summary,
            enabled: settings.enabled,
            fallback: settings.fallback,
            usage_limit: settings.usage_limit,
        }
    }
}

/// The file as it is read, before the registry judges the ids in it.
#[derive(Deserialize)]
struct RawSettings {
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    models: BTreeMap<String, String>,
    #[serde(default = "on")]
    agent_summary: bool,
    #[serde(default = "on")]
    enabled: bool,
    #[serde(default)]
    fallback: Vec<RawFallback>,
    #[serde(default)]
    usage_limit: UsageLimit,
}

#[derive(Deserialize)]
struct RawFallback {
    provider: String,
    #[serde(default)]
    model: Option<String>,
}

impl From<RawSettings> for AiSettings {
    fn from(raw: RawSettings) -> Self {
        let named = raw.provider.as_deref().and_then(ProviderId::from_id);
        let mut settings = Self {
            provider: named.unwrap_or_else(default_provider),
            chosen: named.is_some(),
            models: raw.models,
            agent_summary: raw.agent_summary,
            enabled: raw.enabled,
            fallback: Vec::new(),
            usage_limit: raw.usage_limit,
        };
        for entry in raw.fallback {
            if let Some(entry_provider) = ProviderId::from_id(&entry.provider) {
                // A file edited by hand can name the same agent twice or the
                // one that already answers first; both read as the list the
                // operator can build in Settings.
                let _ = settings.add_fallback(entry_provider, entry.model);
            }
        }
        settings
    }
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            provider: default_provider(),
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
            agent_summary: on(),
            enabled: on(),
            fallback: Vec::new(),
            usage_limit: UsageLimit::default(),
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

impl AiSettings {
    /// The model this provider will be asked for, whether or not the file
    /// named one.
    pub fn model(&self, provider: ProviderId) -> &str {
        self.models
            .get(provider.as_str())
            .map(String::as_str)
            .unwrap_or_else(|| default_model(provider))
    }

    pub fn set_model(&mut self, provider: ProviderId, model: impl Into<String>) {
        self.models
            .insert(provider.as_str().to_owned(), model.into());
    }

    /// The model each provider's backend is built with: a fallback agent with
    /// its own entry uses that model, everything else the model chosen for
    /// the provider. One value per registered provider, so a router built
    /// from it can also be asked about agents that are not in use.
    pub fn models_by_provider(&self) -> BTreeMap<ProviderId, String> {
        PROVIDERS
            .iter()
            .map(|provider| (*provider, self.effective_model(*provider).to_owned()))
            .collect()
    }

    fn effective_model(&self, provider: ProviderId) -> &str {
        if provider != self.provider
            && let Some(model) = self
                .fallback
                .iter()
                .find(|entry| entry.provider == provider)
                .and_then(|entry| entry.model.as_deref())
        {
            return model;
        }
        self.model(provider)
    }

    /// Makes `provider` the agent that answers (Runs on). An agent that was
    /// in the fallback list leaves it, and the model chosen for it there
    /// becomes its model, because asking the same agent twice is no fallback
    /// and the operator's choice of model should not be lost with the entry
    /// (D-27, B44).
    pub fn set_provider(&mut self, provider: ProviderId) {
        if let Some(position) = self
            .fallback
            .iter()
            .position(|entry| entry.provider == provider)
        {
            let entry = self.fallback.remove(position);
            if let Some(model) = entry.model {
                self.set_model(provider, model);
            }
        }
        self.provider = provider;
        self.chosen = true;
    }

    /// Adds an agent at the end of the fallback list; order is the order of
    /// adding (D-16).
    pub fn add_fallback(
        &mut self,
        provider: ProviderId,
        model: Option<String>,
    ) -> Result<(), FallbackRefusal> {
        if provider == self.provider {
            return Err(FallbackRefusal::IsRunsOn);
        }
        if self.fallback.iter().any(|entry| entry.provider == provider) {
            return Err(FallbackRefusal::AlreadyListed);
        }
        self.fallback.push(FallbackEntry { provider, model });
        Ok(())
    }

    pub fn remove_fallback(&mut self, provider: ProviderId) {
        self.fallback.retain(|entry| entry.provider != provider);
    }

    /// Sets the model of one fallback entry; `None` returns it to the
    /// provider's default. An agent that is not listed is left alone.
    pub fn set_fallback_model(&mut self, provider: ProviderId, model: Option<String>) {
        if let Some(entry) = self
            .fallback
            .iter_mut()
            .find(|entry| entry.provider == provider)
        {
            entry.model = model;
        }
    }

    /// The router policy this choice produces: the chosen agent first, then
    /// the fallback list in the order it was added, and nothing else. An agent
    /// that is in neither is never asked (D-16), however many are installed.
    ///
    /// Everything else about the policy stays the default: a chosen agent
    /// that cannot answer still steps aside to the next one, and that move is
    /// still sticky, because the retry and cooldown constants are the
    /// defaults.
    pub fn router_config(&self) -> RouterConfig {
        let mut priority = vec![self.provider];
        priority.extend(self.fallback.iter().map(|entry| entry.provider));
        RouterConfig {
            priority,
            // An unchosen settings value has no agent to ask (B47).
            enabled: self.enabled && self.chosen,
            ..RouterConfig::default()
        }
    }

    /// Whether a router built for `other` asks the same agents, in the same
    /// order, for the same models as one built for this value. Whoever holds
    /// a router across a settings change compares with this, so a removed
    /// fallback agent stops receiving text and an added one is tried.
    pub fn routes_like(&self, other: &Self) -> bool {
        self.router_config().priority == other.router_config().priority
            && self.models_by_provider() == other.models_by_provider()
    }

    /// The agent a first run should start on: the first of the fixed order
    /// that can answer, `None` when none can (D-18). `availability` is what
    /// the router reported; an agent missing from it cannot answer.
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
        assert_eq!(settings.provider, ProviderId::CLAUDE);
        assert!(
            settings.enabled,
            "Hide AI is on until the operator turns it off"
        );
        assert!(
            settings.fallback.is_empty(),
            "nothing falls back by default"
        );
        assert_eq!(
            settings.model(ProviderId::CODEX),
            crate::codex::DEFAULT_MODEL
        );
        assert_eq!(
            settings.model(ProviderId::CLAUDE),
            crate::claude::DEFAULT_MODEL
        );
        assert_eq!(
            settings.model(ProviderId::GROK),
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
        assert_eq!(settings.provider, ProviderId::CLAUDE);
        assert!(
            settings.agent_summary,
            "a file written before the switch existed leaves summaries on"
        );
        assert!(
            settings.enabled,
            "a file written before the switch existed leaves Hide AI on"
        );
        assert_eq!(
            settings.model(ProviderId::CLAUDE),
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
        assert_eq!(settings.provider, ProviderId::CODEX);
        assert_eq!(settings.model(ProviderId::CLAUDE), "haiku");
        assert!(!settings.agent_summary);
        assert!(settings.enabled);
        assert!(settings.fallback.is_empty());
        assert_eq!(
            settings.router_config().priority,
            vec![ProviderId::CODEX],
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
        assert_eq!(settings.provider, ProviderId::CLAUDE);
        assert!(
            !settings.chosen,
            "a choice naming an agent this build does not know is no choice"
        );
        assert_eq!(
            settings.fallback,
            vec![FallbackEntry {
                provider: ProviderId::CODEX,
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
            assert_eq!(settings.provider, ProviderId::CLAUDE);
            assert!(!settings.chosen);
            assert!(!settings.router_config().enabled);
            assert_eq!(
                settings.router_config().priority,
                [ProviderId::CLAUDE, ProviderId::CODEX]
            );
            assert_eq!(settings.fallback.len(), 1);
            assert_eq!(settings.fallback[0].provider, ProviderId::CODEX);
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
            provider: ProviderId::CLAUDE,
            ..AiSettings::default()
        };
        settings.set_model(ProviderId::CLAUDE, "sonnet");
        settings.agent_summary = false;
        settings.enabled = false;
        settings
            .add_fallback(ProviderId::CODEX, Some("gpt-5.6-luna".to_owned()))
            .unwrap();
        settings.add_fallback(ProviderId::PI, None).unwrap();
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
            provider: ProviderId::CLAUDE,
            chosen: true,
            ..AiSettings::default()
        };
        assert_eq!(
            settings.router_config().priority,
            vec![ProviderId::CLAUDE],
            "an empty fallback list means no fallback, however many agents are installed"
        );
        settings.add_fallback(ProviderId::PI, None).unwrap();
        settings.add_fallback(ProviderId::CODEX, None).unwrap();
        let config = settings.router_config();
        assert_eq!(
            config.priority,
            vec![ProviderId::CLAUDE, ProviderId::PI, ProviderId::CODEX],
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
        settings.add_fallback(ProviderId::CODEX, None).unwrap();
        settings.enabled = false;
        let config = settings.router_config();
        assert!(!config.enabled);
        assert_eq!(config.priority, vec![ProviderId::CLAUDE, ProviderId::CODEX]);
    }

    #[test]
    fn the_agent_that_answers_first_is_never_its_own_fallback_and_one_agent_is_listed_once() {
        let mut settings = AiSettings::default();
        assert_eq!(
            settings.add_fallback(ProviderId::CLAUDE, None),
            Err(FallbackRefusal::IsRunsOn)
        );
        settings.add_fallback(ProviderId::CODEX, None).unwrap();
        assert_eq!(
            settings.add_fallback(ProviderId::CODEX, None),
            Err(FallbackRefusal::AlreadyListed)
        );
        settings.remove_fallback(ProviderId::CODEX);
        assert!(settings.fallback.is_empty());
    }

    /// B44: choosing a fallback agent as Runs on takes it out of the list, and
    /// the model the operator picked for it there is not lost.
    #[test]
    fn choosing_a_listed_agent_as_runs_on_takes_it_out_of_the_list() {
        let mut settings = AiSettings::default();
        settings
            .add_fallback(ProviderId::CODEX, Some("gpt-x".to_owned()))
            .unwrap();
        settings.add_fallback(ProviderId::PI, None).unwrap();
        settings.set_provider(ProviderId::CODEX);
        assert_eq!(settings.provider, ProviderId::CODEX);
        assert_eq!(
            settings.fallback,
            vec![FallbackEntry {
                provider: ProviderId::PI,
                model: None
            }]
        );
        assert_eq!(settings.model(ProviderId::CODEX), "gpt-x");
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
                provider: ProviderId::PI,
                model: None
            }]
        );
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
    }

    #[test]
    fn a_fallback_agent_is_built_with_its_own_model() {
        let mut settings = AiSettings::default();
        settings.set_model(ProviderId::CODEX, "gpt-chosen");
        settings
            .add_fallback(ProviderId::CODEX, Some("gpt-fallback".to_owned()))
            .unwrap();
        let models = settings.models_by_provider();
        assert_eq!(models[&ProviderId::CODEX], "gpt-fallback");
        assert_eq!(models[&ProviderId::CLAUDE], crate::claude::DEFAULT_MODEL);
        assert_eq!(models[&ProviderId::PI], "");
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
        chosen.set_provider(ProviderId::CODEX);
        assert!(chosen.chosen && chosen.router_config().enabled);
        save(&home, &chosen).expect("the file is writable");
        assert!(
            load(&home).unwrap().chosen,
            "a saved choice reads as chosen"
        );
        std::fs::remove_dir_all(home).expect("the temporary home is removable");
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
}
