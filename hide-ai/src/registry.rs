//! The agents Hide AI can name, and how each one's backend is built.
//!
//! Which agents are providers, in which order, under which persisted id, with
//! which name and reasoning efforts, is each agent's declaration
//! (`hide_agent_adapter::AgentAdapter::ai`, `label`, `efforts`); an agent
//! that is not a provider declares why. This file binds a declared provider
//! to what only Hide AI decides: the model it is asked for when nobody chose
//! one ([`BINDINGS`]) and its backend ([`build_backend`]).
//! Where its CLI is found is not kept here: that is `hide_platform::programs`,
//! the one search the install kit uses too. Settings, the router and the
//! core's Settings reader all iterate [`PROVIDERS`], in the declarations'
//! order (D-06, D-18): Claude Code, Codex, Grok, OpenCode, Pi, omp,
//! Antigravity.
//!
//! The id is a string the declaration owns, not a closed enum, so a file
//! written by a newer Hide that names an agent this build does not know is
//! read, and the entry is ignored, instead of making the whole file
//! unreadable.

use std::fmt;
use std::sync::Arc;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use hide_agent_adapter::{ADAPTERS, AgentAdapter, usage::Support};

use crate::{AiBackend, AiLogSink};

/// Stable provider name; also the log field value and the settings key.
///
/// Only the registry makes one, so every value is a declared provider.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProviderId(&'static str);

impl ProviderId {
    /// Claude Code. The persisted id predates the registry and stays
    /// `claude`, so a file an earlier Hide wrote reads unchanged.
    pub const CLAUDE: Self = Self("claude");
    pub const CODEX: Self = Self("codex");
    pub const GROK: Self = Self("grok");
    pub const OPENCODE: Self = Self("opencode");
    pub const PI: Self = Self("pi");
    pub const OMP: Self = Self("omp");
    pub const ANTIGRAVITY: Self = Self("antigravity");

    pub fn as_str(self) -> &'static str {
        self.0
    }

    /// The provider's name as a person reads it: its agent's declared label.
    /// The shell renders this rather than capitalising `as_str` itself.
    pub fn label(self) -> &'static str {
        self.adapter().label
    }

    /// The registered provider with this id, `None` for an id this build does
    /// not know.
    pub fn from_id(id: &str) -> Option<Self> {
        PROVIDERS.iter().copied().find(|provider| provider.0 == id)
    }

    /// The provider that runs the install kit's agent `agent` (its adapter
    /// id, [`ProviderId::agent`]), `None` for an agent Hide AI does not run.
    /// The ids differ where Hide AI's settings name an agent apart, as
    /// `claude` and `claude-code` do.
    pub fn for_agent(agent: &str) -> Option<Self> {
        PROVIDERS
            .iter()
            .copied()
            .find(|provider| provider.agent() == agent)
    }

    /// The agent's declaration this provider was made from.
    pub fn adapter(self) -> &'static AgentAdapter {
        ADAPTERS
            .iter()
            .find(|row| matches!(row.ai, Support::Supported(ai) if ai.id == self.0))
            .expect("every ProviderId is a declared provider")
    }

    /// The agent's id in the install kit's adapter table, which is how the
    /// shell joins "this agent is on and installed" to "Hide AI can use it".
    pub fn agent(self) -> &'static str {
        self.adapter().id
    }

    /// The model this provider is asked for when the operator chose none:
    /// the backend's own constant, or [`CLI_DEFAULT_MODEL`] for an agent whose
    /// default is its own CLI's (D-18).
    pub fn default_model(self) -> &'static str {
        self.binding().default_model
    }

    /// The reasoning efforts a request may pick for this agent: the menu its
    /// declaration offers every start (`hide_agent_adapter::LaunchDialect::options`);
    /// empty means the agent takes none and only its CLI default runs.
    pub fn efforts(self) -> &'static [&'static str] {
        self.adapter().efforts()
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

const fn provider_count() -> usize {
    let mut count = 0;
    let mut index = 0;
    while index < ADAPTERS.len() {
        if matches!(ADAPTERS[index].ai, Support::Supported(_)) {
            count += 1;
        }
        index += 1;
    }
    count
}

const fn providers() -> [ProviderId; provider_count()] {
    let mut result = [ProviderId(""); provider_count()];
    let mut source = 0;
    let mut destination = 0;
    while source < ADAPTERS.len() {
        if let Support::Supported(ai) = ADAPTERS[source].ai {
            result[destination] = ProviderId(ai.id);
            destination += 1;
        }
        source += 1;
    }
    result
}

const DECLARED: [ProviderId; provider_count()] = providers();

/// Every provider a choice can name, in the fixed order they are offered:
/// each agent that declares a Hide AI provider, in the declarations' order.
///
/// The Settings picker, the first-run default and the Add menu all use this
/// order; the router's own order is the operator's choice followed by the
/// fallback list, never this one.
pub const PROVIDERS: &[ProviderId] = &DECLARED;

/// The backend that runs a declared provider whose declaration lets Hide AI
/// send it requests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Backend {
    Claude,
    Codex,
    Grok,
    Pi,
    Omp,
    Antigravity,
}

