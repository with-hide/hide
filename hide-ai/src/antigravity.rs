//! `agy` (the Antigravity CLI) in print mode, one child per request, and its
//! `/usage` read (PRD antigravity-first-class D-09, B31-B33).
//!
//! agy has no switch that turns its tools off, so a request runs in a folder
//! Hide keeps for it ([`read_only_workspace`]) whose own `.agents/hooks.json`
//! refuses every tool call ([`READ_ONLY_HOOKS`]). agy loads a folder's hooks
//! only when the folder is in its trusted list, which the install kit records
//! when it installs agy's hook (`hide_agent_hooks::antigravity_trust`), and it
//! refuses a tool call whose `PreToolUse` hook fails in any way, so the hook
//! is fail-closed. Measured against agy 1.4.0 on 2026-10-16 with a real
//! account: in that trusted folder every tool call (`view_file`,
//! `run_command`, `write_to_file`, `manage_task`) ended refused and nothing was
//! written; in an untrusted folder the folder's hook was not loaded. A request
//! is therefore refused unless the folder, its hook file and its trust are all
//! as Hide wrote them, and an answer whose stream shows a tool step that
//! finished is refused too.
//!
//! Every agy child runs with the login environment and a `PATH` naming a
//! folder that does not exist: a signed-out agy opens the operator's browser
//! to sign in through a program it finds on `PATH` (B32), and every hook,
//! the operator's included, then fails to start its shell, which agy also
//! reads as a refused tool. agy's own update check is off, and its log goes
//! to one file of Hide's per kind of child, emptied before each start: agy
//! otherwise keeps a file per process and never removes one.
//!
//! The prompt goes on stdin as agy's one `stream-json` input message; plain
//! `-p` ignores stdin. `--json-schema` is not used: agy ends such a turn
//! through a `finish` tool, which the folder's hook refuses, so the schema is
//! asked for in the prompt as for the other text CLIs and the router
//! validates the answer.
//!
//! Each request leaves a conversation in agy's history (agy has no flag that
//! skips it), filed under the read-only folder, which no checkout reads.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};

use crate::program::Program;
use crate::runner::{self, Environment, Spec};
use crate::text_cli::{TextCliConfig, classify_failure, json_from_text, system_with_schema};
use crate::{
    AiBackend, AiError, AiRequest, AiResponse, AiUsage, Availability, CancelToken, ModelCatalog,
    ProviderId, UsageError,
};

const PROGRAM: &str = "agy";

/// The hook file that keeps a request tool-free: every tool call runs a
/// command that fails, and agy refuses a call whose hook fails.
pub const READ_ONLY_HOOKS: &str = r#"{"hide-ai":{"PreToolUse":[{"matcher":".*","hooks":[{"type":"command","command":"exit 1","timeout":5}]}]}}
"#;

/// The most of agy's settings file read to find the trusted folders.
const SETTINGS_LIMIT: u64 = 1024 * 1024;
/// `agy models` and `/usage` read the account and run no model turn.
const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// The folder that keeps the log of the last agy child of each kind Hide
/// started. Without `--log-file` every agy process adds a file of about 20 KB
/// to `~/.gemini/antigravity-cli/log/` and agy never removes one: the weekly
/// `/usage` read alone added one every five minutes, 226 between 2026-10-16
/// and 10-18. agy appends to a file it is given, so each kind's file is
/// emptied before its next child starts.
fn log_folder(home: &Path) -> PathBuf {
    home.join(".hide").join("ai").join("antigravity-logs")
}
/// The log file of each kind of agy child.
const REQUEST_LOG: &str = "request.log";
const MODELS_LOG: &str = "models.log";
const USAGE_LOG: &str = "usage.log";

/// The folder a Hide AI request to agy runs in.
pub fn read_only_workspace(home: &Path) -> PathBuf {
    home.join(".hide").join("ai").join("antigravity-read-only")
}

/// The folder's own hook file, which agy reads beside its global one.
pub fn read_only_hooks_file(workspace: &Path) -> PathBuf {
    workspace.join(".agents").join("hooks.json")
}

