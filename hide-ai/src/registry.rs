//! The agents Hide AI can name, and how each one's backend is built.
//!
//! A provider is one row here: its stable id, the name a person reads, the
//! agent id the logos and the kit key it by, whether its sign-in can be checked
//! without a request, and the model it is asked for when nobody chose one.
//! Where its CLI is found is not kept here: that is `hide_platform::programs`,
//! the one search the install kit uses too. Nothing else in the
//! workspace keeps a list of them; settings, the router and the core's
//! Settings reader all iterate this one, in this order (D-06, D-18): Claude
//! Code, Codex, Gemini CLI, Grok, OpenCode, Pi, Oh My Pi, Cursor, Antigravity.
//!
//! The id is a string the registry owns, not a closed enum, so a file written
//! by a newer Hide that names an agent this build does not know is read, and
//! the entry is ignored, instead of making the whole file unreadable.

use std::fmt;
use std::sync::Arc;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use crate::{AiBackend, AiLogSink};

/// Stable provider name; also the log field value and the settings key.
///
/// Only the registry makes one, so every value is a registered agent.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProviderId(&'static str);

impl ProviderId {
    /// Claude Code. The persisted id predates the registry and stays
    /// `claude`, so a file an earlier Hide wrote reads unchanged.
    pub const CLAUDE: Self = Self("claude");
    pub const CODEX: Self = Self("codex");
    pub const GEMINI: Self = Self("gemini-cli");
    pub const GROK: Self = Self("grok");
    pub const OPENCODE: Self = Self("opencode");
    pub const PI: Self = Self("pi");
    pub const OMP: Self = Self("omp");
    pub const CURSOR: Self = Self("cursor");
    pub const ANTIGRAVITY: Self = Self("antigravity");

    pub fn as_str(self) -> &'static str {
        self.0
    }

    /// The provider's name as a person reads it. The shell renders this
    /// rather than capitalising `as_str` itself.
    pub fn label(self) -> &'static str {
        self.descriptor().label
    }

    /// The registered provider with this id, `None` for an id this build does
    /// not know.
    pub fn from_id(id: &str) -> Option<Self> {
        PROVIDERS.iter().copied().find(|provider| provider.0 == id)
    }

    /// The provider that runs the install kit's agent `agent` (its adapter
    /// id, [`ProviderDescriptor::agent`]), `None` for an agent Hide AI does
    /// not run. The ids differ where the kit and Herdr name an agent apart,
    /// as `antigravity` and `agy` do.
    pub fn for_agent(agent: &str) -> Option<Self> {
        PROVIDERS
            .iter()
            .copied()
            .find(|provider| provider.descriptor().agent == agent)
    }

    /// Whether this agent's sign-in can be checked without a request
    /// ([`ProviderDescriptor::login_probe`]).
    pub fn login_probe(self) -> bool {
        self.descriptor().login_probe
    }

    pub fn descriptor(self) -> &'static ProviderDescriptor {
        DESCRIPTORS
            .iter()
            .find(|descriptor| descriptor.id == self)
            .expect("every ProviderId constant has a descriptor")
    }

    /// The model this provider is asked for when the operator chose none:
    /// the backend's own constant, or [`CLI_DEFAULT_MODEL`] for an agent whose
    /// default is its own CLI's (D-18).
    pub fn default_model(self) -> &'static str {
        self.descriptor().default_model
    }

    /// The reasoning efforts a request may pick for this agent.
    pub fn efforts(self) -> &'static [&'static str] {
        self.descriptor().efforts
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Serialize for ProviderId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.0)
    }
}

impl<'de> Deserialize<'de> for ProviderId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let id = String::deserialize(deserializer)?;
        Self::from_id(&id).ok_or_else(|| de::Error::custom(format!("unknown provider {id}")))
    }
}

/// The model value that sends no `--model` at all, so the CLI answers with
/// its own default (D-18, B36).
pub const CLI_DEFAULT_MODEL: &str = "";

/// Every provider a choice can name, in the fixed order they are offered.
///
/// The Settings picker, the first-run default and the Add menu all use this
/// order; the router's own order is the operator's choice followed by the
/// fallback list, never this one.
pub const PROVIDERS: &[ProviderId] = &[
    ProviderId::CLAUDE,
    ProviderId::CODEX,
    ProviderId::GEMINI,
    ProviderId::GROK,
    ProviderId::OPENCODE,
    ProviderId::PI,
    ProviderId::OMP,
    ProviderId::CURSOR,
    ProviderId::ANTIGRAVITY,
];

/// What the registry knows about one provider besides its backend.
#[derive(Debug)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
    pub label: &'static str,
    /// The id of the same agent in the install kit's adapter table, which is
    /// how the shell joins "this agent is on and installed" to "Hide AI can
    /// use it". `gemini-cli` is the one exception: the kit no longer lists
    /// Gemini CLI, so for it the id only names its logo.
    pub agent: &'static str,
    /// Whether the CLI can say it is signed in without making a request. An
    /// agent that cannot (Gemini CLI) is `ready` only because its program was
    /// found: it is never chosen by itself, and a sign-in failure on its first
    /// request is remembered for the cooldown, not re-tried on every probe.
    pub login_probe: bool,
    pub default_model: &'static str,
    /// The reasoning efforts a request may pick for this agent; empty means
    /// the agent takes none and only its CLI default runs.
    pub efforts: &'static [&'static str],
}