/// What only Hide AI decides for one declared provider: the model it is
/// asked for when nobody chose one, and its backend, `None` exactly when its
/// declaration refuses requests (`AiProvider::requests`). A provider declared
/// without its row, or with a backend its declaration does not allow, fails
/// `every_declared_provider_is_bound_and_nothing_else_is`.
struct Binding {
    provider: ProviderId,
    default_model: &'static str,
    backend: Option<Backend>,
}

const BINDINGS: &[Binding] = &[
    Binding {
        provider: ProviderId::CLAUDE,
        default_model: crate::claude::DEFAULT_MODEL,
        backend: Some(Backend::Claude),
    },
    Binding {
        provider: ProviderId::CODEX,
        default_model: crate::codex::DEFAULT_MODEL,
        backend: Some(Backend::Codex),
    },
    Binding {
        provider: ProviderId::GROK,
        default_model: CLI_DEFAULT_MODEL,
        backend: Some(Backend::Grok),
    },
    Binding {
        provider: ProviderId::OPENCODE,
        default_model: CLI_DEFAULT_MODEL,
        backend: None,
    },
    Binding {
        provider: ProviderId::PI,
        default_model: CLI_DEFAULT_MODEL,
        backend: Some(Backend::Pi),
    },
    Binding {
        provider: ProviderId::OMP,
        default_model: crate::omp::DEFAULT_MODEL,
        backend: Some(Backend::Omp),
    },
    Binding {
        provider: ProviderId::ANTIGRAVITY,
        default_model: CLI_DEFAULT_MODEL,
        backend: Some(Backend::Antigravity),
    },
];

impl ProviderId {
    fn binding(self) -> &'static Binding {
        BINDINGS
            .iter()
            .find(|binding| binding.provider == self)
            .expect("every declared provider has a binding")
    }
}