/// agy's settings file, whose `trustedWorkspaces` lists the folders whose own
/// hooks it loads.
fn settings_path(home: &Path) -> PathBuf {
    home.join(".gemini")
        .join("antigravity-cli")
        .join("settings.json")
}

/// A `PATH` that finds nothing: no browser opener and no shell for a hook.
fn no_programs(workspace: &Path) -> String {
    workspace
        .join(".agents")
        .join("no-programs")
        .display()
        .to_string()
}

/// The read-only folder as agy spells it, when its hook file is Hide's and
/// agy trusts it; otherwise why a request may not run there.
pub fn read_only_prepared(home: &Path) -> Result<PathBuf, &'static str> {
    let folder = hide_platform::fs::identity::canonical(&read_only_workspace(home))
        .map_err(|_| "antigravity_read_only_missing")?;
    match std::fs::read(read_only_hooks_file(&folder)) {
        Ok(bytes) if bytes == READ_ONLY_HOOKS.as_bytes() => {}
        Ok(_) => return Err("antigravity_read_only_hook_changed"),
        Err(_) => return Err("antigravity_read_only_missing"),
    }
    let spelled = folder.to_str().ok_or("antigravity_read_only_missing")?;
    let trusted = trusted_entries(home)
        .map_err(|reason| reason.unwrap_or("antigravity_read_only_untrusted"))?
        .iter()
        .any(|entry| entry == spelled);
    if trusted {
        Ok(folder)
    } else {
        Err("antigravity_read_only_untrusted")
    }
}

/// The folders agy's settings list as trusted; `Err(None)` when there are
/// no settings to read, `Err(Some(reason))` when they are not agy's JSON.
fn trusted_entries(home: &Path) -> Result<Vec<String>, Option<&'static str>> {
    let settings = std::fs::File::open(settings_path(home))
        .and_then(|file| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut std::io::Read::take(file, SETTINGS_LIMIT), &mut bytes)
                .map(|_| bytes)
        })
        .map_err(|_| None)?;
    let settings: Value =
        serde_json::from_slice(&settings).map_err(|_| Some("antigravity_settings_unreadable"))?;
    Ok(settings
        .get("trustedWorkspaces")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default())
}

/// Whether agy trusts `folder` (canonical), so starting agy there asks no
/// trust question: an entry of agy's list names the same folder, however it
/// is spelled.
pub fn trusts(home: &Path, folder: &Path) -> bool {
    trusted_entries(home).is_ok_and(|entries| {
        entries.iter().any(|entry| {
            hide_platform::fs::identity::canonical(Path::new(entry))
                .is_ok_and(|entry| entry == folder)
        })
    })
}

pub struct AntigravityCliBackend {
    config: TextCliConfig,
    home: Option<PathBuf>,
}

impl AntigravityCliBackend {
    pub fn new(config: TextCliConfig) -> Self {
        Self::with_home(config, hide_platform::host::home_dir().ok())
    }

    /// A backend reading agy's files under `home`, which a test chooses.
    pub fn with_home(config: TextCliConfig, home: Option<PathBuf>) -> Self {
        Self { config, home }
    }

    /// The exact argument vector one request is started with. The prompt is
    /// not here: it goes on stdin.
    pub fn print_arguments(model: &str, deadline: Duration) -> Vec<String> {
        let mut args: Vec<String> = [
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--disable-slash-commands",
            "--print-timeout",
        ]
        .map(str::to_owned)
        .to_vec();
        args.push(format!("{}s", deadline.as_secs().max(1)));
        if !model.is_empty() {
            args.push("--model".to_owned());
            args.push(model.to_owned());
        }
        // `-p` takes a value; the prompt itself is the stdin message.
        args.extend(["-p".to_owned(), String::new()]);
        args
    }

    /// The argument vector of the weekly-limit read: agy's own `/usage`.
    pub fn usage_arguments() -> Vec<String> {
        ["-p", "/usage", "--output-format", "json"]
            .map(str::to_owned)
            .to_vec()
    }

