//! `pi -p`, one child per request.
//!
//! Pi documents print mode (`-p`, with piped stdin prepended to the prompt,
//! the final assistant text on stdout and a non-zero exit on an error or an
//! aborted turn), a flag for every tool, extension and context source, and a
//! login check that makes no model call:
//! https://github.com/earendil-works/pi/tree/main/packages/coding-agent/docs
//! (`cli.md`). Pi is not installed on the machine this was built on; the
//! contract is proven against a stand-in CLI only (`AI_PROVIDERS.md`).
//!
//! Pi is a harness for many upstream providers, so "logged in" means some
//! provider is ready. A model named `provider/id` is checked with
//! `pi auth check --provider <provider>` (exit 0 ready, 1 not ready, 2
//! invalid); with the CLI's own default model there is no provider to name, so
//! the model list is the signal: a Pi that lists models has a provider set up.
//!
//! The call runs with every tool, extension, MCP server, skill, prompt
//! template and context file off, no saved session and thinking off
//! (`defaultThinkingLevel` is `medium`, which is the wrong cost for a short
//! JSON answer).

use serde_json::Value;

use crate::program::Program;
use crate::runner::{self, Environment, Spec};
use crate::text_cli::{TextCliConfig, classify_failure, json_from_text, system_with_schema};
use crate::{
    AiBackend, AiError, AiRequest, AiResponse, AiUsage, Availability, CancelToken, ModelCatalog,
    ProviderId,
};

const PROGRAM: &str = "pi";

/// The message argument; the transcript is on stdin and the instructions are
/// the system prompt.
const INSTRUCTION: &str = "Follow the system instructions for the input above.";

pub struct PiCliBackend {
    config: TextCliConfig,
}

impl PiCliBackend {
    pub fn new(config: TextCliConfig) -> Self {
        Self { config }
    }

    /// The exact argument vector one request is started with. The transcript
    /// is not here: it goes on stdin.
    pub fn print_arguments(model: &str, system: &str, output_schema: &Value) -> Vec<String> {
        let mut args: Vec<String> = [
            "-p",
            "--no-tools",
            "--no-session",
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-prompt-templates",
            "--no-context-files",
            "--thinking",
            "off",
            "--system-prompt",
        ]
        .map(str::to_owned)
        .to_vec();
        args.push(system_with_schema(system, output_schema));
        if !model.is_empty() {
            args.push("--model".to_owned());
            args.push(model.to_owned());
        }
        args.push(INSTRUCTION.to_owned());
        args
    }

    fn list_models(&self, binary: &Program) -> Result<Vec<String>, String> {
        let run = self.config.probe(binary, &["--list-models"], "pi_models")?;
        if !run.succeeded() {
            return Err(format!("pi_models_failed:exit={}", run.exit()));
        }
        Ok(parse_models(&run.stdout))
    }
}

/// The model refs in `pi --list-models`: a `provider/model` token on a line,
/// else the first two columns of a table row joined as `provider/model`. The
/// output format is documented only loosely, so both are read and nothing
/// else is guessed.
fn parse_models(stdout: &str) -> Vec<String> {
    let mut models = Vec::new();
    for line in stdout.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let reference = tokens
            .iter()
            .find(|token| token.contains('/') && !token.starts_with('/') && !token.ends_with('/'))
            .map(|token| (*token).to_owned());
        let reference = reference.or_else(|| match tokens.as_slice() {
            [provider, model, ..]
                if !provider.eq_ignore_ascii_case("provider")
                    && !provider.starts_with('#')
                    && model
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c)) =>
            {
                Some(format!("{provider}/{model}"))
            }
            _ => None,
        });
        if let Some(reference) = reference
            && !models.contains(&reference)
        {
            models.push(reference);
        }
    }
    models
}

impl AiBackend for PiCliBackend {
    fn id(&self) -> ProviderId {
        ProviderId::PI
    }

    fn availability(&self) -> Availability {
        let Some(binary) = self.config.resolved(PROGRAM) else {
            return Availability::NotInstalled;
        };
        if let Some((provider, _)) = self.config.model.split_once('/') {
            return match self.config.probe(
                &binary,
                &["auth", "check", "--provider", provider],
                "pi_auth",
            ) {
                Ok(run) => match run.code {
                    Some(0) => Availability::Ready,
                    Some(1 | 2) => Availability::NeedsLogin,
                    _ => Availability::Unavailable {
                        reason: format!("pi_auth_check_failed:exit={}", run.exit()),
                    },
                },
                Err(reason) => Availability::Unavailable { reason },
            };
        }
        match self.list_models(&binary) {
            Ok(models) if !models.is_empty() => Availability::Ready,
            Ok(_) => Availability::NeedsLogin,
            Err(reason) => Availability::Unavailable { reason },
        }
    }

