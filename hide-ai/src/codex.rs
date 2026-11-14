//! `codex app-server` over stdio, newline-delimited JSON-RPC 2.0.
//!
//! Protocol facts come from `codex app-server generate-json-schema` for the
//! installed CLI, not from guesswork; the request and notification names used
//! here are listed in `agents/runs/hide-ai-provider-layer/CONTRACT-2026-09-10.md`.
//!
//! This backend owns the app-server it starts (`oh-my-principle` resident
//! process practice): it runs the child under a private `CODEX_HOME` that
//! carries only an `auth.json` link, so no `config.toml` pulls MCP servers
//! into the tree, or, where the account may not make that link, under the
//! account's own folder with every listed MCP server switched off; it shuts
//! the child down through one graceful path on every exit; it ends the child
//! after ten idle minutes; and it exposes the process measurement the
//! router's cap is enforced against.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use crate::codex_home::{self, CodexHome};
use crate::log::AiLogEvent;
use hide_platform::process::{self as platform, OwnedChild};

use crate::process::{self, ProcessMeasurement};
use crate::program::{ItemRun, Program};
use crate::runner;
use crate::text_cli::{PROBE_TIMEOUT, json_from_text, system_with_schema};
use crate::{
    AiBackend, AiError, AiLogSink, AiRequest, AiResponse, AiUsage, AnswerMode, Availability,
    CancelToken, ModelCatalog, ModelTokens, ProviderId,
};

/// The user's decision for background features.
pub const DEFAULT_MODEL: &str = "gpt-5.6-luna";

/// Features that load tool definitions into every turn. Measured on
/// codex-cli 0.153.4: disabling them takes a label from 26,703 input tokens
/// to 11,898 on this model, and no config key disables the remainder.
const DISABLED_FEATURES: &[&str] = &[
    "shell_tool",
    "view_image",
    "sleep_tool",
    "unified_exec",
    "code_mode_host",
    "apps",
    "plugins",
    "browser_use",
    "browser_use_full_cdp_access",
    "browser_use_external",
    "computer_use",
    "image_generation",
    "multi_agent",
    "skill_search",
    "skill_mcp_dependency_install",
    "tool_suggest",
    "memories",
    "hooks",
    "goals",
    "guardian_approval",
    "mentions_v2",
    "personality",
    "in_app_browser",
    "in_app_chat",
    "in_app_dictation",
    "in_app_local_automation",
    "in_app_updates",
    "remote_plugin",
    "plugin_sharing",
    "fast_mode",
];

const CONTROL_TIMEOUT: Duration = Duration::from_secs(30);
const INTERRUPT_GRACE: Duration = Duration::from_secs(3);
const POLL: Duration = Duration::from_millis(100);

/// Each stage of the graceful shutdown waits this long before escalating:
/// stdin close, then SIGTERM, then SIGKILL. Measured (2026-09-17): stdin close
/// or SIGTERM ends the whole codex tree within three seconds.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

/// The app-server is shut down after this long with no completed request. The
/// 2026-09-17 incident ran two idle days holding 1,699 processes; a count cap
/// does nothing while requests are zero, so idle time is the release that does.
const IDLE_TIMEOUT: Duration = Duration::from_secs(600);

/// How often the idle reaper checks the last-activity clock.
const REAPER_TICK: Duration = Duration::from_secs(15);

#[derive(Clone, Debug)]
pub struct CodexConfig {
    /// `codex` on `PATH` by default; an explicit path is used as given.
    pub binary: PathBuf,
    pub model: String,
    /// Working directory handed to every thread. A neutral directory keeps a
    /// project's own AGENTS.md out of the prompt.
    pub cwd: PathBuf,
    /// The `PATH` the CLI is looked for on and run with. `None` is this
    /// account's search (`hide_platform::programs`); a caller that has to
    /// choose it, such as a test with a stand-in login shell, gives one built
    /// by `hide_platform::programs::cli_path_with`.
    pub search_path: Option<OsString>,
    /// The item this backend runs as (PRD agent-profiles-phase2 D-07, D-09):
    /// its variables and wrapper reach the app-server, and its account's
    /// `CODEX_HOME` is where the private home's `auth.json` link points,
    /// while the app-server itself runs under the private home, or under
    /// that account folder where the link cannot be made (`CodexHome`).
    pub run: ItemRun,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            binary: PathBuf::from("codex"),
            model: DEFAULT_MODEL.to_owned(),
            cwd: std::env::temp_dir(),
            search_path: None,
            run: ItemRun::default(),
        }
    }
}

enum Line {
    Message(Value),
    Eof,
}

struct Session {
    child: OwnedChild,
    /// Held in an `Option` so shutdown can close the pipe (its EOF is the
    /// app-server's own shutdown signal) while the child is still owned.
    stdin: Option<ChildStdin>,
    incoming: Receiver<Line>,
    /// Notifications that arrived while a response was awaited.
    pending: VecDeque<Value>,
    next_id: u64,
    /// When the last turn finished, for the idle reaper.
    last_activity: Instant,
    /// The reasoning effort each model takes by default, read from the
    /// server's `model/list` the first time a turn in the account's own
    /// folder names none (see `run_turn`).
    default_efforts: Option<Vec<(String, String)>>,
    /// A private home is removed when the session ends; declared last so it
    /// is deleted after the child has stopped.
    home: CodexHome,
}

pub struct CodexAppServerBackend {
    config: CodexConfig,
    /// One child, one turn at a time: a background feature never competes
    /// with itself for the account's rate limit. Shared with the idle reaper.
    session: Arc<Mutex<Option<Session>>>,
    /// The measurement taken after the most recent turn.
    measurement: Mutex<ProcessMeasurement>,
    reaper_stop: Arc<AtomicBool>,
    reaper: Mutex<Option<JoinHandle<()>>>,
    sink: Arc<dyn AiLogSink>,
}

