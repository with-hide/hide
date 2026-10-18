//! Provider boundary for Hide's background AI features.
//!
//! A feature owns its prompt, output schema and parser and submits an
//! [`AiRequest`]; this crate owns provider lifecycle, availability, timeouts,
//! cancellation, structured errors and duplicate suppression, and hands back
//! a schema-validated [`serde_json::Value`]. No feature type lives here.

mod antigravity;
mod claude;
mod codex;
mod codex_home;
mod gemini;
mod grok;
mod log;
mod pi;
mod process;
mod program;
mod registry;
mod router;
mod runner;
mod schema;
pub mod settings;
mod text_cli;
mod unproven;
mod usage_limit;

pub use antigravity::{
    AntigravityCliBackend, READ_ONLY_HOOKS, read_only_hooks_file, read_only_prepared,
    read_only_workspace, trusts as antigravity_trusts,
};
pub use claude::{ClaudeCliBackend, ClaudeConfig, USAGE_ENVIRONMENT, UsageError};
pub use codex::{CodexAppServerBackend, CodexConfig};
pub use gemini::GeminiCliBackend;
pub use grok::GrokCliBackend;
pub use log::{AiLogEvent, AiLogSink, NoopLogSink};
pub use pi::PiCliBackend;
pub use process::ProcessMeasurement;
pub use program::resolve_binary;
pub use registry::{CLI_DEFAULT_MODEL, PROVIDERS, ProviderDescriptor, ProviderId, build_backend};
pub use router::{AiRouter, Degraded, ProviderState, ProviderStatus, RouterConfig};
pub use settings::{AiSettings, FallbackEntry, FallbackRefusal};
pub use text_cli::TextCliConfig;
pub use unproven::UnprovenReadOnlyBackend;
pub use usage_limit::{DEFAULT_LIMIT_PERCENT, PastLimit, UsageLimit, UsageLimits, UsageReading};

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What models a provider offers, or why that is not known.
///
/// A provider that cannot be asked reports the reason rather than an empty
/// list, because an empty list and an unanswered question look the same to a
/// menu and only one of them is a real answer.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum ModelCatalog {
    /// The list the provider's own CLI or account answered.
    Offered(Vec<String>),
    /// A list the CLI documents but cannot be asked for (Gemini CLI's
    /// aliases). The shell says so beside the menu instead of presenting it
    /// as the account's own list (B36).
    Fixed(Vec<String>),
    Unknown {
        reason: String,
    },
}

impl ModelCatalog {
    pub fn offered(&self) -> &[String] {
        match self {
            Self::Offered(models) | Self::Fixed(models) => models,
            Self::Unknown { .. } => &[],
        }
    }

    pub fn is_fixed(&self) -> bool {
        matches!(self, Self::Fixed(_))
    }

    pub fn unknown_reason(&self) -> Option<&str> {
        match self {
            Self::Offered(_) | Self::Fixed(_) => None,
            Self::Unknown { reason } => Some(reason),
        }
    }
}

/// Caller-generated idempotency key. It travels to the provider as the client
/// message id and is the only identifier a log line carries for the request.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(transparent)]
pub struct RequestId(pub String);

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// What a feature submits. Everything a provider needs to answer, and nothing
/// about how any provider works.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AiRequest {
    #[serde(deserialize_with = "owned::cow")]
    pub feature_id: Cow<'static, str>,
    pub request_id: RequestId,
    /// Identifies what the request is about (a pane, a checkout); together
    /// with the feature and the input hash it forms the duplicate key.
    pub subject_id: String,
    pub system: String,
    pub input: String,
    /// The prompt and the schema bound the answer's size; no provider
    /// contract offers a token ceiling, so none is pretended here.
    pub output_schema: Value,
    pub deadline: Duration,
    /// Version of the feature's prompt and schema pair, for the log only.
    #[serde(deserialize_with = "owned::cow")]
    pub schema_version: Cow<'static, str>,
    /// The caller's own agent, model and effort for this request (a
    /// Factory's AI). `None` runs it on the operator's Hide AI order.
    #[serde(default)]
    pub pick: Option<AiPick>,
    /// The language every text a person reads is written in, which
    /// [`AiRequest::system_prompt`] tells the model; `None` only for an
    /// answer that is not prose for a person (a Git name). No default: a
    /// request from a peer that does not send it is refused, so a feature
    /// cannot forget it silently. serde reads a missing `Option` as `None`
    /// unless the field names its reader, which is why it does.
    #[serde(deserialize_with = "Option::<OutputLanguage>::deserialize")]
    pub output_language: Option<OutputLanguage>,
    /// How a provider that has a native schema surface is asked for the
    /// answer's shape. Absent on the wire from an older peer, which asks the
    /// way it always did.
    #[serde(default)]
    pub answer: AnswerMode,
}