    /// The one stdin line a request sends.
    pub fn stdin_message(system: &str, schema: &Value, input: &str) -> String {
        let content = format!("{}\n\nInput:\n{input}", system_with_schema(system, schema));
        let mut line = json!({"event": "user", "message": {"content": content}}).to_string();
        line.push('\n');
        line
    }

    /// Variables every agy child gets after the login environment.
    fn environment(workspace: &Path) -> [(&'static str, String); 2] {
        [
            ("PATH", no_programs(workspace)),
            ("AGY_CLI_DISABLE_AUTO_UPDATE", "1".to_owned()),
        ]
    }

    /// Whether agy was ever signed in here: a home without agy's folder
    /// would only answer by asking to sign in.
    fn signed_in_before(&self) -> bool {
        self.home
            .as_deref()
            .is_some_and(|home| home.join(".gemini").join("antigravity-cli").is_dir())
    }

    fn probe(
        &self,
        binary: &Program,
        args: &[String],
        log: &str,
        cancel: &CancelToken,
    ) -> Result<runner::Run, Failed> {
        let cwd = self
            .home
            .as_deref()
            .map_or_else(|| self.config.cwd.clone(), read_only_workspace);
        let cwd = if cwd.is_dir() {
            cwd
        } else {
            self.config.cwd.clone()
        };
        self.start(binary, args, log, &cwd, None, PROBE_TIMEOUT, cancel)
    }

    /// Runs one agy child in `cwd` with the login environment and the
    /// variables above, writing its log to the file `log` names in
    /// [`log_folder`], which is emptied first. Every agy child Hide starts is
    /// started here, and none starts while its log file cannot be emptied.
    #[allow(clippy::too_many_arguments)]
    fn start(
        &self,
        binary: &Program,
        args: &[String],
        log: &str,
        cwd: &Path,
        stdin: Option<&str>,
        deadline: Duration,
        cancel: &CancelToken,
    ) -> Result<runner::Run, Failed> {
        let log = self
            .emptied_log(log)
            .map_err(|error| Failed::Log(error.kind()))?;
        let mut logged = vec!["--log-file".to_owned(), log.display().to_string()];
        logged.extend_from_slice(args);
        runner::run(
            &Spec {
                binary,
                args: &logged,
                cwd,
                stdin,
                environment: Environment::Login,
                set: &Self::environment(cwd),
                deadline,
            },
            cancel,
        )
        .map_err(Failed::Run)
    }

    /// The log file `name` in [`log_folder`], private to the account and
    /// empty.
    fn emptied_log(&self, name: &str) -> std::io::Result<PathBuf> {
        let home = self
            .home
            .as_deref()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        let folder = log_folder(home);
        hide_platform::fs::private::own_dir(&folder)?;
        let path = folder.join(name);
        // Emptied in place rather than removed: a child of the same kind
        // that is still running keeps appending to it, which Windows would
        // refuse a removal for.
        hide_platform::fs::private::open_own_file(&path, true)?.set_len(0)?;
        Ok(path)
    }

    fn list_models(&self, binary: &Program) -> Result<ModelsOutcome, String> {
        let run = self
            .probe(
                binary,
                &["models".to_owned()],
                MODELS_LOG,
                &CancelToken::new(),
            )
            .map_err(|error| error.diagnostic("antigravity_models"))?;
        Ok(parse_models(&run))
    }

    /// agy's `/usage` answer, the whole JSON frame the core's parser reads.
    pub fn usage_text(&self, cancel: &CancelToken) -> Result<String, UsageError> {
        let binary = self
            .config
            .resolved(PROGRAM)
            .ok_or(UsageError::NotInstalled)?;
        if !self.signed_in_before() {
            return Err(UsageError::NotAuthenticated);
        }
        let run = self
            .probe(&binary, &Self::usage_arguments(), USAGE_LOG, cancel)
            .map_err(|error| match error {
                Failed::Run(runner::RunError::Deadline) => UsageError::Timeout,
                Failed::Run(runner::RunError::Cancelled) => UsageError::Cancelled,
                other => UsageError::Failed(other.diagnostic("antigravity_usage")),
            })?;
        let frame = serde_json::from_str::<Value>(run.stdout.trim()).ok();
        let status = frame
            .as_ref()
            .and_then(|frame| frame.get("status"))
            .and_then(Value::as_str);
        if status == Some("SUCCESS") && run.succeeded() {
            return Ok(run.stdout.trim().to_owned());
        }
        let said = frame
            .as_ref()
            .and_then(|frame| frame.get("error"))
            .and_then(Value::as_str)
            .map_or_else(|| run.stderr.clone(), str::to_owned);
        match classify_failure("antigravity", run.code, &said) {
            AiError::NotAuthenticated => Err(UsageError::NotAuthenticated),
            _ if frame.is_none() => Err(UsageError::NoResultFrame),
            _ => Err(UsageError::Failed(format!("usage_exit_{}", run.exit()))),
        }
    }
}

/// Why an agy child gave no output: its log file could not be emptied, so it
/// was never started, or the run itself failed.
enum Failed {
    Log(std::io::ErrorKind),
    Run(runner::RunError),
}

impl Failed {
    fn diagnostic(&self, stage: &str) -> String {
        match self {
            Self::Log(kind) => format!("{stage}_log_unavailable:{kind}"),
            Self::Run(error) => error.diagnostic(stage),
        }
    }
}

/// What `agy models` said.
#[derive(Debug, Eq, PartialEq)]
enum ModelsOutcome {
    NotAuthenticated,
    Models(Vec<String>),
    Unreadable,
}

/// `agy models` prints a line per model, its id and its name split by a tab,
/// after a "Fetching available models..." line.
fn parse_models(run: &runner::Run) -> ModelsOutcome {
    let models: Vec<String> = run
        .stdout
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(id, _)| id.trim())
        .filter(|id| !id.is_empty() && !id.contains(char::is_whitespace))
        .map(str::to_owned)
        .collect();
    if run.succeeded() && !models.is_empty() {
        return ModelsOutcome::Models(models);
    }
    match classify_failure(
        "antigravity",
        run.code,
        &format!("{}\n{}", run.stdout, run.stderr),
    ) {
        AiError::NotAuthenticated => ModelsOutcome::NotAuthenticated,
        _ => ModelsOutcome::Unreadable,
    }
}