impl CodexAppServerBackend {
    /// Also sweeps, off this thread, the private homes earlier owners were
    /// killed before removing.
    pub fn new(config: CodexConfig, sink: Arc<dyn AiLogSink>) -> Self {
        codex_home::spawn_sweep(Arc::clone(&sink));
        let session: Arc<Mutex<Option<Session>>> = Arc::new(Mutex::new(None));
        let reaper_stop = Arc::new(AtomicBool::new(false));
        let reaper = spawn_idle_reaper(
            Arc::clone(&session),
            Arc::clone(&sink),
            Arc::clone(&reaper_stop),
        );
        Self {
            config,
            sink,
            session,
            measurement: Mutex::new(ProcessMeasurement::Unavailable),
            reaper_stop,
            reaper: Mutex::new(Some(reaper)),
        }
    }

    /// The exact argument vector the child is started with in a private
    /// home. The settings past the disabled features change nothing there,
    /// where no `config.toml` is read: they are what keeps the request the
    /// same in the account's own folder (with-hide/hide#2531, measured on
    /// codex-cli 0.155.1 and 0.167.2): OpenAI stays the provider over the
    /// user's `model_provider`, no `notify` command runs after a turn, and
    /// no skills catalog (the account's or `~/.agents/skills`) is sent.
    pub fn spawn_arguments() -> Vec<String> {
        Self::arguments(Vec::new())
    }

    /// The argument vector in the account's own folder, which reads the
    /// user's `config.toml`: every MCP server `servers` names is switched
    /// off, and the user's developer instructions are not sent. One inline
    /// table switches them off, with each name quoted inside the TOML value,
    /// because `-c mcp_servers.<name>.enabled=false` splits a name holding a
    /// `.` and `-c mcp_servers={}` adds nothing to the user's table (both
    /// measured). Hooks are off on both paths (`--disable hooks`).
    fn account_arguments(servers: &[String]) -> Vec<String> {
        let mut extra = vec!["-c".to_owned(), "developer_instructions=\"\"".to_owned()];
        if !servers.is_empty() {
            let off: Vec<String> = servers
                .iter()
                .map(|name| format!("{}={{enabled=false}}", Value::from(name.as_str())))
                .collect();
            extra.push("-c".to_owned());
            extra.push(format!("mcp_servers={{{}}}", off.join(",")));
        }
        Self::arguments(extra)
    }

    fn arguments(extra: Vec<String>) -> Vec<String> {
        let mut args = vec!["app-server".to_owned()];
        for setting in [
            "web_search=\"disabled\"",
            "model_provider=\"openai\"",
            "notify=[]",
            "skills.include_instructions=false",
        ] {
            args.push("-c".to_owned());
            args.push(setting.to_owned());
        }
        for feature in DISABLED_FEATURES {
            args.push("--disable".to_owned());
            args.push((*feature).to_owned());
        }
        args.extend(extra);
        args.push("--listen".to_owned());
        args.push("stdio://".to_owned());
        args
    }

    fn resolved_binary(&self) -> Option<Program> {
        self.config
            .run
            .program(&self.config.binary, self.config.search_path.as_deref())
    }