    fn models(&self) -> ModelCatalog {
        let Some(binary) = self.config.resolved(PROGRAM) else {
            return ModelCatalog::Unknown {
                reason: "pi_not_installed".to_owned(),
            };
        };
        match self.list_models(&binary) {
            Ok(models) if !models.is_empty() => ModelCatalog::plain(models),
            Ok(_) => ModelCatalog::Unknown {
                reason: "pi_models_empty".to_owned(),
            },
            Err(reason) => ModelCatalog::Unknown { reason },
        }
    }

    fn execute(&self, request: &AiRequest, cancel: &CancelToken) -> Result<AiResponse, AiError> {
        let binary = self
            .config
            .resolved(PROGRAM)
            .ok_or_else(|| AiError::ProviderUnavailable("pi_not_installed".to_owned()))?;
        let run = runner::run(
            &Spec {
                binary: &binary,
                args: &Self::print_arguments(
                    request.model(&self.config.model),
                    &request.system_prompt(),
                    &request.output_schema,
                ),
                cwd: &self.config.cwd,
                stdin: Some(&request.input),
                environment: Environment::Inherit,
                set: &self.config.set(&[]),
                deadline: request.deadline,
            },
            cancel,
        )
        .map_err(|error| error.into_error("pi"))?;
        answer(&run)
    }
}

/// Turns one finished child into the answer or the failure it reported.
fn answer(run: &runner::Run) -> Result<AiResponse, AiError> {
    if !run.succeeded() {
        return Err(classify_failure("pi", run.code, &run.stderr));
    }
    let value = json_from_text(&run.stdout)
        .ok_or_else(|| AiError::InvalidOutput("pi_answer_not_json".to_owned()))?;
    Ok(AiResponse {
        value,
        usage: AiUsage::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_vector_turns_everything_off_and_carries_no_transcript() {
        let args =
            PiCliBackend::print_arguments("anthropic/claude-x", "sys", &json!({"type": "object"}));
        assert_eq!(args.first().map(String::as_str), Some("-p"));
        for flag in [
            "--no-tools",
            "--no-session",
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-prompt-templates",
            "--no-context-files",
        ] {
            assert!(args.iter().any(|arg| arg == flag), "{flag}");
        }
        assert!(args.windows(2).any(|p| p == ["--thinking", "off"]));
        assert!(
            args.windows(2)
                .any(|p| p == ["--model", "anthropic/claude-x"])
        );
        assert!(
            !PiCliBackend::print_arguments("", "s", &json!({}))
                .iter()
                .any(|arg| arg == "--model")
        );
    }

    #[test]
    fn model_refs_are_read_from_a_ref_per_line_or_a_provider_model_table() {
        assert_eq!(
            parse_models("anthropic/claude-x\nopenai/gpt-y  128k\n"),
            ["anthropic/claude-x", "openai/gpt-y"]
        );
        assert_eq!(
            parse_models(
                "provider  model  context\nanthropic  claude-x  200k\nopenai  gpt-y  128k\n"
            ),
            ["anthropic/claude-x", "openai/gpt-y"]
        );
        assert!(parse_models("").is_empty());
    }

    #[test]
    fn a_non_zero_exit_is_classified_and_a_clean_one_must_hold_json() {
        let failed = runner::Run {
            code: Some(1),
            stdout: String::new(),
            stderr: "No API key found for anthropic".to_owned(),
        };
        assert_eq!(answer(&failed).unwrap_err(), AiError::NotAuthenticated);
        let ok = runner::Run {
            code: Some(0),
            stdout: "```json\n{\"summary\":\"x\"}\n```\n".to_owned(),
            stderr: String::new(),
        };
        assert_eq!(answer(&ok).unwrap().value, json!({"summary": "x"}));
        let prose = runner::Run {
            code: Some(0),
            stdout: "I refuse".to_owned(),
            stderr: String::new(),
        };
        assert!(matches!(answer(&prose), Err(AiError::InvalidOutput(_))));
    }
}