impl AiBackend for AntigravityCliBackend {
    fn id(&self) -> ProviderId {
        ProviderId::ANTIGRAVITY
    }

    fn availability(&self) -> Availability {
        let Some(binary) = self.config.resolved(PROGRAM) else {
            return Availability::NotInstalled;
        };
        let Some(home) = self.home.as_deref() else {
            return Availability::Unavailable {
                reason: "antigravity_home_unknown".to_owned(),
            };
        };
        if !self.signed_in_before() {
            return Availability::NeedsLogin;
        }
        // Until the kit has made the folder and agy trusts it, nothing keeps
        // a request tool-free, so the agent stays dimmed with the reason.
        if let Err(reason) = read_only_prepared(home) {
            return Availability::Unsupported {
                reason: reason.to_owned(),
            };
        }
        match self.list_models(&binary) {
            Ok(ModelsOutcome::Models(_)) => Availability::Ready,
            Ok(ModelsOutcome::NotAuthenticated) => Availability::NeedsLogin,
            Ok(ModelsOutcome::Unreadable) => Availability::Unavailable {
                reason: "antigravity_models_unreadable".to_owned(),
            },
            Err(reason) => Availability::Unavailable { reason },
        }
    }

    fn models(&self) -> ModelCatalog {
        let Some(binary) = self.config.resolved(PROGRAM) else {
            return ModelCatalog::Unknown {
                reason: "antigravity_not_installed".to_owned(),
            };
        };
        if !self.signed_in_before() {
            return ModelCatalog::Unknown {
                reason: "antigravity_not_authenticated".to_owned(),
            };
        }
        match self.list_models(&binary) {
            Ok(ModelsOutcome::Models(models)) => ModelCatalog::Offered(models),
            Ok(ModelsOutcome::NotAuthenticated) => ModelCatalog::Unknown {
                reason: "antigravity_not_authenticated".to_owned(),
            },
            Ok(ModelsOutcome::Unreadable) => ModelCatalog::Unknown {
                reason: "antigravity_models_unreadable".to_owned(),
            },
            Err(reason) => ModelCatalog::Unknown { reason },
        }
    }