    fn lock(&self) -> MutexGuard<'_, Option<Session>> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn ensure_session<'a>(
        &self,
        guard: &'a mut Option<Session>,
    ) -> Result<&'a mut Session, AiError> {
        if guard.as_mut().is_some_and(|session| session.is_alive()) {
            return Ok(guard.as_mut().expect("checked above"));
        }
        let binary = self
            .resolved_binary()
            .ok_or_else(|| AiError::ProviderUnavailable("codex_not_installed".to_owned()))?;
        let run = &self.config.run;
        let home = CodexHome::create(&|name| run.variable(name))?;
        let session = Session::spawn(&binary, &self.config.cwd, run, home)?;
        if let CodexHome::Account(_) = session.home {
            let mut event = AiLogEvent::new("ai.codex_home.account");
            event.provider = Some(ProviderId::CODEX);
            event.app_server_pid = Some(session.child.id());
            event.detail = Some("reason=link_needs_privilege".to_owned());
            self.sink.log(event);
        }
        *guard = Some(session);
        Ok(guard.as_mut().expect("just inserted"))
    }

    fn run_turn(
        &self,
        session: &mut Session,
        request: &AiRequest,
        cancel: &CancelToken,
    ) -> Result<AiResponse, AiError> {
        let deadline = Instant::now() + request.deadline;
        // `PlainJson` names the schema in the instructions and sends no
        // `outputSchema`, as the Claude backend does: the API's strict mode
        // refuses keywords a feature's schema may use (`oneOf`, `uniqueItems`:
        // HTTP 400 `invalid_json_schema`, codex-cli 0.163.0), and the router
        // validates the answer against the full schema either way.
        let instructions = match request.answer {
            AnswerMode::Schema => request.system_prompt().into_owned(),
            AnswerMode::PlainJson => {
                system_with_schema(&request.system_prompt(), &request.output_schema)
            }
        };
        let thread = session.request(
            "thread/start",
            json!({
                "ephemeral": true,
                "sandbox": "read-only",
                "approvalPolicy": "never",
                "cwd": self.config.cwd,
                "model": request.model(&self.config.model),
                "baseInstructions": instructions,
            }),
            CONTROL_TIMEOUT,
        )?;
        let thread_id = thread
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| AiError::Transient("thread_start_without_id".to_owned()))?
            .to_owned();
        // The model the server resolved for this thread, which is what the
        // usage ledger prices (D-11); the configured name may be an alias.
        let model = thread
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned);
        // Once this is written the server may be running the turn. A lost
        // answer no longer says whether it completed, and only that answer
        // would make a retry safe; a rejection still does.
        let mut turn_start = json!({
            "threadId": thread_id,
            "input": [{"type": "text", "text": request.input}],
            "clientUserMessageId": request.request_id.0,
        });
        if request.answer == AnswerMode::Schema {
            turn_start["outputSchema"] = request.output_schema.clone();
        }
        let effort = match request.effort(ProviderId::CODEX) {
            Some(effort) => Some(effort.to_owned()),
            // The account's own folder reads the user's
            // `model_reasoning_effort`; a private home runs the model at its
            // default, so a turn there is given that default by name.
            None if matches!(session.home, CodexHome::Account(_)) => {
                session.default_effort(request.model(&self.config.model))?
            }
            None => None,
        };
        if let Some(effort) = effort {
            turn_start["effort"] = json!(effort);
        }
        let turn = session
            .request_raw("turn/start", turn_start, CONTROL_TIMEOUT)
            .map_err(RequestFailure::after_submission)?;
        let turn_id = turn
            .pointer("/turn/id")
            .and_then(Value::as_str)
            .ok_or_else(|| AiError::CompletionUnknown("turn_start_without_id".to_owned()))?
            .to_owned();

        let mut usage = AiUsage::default();
        let mut total: Option<Value> = None;
        let mut text: Option<String> = None;
        loop {
            if cancel.is_cancelled() {
                session.interrupt(&thread_id, &turn_id);
                return Err(AiError::Cancelled);
            }
            let now = Instant::now();
            if now >= deadline {
                session.interrupt(&thread_id, &turn_id);
                return Err(AiError::Timeout);
            }
            let message = match session.next_message(POLL.min(deadline - now)) {
                Ok(message) => message,
                Err(Wait::Elapsed) => continue,
                Err(Wait::Gone(error)) => {
                    return Err(AiError::CompletionUnknown(error.to_string()));
                }
            };
            let Some(method) = message.get("method").and_then(Value::as_str) else {
                continue;
            };
            let params = message.get("params").cloned().unwrap_or(Value::Null);
            if params.get("threadId").and_then(Value::as_str) != Some(thread_id.as_str()) {
                continue;
            }
            match method {
                "thread/tokenUsage/updated" => {
                    let last = params.pointer("/tokenUsage/last");
                    usage.input_tokens = last
                        .and_then(|v| v.get("inputTokens"))
                        .and_then(Value::as_u64);
                    usage.output_tokens = last
                        .and_then(|v| v.get("outputTokens"))
                        .and_then(Value::as_u64);
                    // Every request is a new thread with one turn, so the
                    // running total of the last update is this call's sum.
                    total = params.pointer("/tokenUsage/total").cloned();
                }
                "item/completed" => {
                    if params.pointer("/item/type").and_then(Value::as_str) == Some("agentMessage")
                    {
                        text = params
                            .pointer("/item/text")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                    }
                }
                "turn/completed" => {
                    let status = params.pointer("/turn/status").and_then(Value::as_str);
                    return match status {
                        Some("completed") => {
                            let text = text.ok_or_else(|| {
                                AiError::InvalidOutput("turn_completed_without_message".to_owned())
                            })?;
                            // Under `outputSchema` the message is the object;
                            // without it the model may fence it, so it is read
                            // the way the other plain-JSON readers read it.
                            let value = match request.answer {
                                AnswerMode::Schema => serde_json::from_str::<Value>(&text).ok(),
                                AnswerMode::PlainJson => json_from_text(&text),
                            }
                            .ok_or_else(|| AiError::InvalidOutput("message_not_json".to_owned()))?;
                            usage.models = model
                                .as_deref()
                                .zip(total.as_ref())
                                .and_then(|(model, total)| ledger_tokens(model, total))
                                .into_iter()
                                .collect();
                            Ok(AiResponse { value, usage })
                        }
                        Some("failed") => Err(self.classify_turn_error(
                            session,
                            params.pointer("/turn/error").unwrap_or(&Value::Null),
                        )),
                        Some("interrupted") => Err(AiError::Cancelled),
                        other => Err(AiError::InvalidOutput(format!(
                            "turn_status_{}",
                            other.unwrap_or("unknown")
                        ))),
                    };
                }
                _ => {}
            }
        }
    }

    /// Maps the typed `codexErrorInfo` to the caller's failure classes. No
    /// prose is inspected: the enum is the contract, and an `other` failure's
    /// API error body is read only for its numeric status and its code.
    fn classify_turn_error(&self, session: &mut Session, error: &Value) -> AiError {
        let info = error.get("codexErrorInfo").unwrap_or(&Value::Null);
        // A response stream existed, so model work may already have run.
        // Its HTTP status cannot make this safe to submit a second time.
        if info.get("responseStreamDisconnected").is_some() {
            return AiError::CompletionUnknown("codex_response_stream_disconnected".to_owned());
        }
        let connection_failure = ["httpConnectionFailed", "responseStreamConnectionFailed"]
            .iter()
            .find_map(|key| info.get(key));
        let refusal = (info.as_str() == Some("other"))
            .then(|| api_refusal(error))
            .flatten();
        let http_status = connection_failure
            .and_then(|failure| failure.get("httpStatusCode"))
            .and_then(Value::as_u64)
            .or(refusal.as_ref().map(|(status, _)| *status));
        match (info.as_str(), http_status) {
            (Some("usageLimitExceeded" | "rateLimitExceeded"), _) | (_, Some(429)) => {
                AiError::UsageLimited {
                    retry_after: session.retry_after(),
                }
            }
            (Some("unauthorized"), _) | (_, Some(401 | 403)) => AiError::NotAuthenticated,
            (Some(code @ ("contextWindowExceeded" | "badRequest")), _) => {
                AiError::Rejected(format!("codex_{code}"))
            }
            // The API refused the request itself, before any model work: a
            // schema it does not take, a malformed parameter.
            (Some("other"), Some(status @ 400..=499)) => {
                let code = refusal.and_then(|(_, code)| code);
                AiError::Rejected(match code {
                    Some(code) => format!("codex_request_refused:http_{status}:{code}"),
                    None => format!("codex_request_refused:http_{status}"),
                })
            }
            (Some("sessionBudgetExceeded"), _) => {
                AiError::InvalidOutput("codex_sessionBudgetExceeded".to_owned())
            }
            (Some(code), _) => AiError::CompletionUnknown(format!("codex_{code}")),
            (None, Some(status)) => AiError::Transient(format!("codex_http_{status}")),
            (None, None) if connection_failure.is_some() => {
                AiError::Transient("codex_connection_failed".to_owned())
            }
            (None, None) => AiError::CompletionUnknown("codex_turn_failed".to_owned()),
        }
    }
}

