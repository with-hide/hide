use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use crate::ProviderId;

/// One structured log line. Identifiers and classes only: never the prompt,
/// the input, the generated text, a token, or a provider thread id. It
/// serializes as a flat JSON object without its absent fields.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct AiLogEvent {
    #[serde(deserialize_with = "crate::owned::cow")]
    pub event: Cow<'static, str>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::owned::optional_cow"
    )]
    pub feature_id: Option<Cow<'static, str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderId>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::owned::optional_cow"
    )]
    pub outcome_class: Option<Cow<'static, str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_chars: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    /// Thinking tokens inside `output_tokens`, present only when the provider
    /// reported some.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempt: Option<u8>,
    /// The reasoning effort an attempt asked its provider for, when it
    /// asked one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::owned::optional_cow"
    )]
    pub schema_version: Option<Cow<'static, str>>,
    /// The resident app-server's pid, and the process measurement taken after
    /// the turn. Present on codex requests; `descendants`/`rss_bytes` are
    /// `None` when the platform cannot measure, and the detail says
    /// `measurement=unavailable` rather than logging a zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_server_pid: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descendants: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rss_bytes: Option<u64>,
    /// Short machine-readable detail such as `from=codex;to=claude`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AiLogEvent {
    pub fn new(event: &'static str) -> Self {
        Self {
            event: Cow::Borrowed(event),
            request_id: None,
            feature_id: None,
            provider: None,
            outcome_class: None,
            duration_ms: None,
            input_chars: None,
            output_tokens: None,
            thinking_tokens: None,
            attempt: None,
            effort: None,
            schema_version: None,
            app_server_pid: None,
            descendants: None,
            rss_bytes: None,
            detail: None,
        }
    }
}

pub trait AiLogSink: Send + Sync {
    fn log(&self, event: AiLogEvent);
}

pub struct NoopLogSink;

impl AiLogSink for NoopLogSink {
    fn log(&self, _: AiLogEvent) {}
}