    fn execute(&self, request: &AiRequest, cancel: &CancelToken) -> Result<AiResponse, AiError> {
        let binary = self
            .config
            .resolved(PROGRAM)
            .ok_or_else(|| AiError::ProviderUnavailable("antigravity_not_installed".to_owned()))?;
        let home = self
            .home
            .as_deref()
            .ok_or_else(|| AiError::ProviderUnavailable("antigravity_home_unknown".to_owned()))?;
        if !self.signed_in_before() {
            return Err(AiError::NotAuthenticated);
        }
        let workspace =
            read_only_prepared(home).map_err(|reason| AiError::Unsupported(reason.to_owned()))?;
        let run = self
            .start(
                &binary,
                &Self::print_arguments(request.model(&self.config.model), request.deadline),
                REQUEST_LOG,
                &workspace,
                Some(&Self::stdin_message(
                    &request.system_prompt(),
                    &request.output_schema,
                    &request.input,
                )),
                request.deadline,
                cancel,
            )
            .map_err(|error| match error {
                Failed::Run(error) => error.into_error("antigravity"),
                // Nothing was started, so the request never reached agy.
                log @ Failed::Log(_) => AiError::ProviderUnavailable(log.diagnostic("antigravity")),
            })?;
        answer(&run)
    }
}