/// How a request asks a provider for a JSON answer. The router validates the
/// answer against the feature's schema whichever mode produced it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerMode {
    /// The provider's own schema surface (`claude -p --json-schema`,
    /// Codex's `outputSchema`) constrains the answer. Claude Code does this
    /// by calling a synthetic tool, which is a second model turn and a
    /// larger prompt: the right price for an answer with a long or strict
    /// shape. Codex's surface is the API's strict mode, which refuses
    /// `oneOf` and `uniqueItems`, so a schema using them cannot ask this way
    /// there.
    #[default]
    Schema,
    /// The system prompt names the schema and asks for one JSON object, and
    /// the object is read back from the text; Claude and Codex both honour
    /// it and send no schema of their own. One model turn: the choice for a
    /// short answer whose shape the router checks anyway. For Claude the
    /// schema rides in the system prompt, an argument of the child, so only
    /// a short one belongs here. A provider that has no schema surface
    /// answers this way in either mode.
    PlainJson,
}

/// A language Hide writes in for the operator, the interface's four
/// (docs/LOCALIZATION.md).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub enum OutputLanguage {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "ko")]
    Korean,
    #[serde(rename = "zh-CN")]
    SimplifiedChinese,
    #[serde(rename = "ja")]
    Japanese,
}

impl OutputLanguage {
    pub const ALL: [Self; 4] = [
        Self::English,
        Self::Korean,
        Self::SimplifiedChinese,
        Self::Japanese,
    ];

    /// The language's English name, as an instruction says it; the
    /// instructions themselves stay in one language.
    pub fn english_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Korean => "Korean",
            Self::SimplifiedChinese => "Simplified Chinese",
            Self::Japanese => "Japanese",
        }
    }
}

/// One agent, and optionally its model and reasoning effort, chosen by the
/// caller for one request. The router asks only that agent; an effort the
/// agent does not declare is refused rather than dropped.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
pub struct AiPick {
    pub provider: ProviderId,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
}

impl AiRequest {
    /// The model this request is asked with: the caller's pick, else the one
    /// the backend was configured with.
    pub fn model<'a>(&'a self, configured: &'a str) -> &'a str {
        self.pick
            .as_ref()
            .and_then(|pick| pick.model.as_deref())
            .unwrap_or(configured)
    }

    /// The reasoning effort the caller picked, if any.
    pub fn effort(&self) -> Option<&str> {
        self.pick.as_ref().and_then(|pick| pick.effort.as_deref())
    }

    /// The system prompt every provider is given: the feature's own, then
    /// the one sentence naming the language to write in, so no feature's
    /// prompt names a language of its own.
    pub fn system_prompt(&self) -> Cow<'_, str> {
        match self.output_language {
            None => Cow::Borrowed(&self.system),
            Some(language) => Cow::Owned(format!(
                "{}\n\nWrite every text a person reads in {}, whatever language the input is in; keep code, commands, paths, identifiers, names and quoted text exactly as they are.",
                self.system,
                language.english_name()
            )),
        }
    }
}

/// Provider readiness as the router sees it. Every variant except `Ready`
/// keeps the provider out of selection and is reported to the caller.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Availability {
    Ready,
    NeedsLogin,
    NotInstalled,
    /// The provider exists but cannot answer right now (a broken child
    /// process, a model the account does not offer).
    Unavailable {
        reason: String,
    },
    /// No stable contract exists to drive the provider; the reason names the
    /// gap.
    Unsupported {
        reason: String,
    },
}

impl Availability {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }

    pub fn class(&self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NeedsLogin => "needs_login",
            Self::NotInstalled => "not_installed",
            Self::Unavailable { .. } => "unavailable",
            Self::Unsupported { .. } => "unsupported",
        }
    }
}

