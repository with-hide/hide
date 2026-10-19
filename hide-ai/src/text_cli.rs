//! What the CLIs that answer in text (Grok, Pi, Antigravity) share.
//!
//! None of them enforces an output schema the way Claude Code and Codex do,
//! so each is asked for one JSON object in its system prompt, and the answer
//! is read back as JSON from whatever text it printed. The router re-validates
//! every answer against the feature's schema and turns a mismatch into
//! `InvalidOutput`, so this layer never has to trust the shape.
//!
//! Their failures are classified from the CLI's documented exit codes and the
//! text it prints to stderr. A failure nothing recognises is
//! `CompletionUnknown`: the child ran, so the prompt may have been submitted,
//! and a request whose fate is unknown is never re-sent anywhere (B40).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::program::Program;
use crate::runner::{self, Environment, Run, RunError, Spec};
use crate::{AiError, CancelToken};

/// How the backend of a text-mode CLI is configured.
#[derive(Clone, Debug)]
pub struct TextCliConfig {
    /// The CLI's program name looked up on the login `PATH`, or an explicit
    /// path used as given. `None` takes the registry's name for the agent.
    pub binary: Option<PathBuf>,
    /// The model asked for; empty is the CLI's own default (no `--model`).
    pub model: String,
    /// Where status and model probes run; a request runs in a folder of its
    /// own.
    pub cwd: PathBuf,
    /// The `PATH` the CLI is looked for on and run with. `None` is this
    /// account's search (`hide_platform::programs`); a caller that has to
    /// choose it, such as a test with a stand-in login shell, gives one built
    /// by `hide_platform::programs::cli_path_with`.
    pub search_path: Option<OsString>,
}

impl TextCliConfig {
    pub fn new(model: &str) -> Self {
        Self {
            binary: None,
            model: model.to_owned(),
            cwd: std::env::temp_dir(),
            search_path: None,
        }
    }

    pub(crate) fn resolved(&self, program: &str) -> Option<Program> {
        Program::resolve(
            self.binary.as_deref().unwrap_or_else(|| Path::new(program)),
            self.search_path.as_deref(),
        )
    }
}

/// The bound on a login or model-list probe. These are local reads that make
/// no model turn; the router must not stall on them.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(20);

/// Runs a probe child that makes no model turn.
pub(crate) fn probe(
    binary: &Program,
    args: &[&str],
    cwd: &Path,
    stage: &str,
) -> Result<Run, String> {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    runner::run(
        &Spec {
            binary,
            args: &args,
            cwd,
            stdin: None,
            environment: Environment::Inherit,
            set: &[],
            deadline: PROBE_TIMEOUT,
        },
        &CancelToken::new(),
    )
    .map_err(|error: RunError| error.diagnostic(stage))
}

/// The system prompt a text-mode CLI is given: the feature's own, then the
/// one instruction that makes the answer readable.
pub(crate) fn system_with_schema(system: &str, schema: &Value) -> String {
    format!(
        "{system}\n\nAnswer with exactly one JSON object that matches this JSON Schema, \
         with no prose and no code fence:\n{schema}"
    )
}

/// The JSON a model printed: the whole text, the body of a code fence, or the
/// span from its first `{` to its last `}`, in that order.
pub(crate) fn json_from_text(text: &str) -> Option<Value> {
    let text = text.trim();
    if let Ok(value) = serde_json::from_str::<Value>(text) {
        return Some(value);
    }
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        let body = after.split_once('\n').map_or(after, |(_, body)| body);
        if let Some(end) = body.find("```")
            && let Ok(value) = serde_json::from_str::<Value>(body[..end].trim())
        {
            return Some(value);
        }
    }
    let (first, last) = (text.find('{')?, text.rfind('}')?);
    serde_json::from_str::<Value>(text.get(first..=last)?).ok()
}

/// Whether `text` names the HTTP status `code` as a number of its own, not as
/// a part of a longer one.
fn mentions_status(text: &str, code: u16) -> bool {
    let needle = code.to_string();
    text.match_indices(&needle).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + needle.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_digit()) && !after.is_some_and(|c| c.is_ascii_digit())
    })
}