/// Turns one finished child's event stream into the answer or the failure it
/// reported.
fn answer(run: &runner::Run) -> Result<AiResponse, AiError> {
    let mut result = None;
    let mut tool_ran = false;
    for line in run.stdout.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match event.get("event").and_then(Value::as_str) {
            Some("step_update") => {
                let step = &event["step_update"];
                tool_ran |= step["step_type"] == "tool" && step["state"] == "DONE";
            }
            Some("result") => result = Some(event["result"].clone()),
            _ => {}
        }
    }
    // The folder's hook refuses every tool; a tool that finished means it
    // did not hold, and the answer is not one Hide may use.
    if tool_ran {
        return Err(AiError::InvalidOutput("antigravity_tool_ran".to_owned()));
    }
    let Some(result) = result else {
        return Err(if run.succeeded() {
            AiError::CompletionUnknown("antigravity_no_result".to_owned())
        } else {
            classify_failure("antigravity", run.code, &run.stderr)
        });
    };
    if result.get("status").and_then(Value::as_str) != Some("SUCCESS") {
        let said = result
            .get("error")
            .and_then(Value::as_str)
            .map_or_else(|| run.stderr.clone(), str::to_owned);
        return Err(classify_failure("antigravity", run.code, &said));
    }
    let value = result
        .get("response")
        .and_then(Value::as_str)
        .and_then(json_from_text)
        .ok_or_else(|| AiError::InvalidOutput("antigravity_answer_not_json".to_owned()))?;
    let usage = result.get("usage");
    let tokens = |name: &str| {
        usage
            .and_then(|usage| usage.get(name))
            .and_then(Value::as_u64)
    };
    Ok(AiResponse {
        value,
        usage: AiUsage {
            input_tokens: tokens("input_tokens"),
            output_tokens: tokens("output_tokens"),
            // The request's conversation is in agy's history, which the
            // session usage reader counts; counting it here too would count
            // it twice.
            models: Vec::new(),
            // agy's result names no thinking count.
            thinking_tokens: None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(code: i32, stdout: &str, stderr: &str) -> runner::Run {
        runner::Run {
            code: Some(code),
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    fn result(status: &str, response: &str, error: &str) -> String {
        json!({"event": "result", "result": {"status": status, "response": response,
            "error": error, "usage": {"input_tokens": 9, "output_tokens": 3}}})
        .to_string()
    }

    #[test]
    fn the_prompt_is_the_stdin_message_and_never_an_argument() {
        let args = AntigravityCliBackend::print_arguments("", Duration::from_secs(45));
        assert!(
            args.windows(2)
                .any(|p| p == ["--input-format", "stream-json"])
        );
        assert!(args.windows(2).any(|p| p == ["--print-timeout", "45s"]));
        assert!(args.iter().any(|arg| arg == "--disable-slash-commands"));
        assert!(
            !args
                .iter()
                .any(|arg| arg == "--json-schema" || arg == "--model")
        );
        assert_eq!(args[args.len() - 2..], ["-p".to_owned(), String::new()]);
        let message: Value = serde_json::from_str(&AntigravityCliBackend::stdin_message(
            "Classify.",
            &json!({"type": "object"}),
            "the transcript",
        ))
        .unwrap();
        assert_eq!(message["event"], "user");
        let content = message["message"]["content"].as_str().unwrap();
        assert!(content.starts_with("Classify."));
        assert!(content.ends_with("the transcript"));
    }

    #[test]
    fn the_answer_is_the_json_in_the_result_and_a_tool_that_ran_refuses_it() {
        let step = |state: &str| {
            json!({"event": "step_update", "step_update": {"step_type": "tool", "state": state}})
                .to_string()
        };
        let answered = format!(
            "{}\n{}\n{}\n",
            step("ACTIVE"),
            step("ERROR"),
            result("SUCCESS", "```json\n{\"summary\":\"x\"}\n```", "")
        );
        let response = answer(&run(0, &answered, "")).unwrap();
        assert_eq!(response.value, json!({"summary": "x"}));
        assert_eq!(response.usage.output_tokens, Some(3));
        let ran = format!("{}\n{}\n", step("DONE"), result("SUCCESS", "{}", ""));
        assert_eq!(
            answer(&run(0, &ran, "")).unwrap_err(),
            AiError::InvalidOutput("antigravity_tool_ran".to_owned())
        );
        let prose = result("SUCCESS", "no json", "");
        assert!(matches!(
            answer(&run(0, &prose, "")),
            Err(AiError::InvalidOutput(_))
        ));
    }

    #[test]
    fn a_failed_turn_is_classified_from_what_agy_said() {
        let logged_out = result("ERROR", "", "Error: authentication failed");
        assert_eq!(
            answer(&run(1, &logged_out, "")).unwrap_err(),
            AiError::NotAuthenticated
        );
        assert_eq!(
            answer(&run(0, "", "")).unwrap_err(),
            AiError::CompletionUnknown("antigravity_no_result".to_owned())
        );
    }

    #[test]
    fn the_model_list_is_the_ids_before_each_tab() {
        let listed = run(
            0,
            "Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\nclaude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)\n",
            "",
        );
        assert_eq!(
            parse_models(&listed),
            ModelsOutcome::Models(vec![
                "gemini-3.8-flash-high".to_owned(),
                "claude-sonnet-4-6".to_owned()
            ])
        );
        assert_eq!(
            parse_models(&run(1, "", "Error: authentication failed")),
            ModelsOutcome::NotAuthenticated
        );
        assert_eq!(
            parse_models(&run(0, "Fetching available models...\n", "")),
            ModelsOutcome::Unreadable
        );
    }

    #[test]
    fn a_request_runs_only_where_hide_wrote_the_hook_and_agy_trusts_the_folder() {
        let home = tempfile::tempdir().unwrap();
        assert_eq!(
            read_only_prepared(home.path()),
            Err("antigravity_read_only_missing")
        );
        let folder = read_only_workspace(home.path());
        std::fs::create_dir_all(folder.join(".agents")).unwrap();
        std::fs::write(read_only_hooks_file(&folder), READ_ONLY_HOOKS).unwrap();
        assert_eq!(
            read_only_prepared(home.path()),
            Err("antigravity_read_only_untrusted")
        );
        let canonical = hide_platform::fs::identity::canonical(&folder).unwrap();
        let settings = settings_path(home.path());
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            json!({"trustedWorkspaces": [canonical.to_str().unwrap()]}).to_string(),
        )
        .unwrap();
        assert_eq!(read_only_prepared(home.path()), Ok(canonical));
        std::fs::write(read_only_hooks_file(&folder), "{}").unwrap();
        assert_eq!(
            read_only_prepared(home.path()),
            Err("antigravity_read_only_hook_changed")
        );
    }
}