const DESCRIPTORS: &[ProviderDescriptor] = &[
    ProviderDescriptor {
        id: ProviderId::CLAUDE,
        label: "Claude Code",
        agent: "claude-code",
        login_probe: true,
        default_model: crate::claude::DEFAULT_MODEL,
        efforts: crate::claude::EFFORTS,
    },
    ProviderDescriptor {
        id: ProviderId::CODEX,
        label: "Codex",
        agent: "codex",
        login_probe: true,
        default_model: crate::codex::DEFAULT_MODEL,
        efforts: crate::codex::EFFORTS,
    },
    ProviderDescriptor {
        id: ProviderId::GEMINI,
        label: "Gemini CLI",
        agent: "gemini-cli",
        login_probe: false,
        default_model: CLI_DEFAULT_MODEL,
        efforts: &[],
    },
    ProviderDescriptor {
        id: ProviderId::GROK,
        label: "Grok",
        agent: "grok",
        login_probe: true,
        default_model: CLI_DEFAULT_MODEL,
        efforts: &[],
    },
    ProviderDescriptor {
        id: ProviderId::OPENCODE,
        label: "OpenCode",
        agent: "opencode",
        login_probe: true,
        default_model: CLI_DEFAULT_MODEL,
        efforts: &[],
    },
    ProviderDescriptor {
        id: ProviderId::PI,
        label: "Pi",
        agent: "pi",
        login_probe: true,
        default_model: CLI_DEFAULT_MODEL,
        efforts: &[],
    },
    ProviderDescriptor {
        id: ProviderId::OMP,
        label: "Oh My Pi",
        agent: "omp",
        login_probe: true,
        default_model: crate::omp::DEFAULT_MODEL,
        efforts: &[],
    },
    ProviderDescriptor {
        id: ProviderId::CURSOR,
        label: "Cursor",
        agent: "cursor",
        login_probe: true,
        default_model: CLI_DEFAULT_MODEL,
        efforts: &[],
    },
    ProviderDescriptor {
        id: ProviderId::ANTIGRAVITY,
        label: "Antigravity",
        agent: "antigravity",
        login_probe: true,
        default_model: CLI_DEFAULT_MODEL,
        efforts: &[],
    },
];

/// Builds the backend for one registered provider, configured with the model
/// the operator chose for it. This is the only place a backend is chosen by
/// provider, so a new agent is a row above and an arm here.
pub fn build_backend(
    provider: ProviderId,
    model: &str,
    sink: Arc<dyn AiLogSink>,
) -> Arc<dyn AiBackend> {
    match provider {
        ProviderId::CLAUDE => Arc::new(crate::ClaudeCliBackend::new(crate::ClaudeConfig {
            model: model.to_owned(),
            ..crate::ClaudeConfig::default()
        })),
        ProviderId::CODEX => Arc::new(crate::CodexAppServerBackend::new(
            crate::CodexConfig {
                model: model.to_owned(),
                ..crate::CodexConfig::default()
            },
            sink,
        )),
        ProviderId::GEMINI => Arc::new(crate::GeminiCliBackend::new(crate::TextCliConfig::new(
            model,
        ))),
        ProviderId::GROK => Arc::new(crate::GrokCliBackend::new(crate::TextCliConfig::new(model))),
        ProviderId::PI => Arc::new(crate::PiCliBackend::new(crate::TextCliConfig::new(model))),
        ProviderId::OMP => Arc::new(crate::OmpSdkBackend::new(crate::OmpConfig::new(model))),
        ProviderId::ANTIGRAVITY => Arc::new(crate::AntigravityCliBackend::new(
            crate::TextCliConfig::new(model),
        )),
        other => Arc::new(crate::UnprovenReadOnlyBackend::new(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_persisted_ids_of_the_first_two_providers_are_the_ones_files_already_carry() {
        assert_eq!(ProviderId::CLAUDE.as_str(), "claude");
        assert_eq!(ProviderId::CODEX.as_str(), "codex");
        assert_eq!(ProviderId::from_id("claude"), Some(ProviderId::CLAUDE));
        assert_eq!(ProviderId::from_id("codex"), Some(ProviderId::CODEX));
    }

    #[test]
    fn the_fixed_order_is_the_order_the_pickers_offer() {
        let labels: Vec<_> = PROVIDERS.iter().map(|provider| provider.label()).collect();
        assert_eq!(
            labels,
            [
                "Claude Code",
                "Codex",
                "Gemini CLI",
                "Grok",
                "OpenCode",
                "Pi",
                "Oh My Pi",
                "Cursor",
                "Antigravity"
            ]
        );
    }

    #[test]
    fn an_unknown_id_is_not_a_provider() {
        assert_eq!(ProviderId::from_id("some-future-agent"), None);
        assert!(serde_json::from_str::<ProviderId>("\"some-future-agent\"").is_err());
        assert_eq!(
            serde_json::to_string(&ProviderId::GROK).unwrap(),
            "\"grok\""
        );
    }

    #[test]
    fn every_provider_has_a_descriptor_whose_id_matches() {
        for provider in PROVIDERS {
            assert_eq!(provider.descriptor().id, *provider);
        }
    }
}
