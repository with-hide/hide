//! `grok --prompt-file`, one child per request.
//!
//! Grok Build documents a headless mode with `--output-format json` (one
//! object with `text`, `stopReason`, `sessionId`, `usage`), `--json-schema`,
//! `--tools`, `--no-subagents`, `--disable-web-search`, `--max-turns`,
//! `--system-prompt-override` and error objects `{"type":"error",...}` with
//! exit 1: https://github.com/xai-org/grok-build, `14-headless-mode.md`.
//! Headless mode does not read piped stdin into the prompt, so the transcript
//! goes into an owner-only temporary file named by `--prompt-file` and removed
//! when the request ends; it never reaches an argument vector.
//!
//! None of this has been run against a real account from this build, and two
//! facts the documentation leaves open are handled by reading both places: the
//! validated object may arrive as `structured_output` or inside `text`. The
//! contract is proven against a stand-in CLI only (`AI_PROVIDERS.md`).
//!
//! There is no `auth status`: `grok models` prints "You are not
//! authenticated." when logged out and the account's model list otherwise,
//! each model on a line marked `*` (the default) or `-`.

use serde_json::Value;

use crate::program::Program;
use crate::runner::{self, Environment, PrivateFile, Spec};
use crate::text_cli::{TextCliConfig, classify_failure, json_from_text};
use crate::{
    AiBackend, AiError, AiRequest, AiResponse, AiUsage, Availability, CancelToken, ModelCatalog,
    ProviderId,
};

const PROGRAM: &str = "grok";

pub struct GrokCliBackend {
    config: TextCliConfig,
}

impl GrokCliBackend {
    pub fn new(config: TextCliConfig) -> Self {
        Self { config }
    }

    /// The exact argument vector one request is started with, `prompt_file`
    /// naming the file that holds the transcript.
    pub fn print_arguments(
        model: &str,
        system: &str,
        output_schema: &Value,
        prompt_file: &str,
    ) -> Vec<String> {
        let mut args: Vec<String> = [
            "--prompt-file",
            prompt_file,
            "--output-format",
            "json",
            "--json-schema",
            &output_schema.to_string(),
            "--system-prompt-override",
            system,
            // No tool, no sub-agent, no web search, one turn: the call reads
            // the transcript and answers.
            "--tools",
            "",
            "--no-subagents",
            "--disable-web-search",
            "--max-turns",
            "1",
        ]
        .map(str::to_owned)
        .to_vec();
        if !model.is_empty() {
            args.push("--model".to_owned());
            args.push(model.to_owned());
        }
        args
    }

    /// Environment that keeps the call from reading or leaving anything
    /// beyond the CLI's own session file: no cross-session memory, no update
    /// check.
    fn environment() -> [(&'static str, String); 2] {
        [
            ("GROK_MEMORY", "0".to_owned()),
            ("GROK_DISABLE_AUTOUPDATER", "1".to_owned()),
        ]
    }

    fn list_models(&self, binary: &Program) -> Result<ModelsOutcome, String> {
        let run = self.config.probe(binary, &["models"], "grok_models")?;
        Ok(parse_models(&run.stdout, &run.stderr))
    }
}

/// What `grok models` said.
#[derive(Debug, Eq, PartialEq)]
enum ModelsOutcome {
    NotAuthenticated,
    Models(Vec<String>),
    Unreadable,
}

fn parse_models(stdout: &str, stderr: &str) -> ModelsOutcome {
    let text = format!("{stdout}\n{stderr}").to_lowercase();
    if text.contains("not authenticated") {
        return ModelsOutcome::NotAuthenticated;
    }
    let models: Vec<String> = stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix('*').or_else(|| line.strip_prefix('-'))?;
            rest.split_whitespace().next().map(str::to_owned)
        })
        .collect();
    if models.is_empty() {
        ModelsOutcome::Unreadable
    } else {
        ModelsOutcome::Models(models)
    }
}

impl AiBackend for GrokCliBackend {
    fn id(&self) -> ProviderId {
        ProviderId::GROK
    }

    fn availability(&self) -> Availability {
        let Some(binary) = self.config.resolved(PROGRAM) else {
            return Availability::NotInstalled;
        };
        match self.list_models(&binary) {
            Ok(ModelsOutcome::Models(_)) => Availability::Ready,
            Ok(ModelsOutcome::NotAuthenticated) => Availability::NeedsLogin,
            Ok(ModelsOutcome::Unreadable) => Availability::Unavailable {
                reason: "grok_models_unreadable".to_owned(),
            },
            Err(reason) => Availability::Unavailable { reason },
        }
    }

    fn models(&self) -> ModelCatalog {
        let Some(binary) = self.config.resolved(PROGRAM) else {
            return ModelCatalog::Unknown {
                reason: "grok_not_installed".to_owned(),
            };
        };
        match self.list_models(&binary) {
            Ok(ModelsOutcome::Models(models)) => ModelCatalog::plain(models),
            Ok(ModelsOutcome::NotAuthenticated) => ModelCatalog::Unknown {
                reason: "grok_not_authenticated".to_owned(),
            },
            Ok(ModelsOutcome::Unreadable) => ModelCatalog::Unknown {
                reason: "grok_models_unreadable".to_owned(),
            },
            Err(reason) => ModelCatalog::Unknown { reason },
        }
    }