/// The status and code of the API error body an `other` turn failure
/// carries as its `message` (codex-cli 0.163.0 forwards the Responses API's
/// own frame: `{"type":"error","error":{"code":...},"status":400}`). The
/// code is kept only when it is a short identifier, so no free text reaches
/// the log.
fn api_refusal(error: &Value) -> Option<(u64, Option<String>)> {
    let body: Value = serde_json::from_str(error.get("message")?.as_str()?).ok()?;
    let status = body.get("status")?.as_u64()?;
    let code = body
        .pointer("/error/code")
        .and_then(Value::as_str)
        .filter(|code| {
            !code.is_empty()
                && code.len() <= 64
                && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .map(str::to_owned);
    Some((status, code))
}

impl AiBackend for CodexAppServerBackend {
    fn id(&self) -> ProviderId {
        ProviderId::CODEX
    }

    /// The app-server is resident, so its descendants and size are measured
    /// after each turn and held to the router's caps.
    fn measurable(&self) -> bool {
        true
    }

    fn availability(&self) -> Availability {
        if self.resolved_binary().is_none() {
            return Availability::NotInstalled;
        }
        let mut guard = self.lock();
        let session = match self.ensure_session(&mut guard) {
            Ok(session) => session,
            Err(error) => {
                return Availability::Unavailable {
                    reason: error.to_string(),
                };
            }
        };
        let account = match session.request("account/read", json!({}), CONTROL_TIMEOUT) {
            Ok(account) => account,
            Err(error) => {
                return Availability::Unavailable {
                    reason: error.to_string(),
                };
            }
        };
        if account.get("account").is_none_or(Value::is_null) {
            return Availability::NeedsLogin;
        }
        match session.request("model/list", json!({}), CONTROL_TIMEOUT) {
            Ok(models) => {
                let offered = models
                    .get("data")
                    .and_then(Value::as_array)
                    .is_some_and(|models| {
                        models.iter().any(|m| {
                            m.get("id").and_then(Value::as_str) == Some(&self.config.model)
                        })
                    });
                if offered {
                    Availability::Ready
                } else {
                    Availability::Unavailable {
                        reason: format!("model_not_offered:{}", self.config.model),
                    }
                }
            }
            Err(error) => Availability::Unavailable {
                reason: error.to_string(),
            },
        }
    }

    /// `model/list` is the app-server's own answer, so nothing above this
    /// backend keeps a list of Codex models.
    fn models(&self) -> ModelCatalog {
        if self.resolved_binary().is_none() {
            return ModelCatalog::Unknown {
                reason: "codex_not_installed".to_owned(),
            };
        }
        let mut guard = self.lock();
        let session = match self.ensure_session(&mut guard) {
            Ok(session) => session,
            Err(error) => {
                return ModelCatalog::Unknown {
                    reason: error.to_string(),
                };
            }
        };
        match session.request("model/list", json!({}), CONTROL_TIMEOUT) {
            Ok(models) => match models.get("data").and_then(Value::as_array) {
                Some(offered) => ModelCatalog::plain(
                    offered
                        .iter()
                        .filter_map(|model| model.get("id").and_then(Value::as_str)),
                ),
                // The field is the contract; without it the list is unknown
                // rather than empty.
                None => ModelCatalog::Unknown {
                    reason: "model_list_without_data".to_owned(),
                },
            },
            Err(error) => ModelCatalog::Unknown {
                reason: error.to_string(),
            },
        }
    }

    fn execute(&self, request: &AiRequest, cancel: &CancelToken) -> Result<AiResponse, AiError> {
        let mut guard = self.lock();
        let session = self.ensure_session(&mut guard)?;
        let pid = session.child.id();
        let result = self.run_turn(session, request, cancel);
        session.last_activity = Instant::now();
        // A refusal or a lost connection means the child is gone; measure the
        // live tree otherwise, while the lock still holds the session still.
        let child_gone = matches!(
            result,
            Err(AiError::ProviderUnavailable(_) | AiError::CompletionUnknown(_))
        );
        *self.measurement.lock().unwrap_or_else(|e| e.into_inner()) = if child_gone {
            ProcessMeasurement::Unavailable
        } else {
            process::measure(pid)
        };
        if child_gone {
            // Take the dead session out, release the lock, then drop it: its
            // shutdown runs `Session::terminate` and a lost-but-not-yet-dead
            // child can hold that for the full graceful grace, which must not
            // block the mutex other callers (the idle reaper, `restart`, the
            // backend's own `Drop`) wait on. The next request starts a fresh
            // one.
            let dead = guard.take();
            drop(guard);
            drop(dead);
        }
        result
    }

    fn last_measurement(&self) -> ProcessMeasurement {
        *self.measurement.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn restart(&self) {
        let victim = self.lock().take();
        // Drop outside the lock: shutdown may take a few seconds and no other
        // request can start one meanwhile.
        drop(victim);
        *self.measurement.lock().unwrap_or_else(|e| e.into_inner()) =
            ProcessMeasurement::Unavailable;
    }
}

impl Drop for CodexAppServerBackend {
    fn drop(&mut self) {
        self.reaper_stop.store(true, Ordering::SeqCst);
        // End the child now rather than waiting for the reaper to notice.
        let victim = self.lock().take();
        drop(victim);
        if let Some(handle) = self.reaper.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = handle.join();
        }
    }
}

/// Watches the last-activity clock and ends the app-server once it has been
/// idle past [`IDLE_TIMEOUT`], releasing it during the quiet the 2026-09-17
/// incident held it through.
#[allow(clippy::disallowed_methods)] // a production wait, not test code
fn spawn_idle_reaper(
    session: Arc<Mutex<Option<Session>>>,
    sink: Arc<dyn AiLogSink>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("hide-ai-codex-idle".to_owned())
        .spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                // Sleep in short steps so the thread joins promptly on drop
                // (a CLI that builds a router and exits must not wait a whole
                // tick), while still only checking idle once per tick.
                let mut waited = Duration::ZERO;
                while waited < REAPER_TICK {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    waited += Duration::from_millis(200);
                }
                let victim = {
                    let mut guard = session.lock().unwrap_or_else(|e| e.into_inner());
                    match guard.as_ref() {
                        Some(active) if active.last_activity.elapsed() >= IDLE_TIMEOUT => {
                            guard.take()
                        }
                        _ => None,
                    }
                };
                if let Some(active) = victim {
                    let pid = active.child.id();
                    // Drop outside the lock so the graceful shutdown does not
                    // hold requests.
                    drop(active);
                    let mut event = AiLogEvent::new("ai.app_server.idle_exit");
                    event.provider = Some(ProviderId::CODEX);
                    event.app_server_pid = Some(pid);
                    event.detail = Some(format!("idle_s={}", IDLE_TIMEOUT.as_secs()));
                    sink.log(event);
                }
            }
        })
        .expect("idle reaper thread")
}