/// Classifies what a CLI printed when it failed.
///
/// The words are the ones the CLIs' own documentation and the HTTP APIs they
/// call use; a CLI that has been run against a real account refines this in
/// its own module before it falls here. Nothing a failure prints is kept: the
/// returned error carries the provider and the exit status only.
pub(crate) fn classify_failure(provider: &str, code: Option<i32>, stderr: &str) -> AiError {
    let text = stderr.to_lowercase();
    let says = |words: &[&str]| words.iter().any(|word| text.contains(word));
    if mentions_status(&text, 401)
        || mentions_status(&text, 403)
        || says(&[
            "unauthorized",
            "unauthenticated",
            "not authenticated",
            "not logged in",
            "log in",
            "login required",
            "api key",
            "authentication",
        ])
    {
        return AiError::NotAuthenticated;
    }
    if mentions_status(&text, 429)
        || says(&[
            "rate limit",
            "rate_limit",
            "quota",
            "usage limit",
            "too many requests",
            "resource_exhausted",
            "resource exhausted",
        ])
    {
        return AiError::UsageLimited { retry_after: None };
    }
    if [500, 502, 503, 504, 529]
        .iter()
        .any(|status| mentions_status(&text, *status))
        || says(&[
            "overloaded",
            "service unavailable",
            "econnreset",
            "etimedout",
            "enotfound",
            "network error",
            "temporarily",
        ])
    {
        return AiError::Transient(format!("{provider}_upstream"));
    }
    AiError::CompletionUnknown(format!(
        "{provider}_failed:exit={}",
        code.map_or_else(|| "signal".to_owned(), |code| code.to_string())
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json_is_found_whether_plain_fenced_or_inside_prose() {
        let expected = json!({"summary": "x"});
        for text in [
            r#"{"summary":"x"}"#,
            "  {\"summary\": \"x\"}\n",
            "```json\n{\"summary\":\"x\"}\n```",
            "Here it is:\n```\n{\"summary\":\"x\"}\n```\nDone.",
            "Sure! {\"summary\":\"x\"} hope that helps",
        ] {
            assert_eq!(json_from_text(text), Some(expected.clone()), "{text}");
        }
        assert_eq!(json_from_text("no json here"), None);
        assert_eq!(json_from_text(""), None);
    }

    #[test]
    fn a_status_is_a_number_of_its_own_not_part_of_another() {
        assert!(mentions_status("http 429 too many", 429));
        assert!(mentions_status("status=401", 401));
        assert!(!mentions_status("took 14290 ms", 429));
        assert!(!mentions_status("id 5030", 503));
    }

    #[test]
    fn a_failure_is_a_refusal_class_only_when_the_cli_said_which() {
        assert_eq!(
            classify_failure("grok", Some(1), "Error: 401 Unauthorized"),
            AiError::NotAuthenticated
        );
        assert_eq!(
            classify_failure("grok", Some(1), "You are not authenticated."),
            AiError::NotAuthenticated
        );
        assert_eq!(
            classify_failure("pi", Some(1), "429 rate limit exceeded"),
            AiError::UsageLimited { retry_after: None }
        );
        assert_eq!(
            classify_failure("grok", Some(1), "503 Service Unavailable"),
            AiError::Transient("grok_upstream".to_owned())
        );
        assert_eq!(
            classify_failure("pi", Some(3), "something odd happened"),
            AiError::CompletionUnknown("pi_failed:exit=3".to_owned()),
            "an unrecognised failure never reads as a refusal that may be re-sent"
        );
        assert_eq!(
            classify_failure("pi", None, ""),
            AiError::CompletionUnknown("pi_failed:exit=signal".to_owned())
        );
    }

    #[test]
    fn the_system_prompt_names_the_schema_and_asks_for_one_object() {
        let prompt = system_with_schema("Classify.", &json!({"type": "object"}));
        assert!(prompt.starts_with("Classify."));
        assert!(prompt.contains(r#"{"type":"object"}"#));
        assert!(prompt.contains("exactly one JSON object"));
    }
}