/// Failure classes a caller can branch on. Payloads carry diagnostics, never
/// prompt or output content.
///
/// A refusal (`NotAuthenticated`, `UsageLimited`, `ProviderUnavailable`,
/// `Transient`, `Rejected`, `Unsupported`) means no model turn ran, so
/// asking again or asking another provider repeats no model work.
/// `InvalidOutput` may follow a model turn and is retried at most twice;
/// each dispatch is counted even though its answer was unusable.
/// A completion-unknown outcome (`Timeout`, `Cancelled`, `CompletionUnknown`)
/// means the request was submitted and its fate is not known; it is final
/// on that provider and is never re-run anywhere.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub enum AiError {
    /// Not retried until availability changes.
    NotAuthenticated,
    /// Account-global cooldown on that provider.
    UsageLimited {
        retry_after: Option<Duration>,
    },
    /// The provider could not take the request; nothing was submitted.
    ProviderUnavailable(String),
    /// The deadline passed after the request was submitted.
    Timeout,
    Cancelled,
    /// A submitted request ended without a reliable completion, such as a
    /// lost connection or child, or an unclassified failure after turn start.
    CompletionUnknown(String),
    /// Schema mismatch or a message that is not the answer. The backend had
    /// the prompt, so a model turn may have run.
    InvalidOutput(String),
    /// The provider refused the request itself, before any model turn (an
    /// HTTP 4xx, a prompt that does not fit, an input error): nothing was
    /// spent. It is retried and reported as `InvalidOutput` is, and is the
    /// one input failure the usage ledger does not count.
    Rejected(String),
    /// A refusal before model work that may clear on its own.
    Transient(String),
    /// The provider layer itself failed (a panicked request leader).
    Internal(String),
    /// The provider has no stable contract; see `Availability::Unsupported`.
    Unsupported(String),
    /// A resource cap was crossed before the request was submitted (a
    /// concurrency or per-minute limit) or measured after it (the
    /// app-server's descendant or resident-size cap). It is a refusal:
    /// nothing new was submitted, so a retry is safe, but it does not move to
    /// another provider - the cap is this account's, not this provider's
    /// fault. `cap` names which limit; `measured` is the value that crossed
    /// it.
    OverBudget {
        #[serde(deserialize_with = "owned::cow")]
        cap: Cow<'static, str>,
        measured: u64,
    },
    /// No connected provider; carries each provider's availability.
    NoProvider(Vec<(ProviderId, Availability)>),
    /// Use Hide AI is off: no provider was asked anything and nothing was
    /// submitted (B33). Not a failure to report; the caller falls back to its
    /// own non-AI behavior.
    Disabled,
}

impl AiError {
    pub fn class(&self) -> &'static str {
        match self {
            Self::NotAuthenticated => "not_authenticated",
            Self::UsageLimited { .. } => "usage_limited",
            Self::ProviderUnavailable(_) => "provider_unavailable",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::CompletionUnknown(_) => "completion_unknown",
            Self::InvalidOutput(_) | Self::Rejected(_) => "invalid_output",
            Self::Transient(_) => "transient",
            Self::Internal(_) => "internal",
            Self::Unsupported(_) => "unsupported",
            Self::OverBudget { .. } => "over_budget",
            Self::NoProvider(_) => "no_provider",
            Self::Disabled => "disabled",
        }
    }
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UsageLimited { retry_after } => match retry_after {
                Some(wait) => write!(f, "usage_limited;retry_after_s={}", wait.as_secs()),
                None => f.write_str("usage_limited"),
            },
            Self::ProviderUnavailable(reason)
            | Self::CompletionUnknown(reason)
            | Self::InvalidOutput(reason)
            | Self::Rejected(reason)
            | Self::Transient(reason)
            | Self::Internal(reason)
            | Self::Unsupported(reason) => write!(f, "{}:{reason}", self.class()),
            Self::OverBudget { cap, measured } => {
                write!(f, "over_budget:{cap};measured={measured}")
            }
            Self::NoProvider(states) => {
                f.write_str("no_provider")?;
                for (provider, state) in states {
                    write!(f, ";{provider}={}", state.class())?;
                }
                Ok(())
            }
            _ => f.write_str(self.class()),
        }
    }
}