enum Wait {
    Elapsed,
    Gone(AiError),
}

/// How a control request ended without a result. Only a rejection proves
/// the server declined the request; the other two leave its fate open,
/// which matters once the request is a turn.
enum RequestFailure {
    /// The server answered with a JSON-RPC error.
    Rejected { method: &'static str, code: i64 },
    /// No answer before the control timeout.
    Unanswered { method: &'static str },
    /// The child or its connection is gone.
    Gone(AiError),
}

impl RequestFailure {
    /// The classification for a request the server has not acted on yet.
    fn before_submission(self) -> AiError {
        match self {
            Self::Rejected { method, code } => {
                AiError::Transient(format!("rpc_error:{method}:{code}"))
            }
            Self::Unanswered { method } => AiError::Transient(format!("control_timeout:{method}")),
            Self::Gone(error) => error,
        }
    }

    /// The classification once the request may already be running.
    fn after_submission(self) -> AiError {
        match self {
            Self::Rejected { .. } => self.before_submission(),
            Self::Unanswered { method } => {
                AiError::CompletionUnknown(format!("control_timeout:{method}"))
            }
            Self::Gone(error) => AiError::CompletionUnknown(error.to_string()),
        }
    }
}

/// The names of the MCP servers the account's `config.toml` declares, as
/// `codex mcp list --json` reports them, read in that folder with the item's
/// variables: what the app-server would start there. A list that cannot be
/// read refuses the start, since then no server could be switched off.
fn mcp_servers(
    program: &Program,
    cwd: &Path,
    run: &ItemRun,
    home: &Path,
) -> Result<Vec<String>, AiError> {
    let mut set = run.set();
    set.push(("CODEX_HOME", home.display().to_string()));
    let args = ["mcp", "list", "--json"].map(str::to_owned);
    let listed = runner::run(
        &runner::Spec {
            binary: program,
            args: &args,
            cwd,
            stdin: None,
            environment: runner::Environment::Inherit,
            set: &set,
            deadline: PROBE_TIMEOUT,
        },
        &CancelToken::new(),
    )
    .map_err(|error| AiError::ProviderUnavailable(error.diagnostic("codex_mcp_list")))?;
    if !listed.succeeded() {
        return Err(AiError::ProviderUnavailable(format!(
            "codex_mcp_list_failed:exit_{}",
            listed.exit()
        )));
    }
    let unreadable = || AiError::ProviderUnavailable("codex_mcp_list_unreadable".to_owned());
    serde_json::from_str::<Value>(&listed.stdout)
        .ok()
        .and_then(|listed| {
            listed
                .as_array()?
                .iter()
                .map(|server| server.get("name")?.as_str().map(str::to_owned))
                .collect::<Option<Vec<String>>>()
        })
        .ok_or_else(unreadable)
}

impl Session {
    /// Starts the app-server in `home`. In the account's own folder the MCP
    /// servers are listed right before the start, and a start that fails is
    /// tried once more when the list read again differs: a server switched
    /// off by a name the config no longer holds makes Codex refuse to start
    /// (`invalid transport`, measured), so an edit landing between the list
    /// and the start must not stop Hide AI.
    fn spawn(
        program: &Program,
        cwd: &Path,
        run: &ItemRun,
        home: CodexHome,
    ) -> Result<Self, AiError> {
        let account = match &home {
            CodexHome::Private(_) => {
                return Self::start(
                    program,
                    cwd,
                    run,
                    home,
                    &CodexAppServerBackend::spawn_arguments(),
                );
            }
            CodexHome::Account(path) => path.clone(),
        };
        let servers = mcp_servers(program, cwd, run, &account)?;
        let args = CodexAppServerBackend::account_arguments(&servers);
        match Self::start(program, cwd, run, home, &args) {
            Err(error) => {
                let again = mcp_servers(program, cwd, run, &account)?;
                if again == servers {
                    return Err(error);
                }
                let args = CodexAppServerBackend::account_arguments(&again);
                Self::start(program, cwd, run, CodexHome::Account(account), &args)
            }
            started => started,
        }
    }

    fn start(
        program: &Program,
        cwd: &Path,
        run: &ItemRun,
        home: CodexHome,
        args: &[String],
    ) -> Result<Self, AiError> {
        let mut command = program.command();
        program.apply(&mut command);
        // The item's variables, then the home over its own: in a private home
        // the account is reached through the `auth.json` link alone.
        for (key, value) in &run.env {
            command.env(key, value);
        }
        command
            .args(args)
            .current_dir(cwd)
            .env("CODEX_HOME", home.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = OwnedChild::spawn(&mut command).map_err(|error| {
            AiError::ProviderUnavailable(format!("app_server_spawn_failed:{}", error.kind()))
        })?;
        let stdin = child
            .take_stdin()
            .ok_or_else(|| AiError::ProviderUnavailable("app_server_no_stdin".to_owned()))?;
        let stdout = child
            .take_stdout()
            .ok_or_else(|| AiError::ProviderUnavailable("app_server_no_stdout".to_owned()))?;
        let (sender, incoming) = mpsc::channel();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines() {
                match line {
                    Ok(line) => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        if let Ok(value) = serde_json::from_str::<Value>(&line)
                            && sender.send(Line::Message(value)).is_err()
                        {
                            return;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = sender.send(Line::Eof);
        });
        let mut session = Self {
            child,
            stdin: Some(stdin),
            incoming,
            pending: VecDeque::new(),
            next_id: 0,
            last_activity: Instant::now(),
            default_efforts: None,
            home,
        };
        session.request(
            "initialize",
            json!({"clientInfo": {"name": "hide-ai", "version": env!("CARGO_PKG_VERSION")}}),
            CONTROL_TIMEOUT,
        )?;
        session.notify("initialized", json!({}))?;
        Ok(session)
    }

    fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    fn write(&mut self, message: &Value) -> Result<(), AiError> {
        let mut line = serde_json::to_vec(message)
            .map_err(|_| AiError::Transient("request_not_encodable".to_owned()))?;
        line.push(b'\n');
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| AiError::ProviderUnavailable("app_server_stdin_closed".to_owned()))?;
        stdin
            .write_all(&line)
            .and_then(|()| stdin.flush())
            .map_err(|error| {
                AiError::ProviderUnavailable(format!("app_server_write_failed:{}", error.kind()))
            })
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), AiError> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// A control request the server has not acted on: any failure is a
    /// refusal and safe to repeat.
    fn request(
        &mut self,
        method: &'static str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, AiError> {
        self.request_raw(method, params, timeout)
            .map_err(RequestFailure::before_submission)
    }

    fn request_raw(
        &mut self,
        method: &'static str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, RequestFailure> {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .map_err(RequestFailure::Gone)?;
        let deadline = Instant::now() + timeout;
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(RequestFailure::Unanswered { method });
            }
            let message = match self.recv(deadline - now) {
                Ok(message) => message,
                Err(Wait::Elapsed) => continue,
                Err(Wait::Gone(error)) => return Err(RequestFailure::Gone(error)),
            };
            if message.get("id").and_then(Value::as_u64) == Some(id)
                && message.get("method").is_none()
            {
                if let Some(error) = message.get("error") {
                    let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
                    return Err(RequestFailure::Rejected { method, code });
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
            self.route(message).map_err(RequestFailure::Gone)?;
        }
    }

    /// Next notification, draining what a control request set aside first.
    fn next_message(&mut self, timeout: Duration) -> Result<Value, Wait> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(message);
        }
        let message = self.recv(timeout)?;
        if message.get("id").is_some() && message.get("method").is_some() {
            self.route(message).map_err(Wait::Gone)?;
            return Err(Wait::Elapsed);
        }
        Ok(message)
    }

    fn recv(&mut self, timeout: Duration) -> Result<Value, Wait> {
        match self.incoming.recv_timeout(timeout) {
            Ok(Line::Message(message)) => Ok(message),
            Ok(Line::Eof) | Err(RecvTimeoutError::Disconnected) => {
                let status = match self.child.try_wait() {
                    Ok(Some(status)) => {
                        status.code().map_or("signal".to_owned(), |c| c.to_string())
                    }
                    _ => "unknown".to_owned(),
                };
                Err(Wait::Gone(AiError::ProviderUnavailable(format!(
                    "app_server_exited:{status}"
                ))))
            }
            Err(RecvTimeoutError::Timeout) => Err(Wait::Elapsed),
        }
    }

    /// A server-to-client request (an approval) cannot happen under
    /// `approvalPolicy: never` with a read-only sandbox; refuse it so the
    /// server never waits on us. Notifications are kept for the turn loop.
    fn route(&mut self, message: Value) -> Result<(), AiError> {
        if let (Some(id), Some(_)) = (message.get("id"), message.get("method")) {
            let id = id.clone();
            return self.write(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": "hide-ai does not serve requests"},
            }));
        }
        self.pending.push_back(message);
        Ok(())
    }

    /// Asks the server to stop the turn and waits briefly for its completion
    /// so the next request starts on a quiet connection. The child lives on.
    fn interrupt(&mut self, thread_id: &str, turn_id: &str) {
        let _ = self.request(
            "turn/interrupt",
            json!({"threadId": thread_id, "turnId": turn_id}),
            INTERRUPT_GRACE,
        );
        let deadline = Instant::now() + INTERRUPT_GRACE;
        while Instant::now() < deadline {
            match self.next_message(POLL) {
                Ok(message)
                    if message.get("method").and_then(Value::as_str) == Some("turn/completed")
                        && message.pointer("/params/threadId").and_then(Value::as_str)
                            == Some(thread_id) =>
                {
                    return;
                }
                Ok(_) | Err(Wait::Elapsed) => {}
                Err(Wait::Gone(_)) => return,
            }
        }
    }

    /// The reasoning effort `model` takes by default, as this server's
    /// `model/list` says (`defaultReasoningEffort`), read once per session;
    /// `None` for a model the list does not carry or gives no default.
    fn default_effort(&mut self, model: &str) -> Result<Option<String>, AiError> {
        if self.default_efforts.is_none() {
            let listed = self.request("model/list", json!({}), CONTROL_TIMEOUT)?;
            let models = listed
                .get("data")
                .and_then(Value::as_array)
                .ok_or_else(|| AiError::Transient("model_list_without_data".to_owned()))?;
            let efforts = models
                .iter()
                .filter_map(|listed| {
                    Some((
                        listed.get("id")?.as_str()?.to_owned(),
                        listed.get("defaultReasoningEffort")?.as_str()?.to_owned(),
                    ))
                })
                .collect();
            self.default_efforts = Some(efforts);
        }
        Ok(self
            .default_efforts
            .iter()
            .flatten()
            .find(|(id, _)| id == model)
            .map(|(_, effort)| effort.clone()))
    }

    /// Seconds until the primary window resets, when the server knows.
    fn retry_after(&mut self) -> Option<Duration> {
        let limits = self
            .request("account/rateLimits/read", json!({}), CONTROL_TIMEOUT)
            .ok()?;
        let resets_at = limits
            .pointer("/rateLimits/primary/resetsAt")
            .and_then(Value::as_u64)?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
        Some(Duration::from_secs(resets_at.saturating_sub(now)))
    }

    /// Ends the child through one path: close stdin (its EOF is the
    /// app-server's shutdown signal and ends the whole tree), then escalate to
    /// SIGTERM and finally SIGKILL if it has not exited. Every exit path -
    /// `Drop`, a session swap, an over-budget restart - goes through here.
    fn terminate(&mut self) {
        let pid = self.child.id();
        // 1. stdin EOF: the app-server's own graceful shutdown.
        self.stdin = None;
        if self.wait_for_exit(SHUTDOWN_GRACE) {
            // Whatever the wrapper left in its group goes too: a helper that
            // outlived its parent is the leak the process cap exists for.
            let _ = self.child.kill_tree();
            return;
        }
        // 2. SIGTERM (an immediate end where the system has no polite one).
        let _ = platform::terminate(pid);
        if self.wait_for_exit(SHUTDOWN_GRACE) {
            let _ = self.child.kill_tree();
            return;
        }
        // 3. SIGKILL for the wrapper and everything it started, and reap so
        // no zombie is left.
        let _ = self.child.kill_tree();
        let _ = self.child.wait();
    }

    #[allow(clippy::disallowed_methods)] // a production wait, not test code
    fn wait_for_exit(&mut self, grace: Duration) -> bool {
        let deadline = Instant::now() + grace;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return true,
                Ok(None) => {}
                Err(_) => return false,
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(
                POLL.min(deadline - Instant::now())
                    .max(Duration::from_millis(1)),
            );
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// What the usage ledger counts from a thread's running total, or `None` when
/// the total does not give it (D-11). `inputTokens` includes both cache
/// categories, so they come out of it; reasoning is part of output. Required
/// counts are input and output; a missing cache count is zero, and a
/// subtraction below zero is an inconsistent report, not a count.
fn ledger_tokens(model: &str, total: &Value) -> Option<ModelTokens> {
    let count = |key: &str| total.get(key).and_then(Value::as_u64);
    let cache_read = count("cachedInputTokens").unwrap_or(0);
    let cache_write = count("cacheWriteInputTokens").unwrap_or(0);
    let input = count("inputTokens")?
        .checked_sub(cache_read)?
        .checked_sub(cache_write)?;
    let output = count("outputTokens")?;
    let reasoning = count("reasoningOutputTokens");
    if reasoning.is_some_and(|reasoning| reasoning > output) {
        return None;
    }
    Some(ModelTokens {
        model: model.to_owned(),
        input,
        output,
        cache_read,
        cache_write,
        cache_write_1h: 0,
        reasoning,
        lower_bound: false,
    })
}

/// The account's own folder (with-hide/hide#2531), which no Unix account is
/// refused a link for, so these start the stand-in app-server there by
/// hand; whether a machine takes that folder is `codex_home`'s test. The
/// stand-in's switches go through the item's variables, so no test here
/// changes this process's environment.
#[cfg(all(test, unix))]
mod account_home_tests {
    use super::*;
    use crate::NoopLogSink;

    struct Account {
        dir: tempfile::TempDir,
    }

    impl Account {
        /// An account folder whose `config.toml` declares `servers`.
        fn declaring(servers: &[&str]) -> Self {
            let dir = tempfile::tempdir().expect("a test folder is creatable");
            std::fs::create_dir(dir.path().join("codex")).expect("the account folder is creatable");
            std::fs::write(dir.path().join("mcp.json"), json!(servers).to_string())
                .expect("the server list writes");
            Self { dir }
        }

        fn folder(&self) -> PathBuf {
            self.dir.path().join("codex")
        }

        fn file(&self, name: &str) -> String {
            self.dir.path().join(name).display().to_string()
        }

        fn run(&self, switches: &[(&str, &str)]) -> ItemRun {
            let mut env = vec![
                ("FAKE_MCP_FILE".to_owned(), self.file("mcp.json")),
                ("FAKE_STARTS_FILE".to_owned(), self.file("starts")),
                ("FAKE_ENV_FILE".to_owned(), self.file("env.json")),
                ("FAKE_TURN_FILE".to_owned(), self.file("turn.json")),
            ];
            env.extend(
                switches
                    .iter()
                    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned())),
            );
            ItemRun { env, program: None }
        }

        fn spawn(&self, run: &ItemRun) -> Result<Session, AiError> {
            Session::spawn(
                &fake(),
                &std::env::temp_dir(),
                run,
                CodexHome::Account(self.folder()),
            )
        }

        /// The argument vector of every app-server started, in order.
        fn starts(&self) -> Vec<Vec<String>> {
            match std::fs::read_to_string(self.file("starts")) {
                Ok(text) => text
                    .lines()
                    .map(|line| serde_json::from_str(line).expect("a start is recorded as JSON"))
                    .collect(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(error) => panic!("the start record is unreadable: {error}"),
            }
        }

        fn recorded(&self, name: &str) -> Value {
            serde_json::from_str(&std::fs::read_to_string(self.file(name)).expect("recorded"))
                .expect("recorded as JSON")
        }
    }

    fn fake() -> Program {
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-app-server.py");
        Program::resolve(&fake, None).expect("the stand-in is a file")
    }

    /// The value following each `-c`.
    fn settings(args: &[String]) -> Vec<&str> {
        args.windows(2)
            .filter(|pair| pair[0] == "-c")
            .map(|pair| pair[1].as_str())
            .collect()
    }

    fn turn_request(efforts: Vec<(ProviderId, String)>) -> AiRequest {
        AiRequest {
            feature_id: "fixture".into(),
            request_id: crate::RequestId("req-1".to_owned()),
            subject_id: "pane-1".to_owned(),
            system: "Answer as JSON.".to_owned(),
            input: "hello".to_owned(),
            output_schema: json!({"type": "object"}),
            deadline: Duration::from_secs(10),
            schema_version: "fixture.v1".into(),
            pick: None,
            output_language: None,
            answer: AnswerMode::default(),
            provider_efforts: efforts,
        }
    }

    #[test]
    fn the_account_folder_runs_with_its_servers_hooks_and_notify_off() {
        let account = Account::declaring(&["docs", "acme.search"]);
        let mut session = account
            .spawn(&account.run(&[]))
            .expect("the app-server starts");
        session
            .request("model/list", json!({}), CONTROL_TIMEOUT)
            .expect("the models are listed");
        assert_eq!(
            account.recorded("env.json")["CODEX_HOME"],
            account.folder().display().to_string(),
        );
        drop(session);

        let starts = account.starts();
        assert_eq!(starts.len(), 1, "one app-server start: {starts:?}");
        let settings = settings(&starts[0]);
        for expected in [
            r#"mcp_servers={"docs"={enabled=false},"acme.search"={enabled=false}}"#,
            "notify=[]",
            r#"model_provider="openai""#,
            "skills.include_instructions=false",
            r#"developer_instructions="""#,
        ] {
            assert!(
                settings.contains(&expected),
                "{expected} missing from {settings:?}"
            );
        }
        assert!(
            starts[0]
                .windows(2)
                .any(|pair| pair == ["--disable", "hooks"]),
            "hooks stay off: {:?}",
            starts[0]
        );
        assert!(
            account.folder().is_dir(),
            "the account's folder outlives the session"
        );
    }

    #[test]
    fn a_server_removed_after_the_listing_is_listed_again_and_the_start_tried_once_more() {
        let account = Account::declaring(&["docs", "gone"]);
        let run = account.run(&[("FAKE_MCP_AFTER_LIST", r#"["docs"]"#)]);
        account.spawn(&run).expect("the second start succeeds");

        let starts = account.starts();
        assert_eq!(starts.len(), 2, "{starts:?}");
        assert!(settings(&starts[1]).contains(&r#"mcp_servers={"docs"={enabled=false}}"#));
    }

    #[test]
    fn a_start_that_fails_with_the_same_servers_listed_is_not_tried_again() {
        let account = Account::declaring(&["docs"]);
        let run = account.run(&[("FAKE_EXIT_AT_START", "1")]);
        assert!(account.spawn(&run).is_err());
        assert_eq!(account.starts().len(), 1);
    }

    #[test]
    fn a_server_list_codex_cannot_read_starts_no_app_server() {
        let account = Account::declaring(&["docs"]);
        let run = account.run(&[("FAKE_MCP_LIST_FAIL", "1")]);
        let Err(error) = account.spawn(&run) else {
            panic!("no server could be switched off, so nothing starts");
        };
        assert_eq!(
            error,
            AiError::ProviderUnavailable("codex_mcp_list_failed:exit_1".to_owned())
        );
        assert!(account.starts().is_empty());
    }

    /// The account's `model_reasoning_effort` would apply to a turn that
    /// names none, so the turn names the model's default instead; an effort
    /// the feature chose still wins.
    #[test]
    fn a_turn_in_the_account_folder_names_the_models_default_effort_unless_one_was_chosen() {
        let account = Account::declaring(&[]);
        let run = account.run(&[("FAKE_DEFAULT_EFFORT", "low")]);
        let backend = CodexAppServerBackend::new(
            CodexConfig {
                binary: PathBuf::from("codex"),
                model: DEFAULT_MODEL.to_owned(),
                cwd: std::env::temp_dir(),
                search_path: None,
                run: run.clone(),
            },
            Arc::new(NoopLogSink),
        );
        let mut session = account.spawn(&run).expect("the app-server starts");
        assert!(
            !settings(&account.starts()[0])
                .iter()
                .any(|setting| setting.starts_with("mcp_servers=")),
            "no server is named when none is declared"
        );

        backend
            .run_turn(&mut session, &turn_request(Vec::new()), &CancelToken::new())
            .expect("the turn completes");
        assert_eq!(account.recorded("turn.json")["effort"], "low");

        let chosen = vec![(ProviderId::CODEX, "high".to_owned())];
        backend
            .run_turn(&mut session, &turn_request(chosen), &CancelToken::new())
            .expect("the turn completes");
        assert_eq!(account.recorded("turn.json")["effort"], "high");
    }
}