    fn execute(&self, request: &AiRequest, cancel: &CancelToken) -> Result<AiResponse, AiError> {
        let binary = self
            .config
            .resolved(PROGRAM)
            .ok_or_else(|| AiError::ProviderUnavailable("grok_not_installed".to_owned()))?;
        let prompt = PrivateFile::create("prompt.txt", &request.input).map_err(|error| {
            AiError::ProviderUnavailable(format!("grok_prompt_file:{}", error.kind()))
        })?;
        let run = runner::run(
            &Spec {
                binary: &binary,
                args: &Self::print_arguments(
                    request.model(&self.config.model),
                    &request.system_prompt(),
                    &request.output_schema,
                    &prompt.path().display().to_string(),
                ),
                cwd: prompt.dir(),
                stdin: None,
                environment: Environment::Inherit,
                set: &self.config.set(&Self::environment()),
                deadline: request.deadline,
            },
            cancel,
        )
        .map_err(|error| error.into_error("grok"))?;
        answer(&run)
    }
}

/// Turns one finished child into the answer or the failure it reported.
fn answer(run: &runner::Run) -> Result<AiResponse, AiError> {
    let frame = serde_json::from_str::<Value>(run.stdout.trim()).ok();
    let is_error = frame
        .as_ref()
        .is_some_and(|frame| frame.get("type").and_then(Value::as_str) == Some("error"));
    if !run.succeeded() || is_error {
        let text = match &frame {
            Some(frame) if is_error => frame.to_string(),
            _ => run.stderr.clone(),
        };
        return Err(classify_failure("grok", run.code, &text));
    }
    let frame = frame.ok_or_else(|| {
        // A clean exit with output that is not the CLI's object: the turn ran.
        AiError::CompletionUnknown("grok_output_not_json".to_owned())
    })?;
    // The validated object is not documented to have one home in the plain
    // `json` shape, so both are read; the router validates what comes back.
    let value = frame
        .get("structured_output")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            frame
                .get("text")
                .and_then(Value::as_str)
                .and_then(json_from_text)
        })
        .ok_or_else(|| AiError::InvalidOutput("grok_answer_not_json".to_owned()))?;
    let usage = frame.get("usage");
    Ok(AiResponse {
        value,
        usage: AiUsage {
            input_tokens: usage
                .and_then(|usage| usage.get("input_tokens"))
                .and_then(Value::as_u64),
            output_tokens: usage
                .and_then(|usage| usage.get("output_tokens"))
                .and_then(Value::as_u64),
            // Whether a Grok one-shot leaves a session file is not known, and
            // counting one that does would count it twice (PRD D-06).
            models: Vec::new(),
            thinking_tokens: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run(code: i32, stdout: Value, stderr: &str) -> runner::Run {
        runner::Run {
            code: Some(code),
            stdout: stdout.to_string(),
            stderr: stderr.to_owned(),
        }
    }

    #[test]
    fn the_vector_turns_every_tool_off_and_names_the_prompt_file_not_the_prompt() {
        let args = GrokCliBackend::print_arguments(
            "",
            "sys",
            &json!({"type": "object"}),
            "/tmp/x/prompt.txt",
        );
        assert!(
            args.windows(2)
                .any(|p| p == ["--prompt-file", "/tmp/x/prompt.txt"])
        );
        assert!(args.windows(2).any(|p| p == ["--tools", ""]));
        for flag in ["--no-subagents", "--disable-web-search"] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}");
        }
        assert!(args.windows(2).any(|p| p == ["--max-turns", "1"]));
        assert!(!args.iter().any(|arg| arg == "--model"));
        assert!(
            GrokCliBackend::print_arguments("grok-4.6", "s", &json!({}), "p")
                .windows(2)
                .any(|p| p == ["--model", "grok-4.6"])
        );
    }

    #[test]
    fn the_models_text_is_a_list_a_login_signal_or_unreadable() {
        assert_eq!(
            parse_models("* grok-4.6 (default)\n- grok-4.5\n", ""),
            ModelsOutcome::Models(vec!["grok-4.6".to_owned(), "grok-4.5".to_owned()])
        );
        assert_eq!(
            parse_models("You are not authenticated.\n", ""),
            ModelsOutcome::NotAuthenticated
        );
        assert_eq!(parse_models("", ""), ModelsOutcome::Unreadable);
        assert_eq!(
            parse_models("usage: grok models\n", ""),
            ModelsOutcome::Unreadable
        );
    }

    #[test]
    fn the_answer_is_structured_output_or_the_json_in_text() {
        let structured = run(
            0,
            json!({"text": "ignored", "structured_output": {"summary": "x"},
                   "usage": {"input_tokens": 5, "output_tokens": 2}}),
            "",
        );
        let response = answer(&structured).unwrap();
        assert_eq!(response.value, json!({"summary": "x"}));
        assert_eq!(response.usage.output_tokens, Some(2));
        let text = run(0, json!({"text": "{\"summary\":\"y\"}"}), "");
        assert_eq!(answer(&text).unwrap().value, json!({"summary": "y"}));
        let prose = run(0, json!({"text": "no"}), "");
        assert!(matches!(answer(&prose), Err(AiError::InvalidOutput(_))));
    }

    #[test]
    fn an_error_object_or_a_failed_exit_is_classified_not_read_as_an_answer() {
        let logged_out = run(
            1,
            json!({"type": "error", "message": "401 unauthorized"}),
            "",
        );
        assert_eq!(answer(&logged_out).unwrap_err(), AiError::NotAuthenticated);
        let limited = run(1, json!({}), "429 too many requests");
        assert_eq!(
            answer(&limited).unwrap_err(),
            AiError::UsageLimited { retry_after: None }
        );
        let clean_error = run(0, json!({"type": "error", "message": "503 overloaded"}), "");
        assert_eq!(
            answer(&clean_error).unwrap_err(),
            AiError::Transient("grok_upstream".to_owned())
        );
    }
}