impl std::error::Error for AiError {}

/// Token accounting a provider reports for one answer.
///
/// `input_tokens` and `output_tokens` are what the provider printed first and
/// are for the log only. `models` is what the usage ledger counts.
#[derive(Clone, Debug, Default, Eq, PartialEq, Deserialize, Serialize)]
pub struct AiUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    /// One entry per model that answered. Empty means the provider took the
    /// request and reported no usage the ledger accepts, which the router
    /// hands on as an unknown call rather than as zero tokens. A node on an
    /// older build sends none.
    #[serde(default)]
    pub models: Vec<ModelTokens>,
    /// The thinking tokens the provider reports inside `output_tokens`, for
    /// the log only; `None` when it reports none. Thinking is turned off for
    /// a background turn, so a value above zero is a turn that thought
    /// anyway.
    #[serde(default)]
    pub thinking_tokens: Option<u64>,
}

/// One model's tokens in one answer, in the disjoint components the usage
/// ledger counts: input excludes both cache categories, reasoning is a subset
/// of output and one-hour cache writes a subset of cache writes.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct ModelTokens {
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cache_write_1h: u64,
    pub reasoning: Option<u64>,
    /// The provider did not split this model's cache writes by lifetime and
    /// some may have been one-hour writes, so its price is a lower bound.
    #[serde(default)]
    pub lower_bound: bool,
}

/// Told about every backend attempt that reached a provider, before the
/// answer is validated, so a rejected answer is still counted. A request that
/// joined another's flight is one execution and is told once. Implementations
/// queue and return: the router calls this on the request's own thread.
pub trait AiUsageSink: Send + Sync + fmt::Debug {
    /// `models` is empty for an attempt that was submitted and reported no
    /// usage (a timeout, a lost connection, a cancel or an unusable answer
    /// after the prompt went out, a provider whose usage is not read): an
    /// unknown call.
    fn attempt(&self, feature_id: &str, provider: ProviderId, models: &[ModelTokens]);
}

/// One provider answer before the router validates it.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AiResponse {
    pub value: Value,
    pub usage: AiUsage,
}

/// What the router hands back: the validated answer and which provider
/// produced it, so a fallback is visible to the caller and its log.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AiResult {
    pub provider: ProviderId,
    pub value: Value,
}

/// Cooperative cancellation shared between the caller and the transport.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub trait AiBackend: Send + Sync {
    fn id(&self) -> ProviderId;
    /// Whether this backend owns a resident process whose descendants and
    /// size the router measures after each turn and holds to its caps. The
    /// backend declares it; the router never names a provider for it.
    fn measurable(&self) -> bool {
        false
    }
    fn availability(&self) -> Availability;
    /// Which models this provider offers the logged-in account. Every backend
    /// answers, so nothing above this layer keeps a model list of its own.
    fn models(&self) -> ModelCatalog;
    fn execute(&self, request: &AiRequest, cancel: &CancelToken) -> Result<AiResponse, AiError>;

    /// The process measurement taken during the last [`AiBackend::execute`]:
    /// the resident child's descendant count and size. A backend that owns no
    /// resident process, or one on a platform without the kernel query,
    /// answers `Unavailable`, which the router logs as such rather than as a
    /// zero.
    fn last_measurement(&self) -> ProcessMeasurement {
        ProcessMeasurement::Unavailable
    }

    /// Ends the backend's resident process so the next request starts a fresh
    /// one. The router calls this when the process cap is crossed. A backend
    /// with no resident process does nothing.
    fn restart(&self) {}
}

/// A name that is a literal where it is made and owned where it was read
/// off the wire: serde would otherwise only read one that lives forever.
pub(crate) mod owned {
    use std::borrow::Cow;

    use serde::{Deserialize, Deserializer};

    pub fn cow<'de, D: Deserializer<'de>>(reader: D) -> Result<Cow<'static, str>, D::Error> {
        String::deserialize(reader).map(Cow::Owned)
    }

    pub fn optional_cow<'de, D: Deserializer<'de>>(
        reader: D,
    ) -> Result<Option<Cow<'static, str>>, D::Error> {
        Option::<String>::deserialize(reader).map(|name| name.map(Cow::Owned))
    }
}