/// Builds the backend for one registered provider, configured with the model
/// the operator chose for it. This is the only place a backend is chosen, by
/// the provider's binding: a provider whose declaration refuses requests gets
/// the backend that refuses them with its reason.
pub fn build_backend(
    provider: ProviderId,
    model: &str,
    sink: Arc<dyn AiLogSink>,
) -> Arc<dyn AiBackend> {
    let Some(backend) = provider.binding().backend else {
        return Arc::new(crate::UnprovenReadOnlyBackend::new(provider));
    };
    match backend {
        Backend::Claude => Arc::new(crate::ClaudeCliBackend::new(crate::ClaudeConfig {
            model: model.to_owned(),
            ..crate::ClaudeConfig::default()
        })),
        Backend::Codex => Arc::new(crate::CodexAppServerBackend::new(
            crate::CodexConfig {
                model: model.to_owned(),
                ..crate::CodexConfig::default()
            },
            sink,
        )),
        Backend::Grok => Arc::new(crate::GrokCliBackend::new(crate::TextCliConfig::new(model))),
        Backend::Pi => Arc::new(crate::PiCliBackend::new(crate::TextCliConfig::new(model))),
        Backend::Omp => Arc::new(crate::OmpSdkBackend::new(crate::OmpConfig::new(model))),
        Backend::Antigravity => Arc::new(crate::AntigravityCliBackend::new(
            crate::TextCliConfig::new(model),
        )),
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
                "Grok",
                "OpenCode",
                "Pi",
                "omp",
                "Antigravity"
            ]
        );
    }

    #[test]
    fn an_unknown_id_is_not_a_provider() {
        for retired in ["gemini-cli", "gemini", "cursor"] {
            assert_eq!(ProviderId::from_id(retired), None);
            assert_eq!(ProviderId::for_agent(retired), None);
            assert!(serde_json::from_value::<ProviderId>(serde_json::json!(retired)).is_err());
        }
        assert_eq!(ProviderId::from_id("some-future-agent"), None);
        assert!(serde_json::from_str::<ProviderId>("\"some-future-agent\"").is_err());
        assert_eq!(
            serde_json::to_string(&ProviderId::GROK).unwrap(),
            "\"grok\""
        );
    }

    #[test]
    fn every_declared_provider_is_bound_and_nothing_else_is() {
        let mut bound: Vec<_> = BINDINGS.iter().map(|binding| binding.provider).collect();
        let mut declared = PROVIDERS.to_vec();
        bound.sort();
        declared.sort();
        assert_eq!(
            bound, declared,
            "each agent declaring a Hide AI provider needs its row in BINDINGS"
        );
        for provider in PROVIDERS {
            let Support::Supported(ai) = provider.adapter().ai else {
                panic!("{provider} is listed without a declared provider");
            };
            assert_eq!(ai.id, provider.as_str());
            assert_eq!(ProviderId::for_agent(provider.agent()), Some(*provider));
            assert_eq!(
                provider.binding().backend.is_some(),
                matches!(ai.requests, Support::Supported(())),
                "{provider}: a backend exactly when its declaration lets Hide AI send it requests"
            );
        }
    }

    /// What a caller sees of a provider whose declaration refuses requests,
    /// without starting anything: listed, unsupported with the reason the
    /// shell has a sentence for, and refusing a request.
    #[test]
    fn a_provider_whose_declaration_refuses_requests_is_listed_and_refuses_them() {
        let refused: Vec<_> = PROVIDERS
            .iter()
            .copied()
            .filter(|provider| provider.binding().backend.is_none())
            .collect();
        assert_eq!(refused, [ProviderId::OPENCODE]);
        for provider in refused {
            let backend = build_backend(provider, CLI_DEFAULT_MODEL, Arc::new(crate::NoopLogSink));
            assert_eq!(backend.id(), provider);
            assert_eq!(
                backend.availability(),
                crate::Availability::Unsupported {
                    reason: crate::unproven::CANNOT_GUARANTEE_READ_ONLY.to_owned()
                }
            );
        }
    }

    #[test]
    fn a_request_picks_from_the_efforts_every_start_of_the_agent_offers() {
        for provider in PROVIDERS {
            let adapter = provider.adapter();
            assert_eq!(
                provider.efforts(),
                adapter
                    .start
                    .map_or(&[][..], |start| start.options().efforts)
            );
        }
        assert_eq!(
            ProviderId::CODEX.efforts(),
            ["minimal", "low", "medium", "high", "xhigh"]
        );
        assert_eq!(
            ProviderId::CLAUDE.efforts(),
            ["low", "medium", "high", "xhigh", "max"]
        );
    }
}
