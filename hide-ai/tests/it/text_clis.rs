//! Drives the Grok, Pi and Antigravity backends against scripted stand-ins for
//! the real CLIs, so each contract is exercised end to end without a login, a
//! network or a paid request. None of those CLIs is installed where this is
//! built: what these tests prove is what the backend sends and how it reads the
//! documented answers, not that a real CLI accepts it (`AI_PROVIDERS.md`).
//!
//! Each test writes its own stand-in script into a private folder, with its
//! answers embedded, so tests share no environment and run in parallel.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use hide_ai::{
    AiBackend, AiError, AiRequest, Availability, CancelToken, GrokCliBackend, ModelCatalog,
    OutputLanguage, PiCliBackend, RequestId, TextCliConfig,
};
use serde_json::{Value, json};

/// The stand-in: records what it was started with, appends a line to the
/// file each `append_args` flag names (as a CLI writes its log), then answers
/// the first rule whose `when` words are all among its arguments.
const SCRIPT: &str = r#"#!/usr/bin/env python3
import json, os, sys, time
SCRIPT = json.loads(r'''__SCRIPT__''')
argv = sys.argv[1:]
stdin = sys.stdin.read()
files = {}
for flag in SCRIPT.get("file_args", []):
    if flag in argv:
        path = argv[argv.index(flag) + 1]
        with open(path, encoding="utf-8") as handle:
            files[flag] = {"text": handle.read(), "mode": oct(os.stat(path).st_mode & 0o777),
                           "dir_mode": oct(os.stat(os.path.dirname(path)).st_mode & 0o777)}
for flag in SCRIPT.get("append_args", []):
    if flag in argv:
        with open(argv[argv.index(flag) + 1], "a", encoding="utf-8") as handle:
            handle.write(" ".join(argv) + "\n")
record = {
    "argv": argv,
    "stdin": stdin,
    "cwd": os.getcwd(),
    "env": {key: os.environ.get(key) for key in SCRIPT.get("env_keys", [])},
    "files": files,
}
with open(SCRIPT["record"], "a", encoding="utf-8") as handle:
    handle.write(json.dumps(record) + "\n")
for rule in SCRIPT["rules"]:
    if all(word in argv for word in rule.get("when", [])):
        time.sleep(rule.get("sleep", 0))
        sys.stdout.write(rule.get("stdout", ""))
        sys.stderr.write(rule.get("stderr", ""))
        if "flood" in rule:
            sys.stdout.write("x" * rule["flood"])
        sys.exit(rule.get("exit", 0))
sys.exit(99)
"#;

struct Fake {
    /// Removed with the stand-in; a name no other test can share.
    _folder: tempfile::TempDir,
    dir: PathBuf,
    binary: PathBuf,
    record: PathBuf,
}

impl Fake {
    fn new(name: &str, script: Value) -> Self {
        let folder = tempfile::Builder::new()
            .prefix(&format!("hide-ai-fake-{name}-"))
            .tempdir()
            .unwrap();
        let dir = folder.path().to_path_buf();
        let record = dir.join("record.jsonl");
        let mut script = script;
        script["record"] = json!(record);
        let binary = dir.join(name);
        std::fs::write(&binary, SCRIPT.replace("__SCRIPT__", &script.to_string())).unwrap();
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            _folder: folder,
            dir,
            binary,
            record,
        }
    }

    fn config(&self, model: &str) -> TextCliConfig {
        TextCliConfig {
            binary: Some(self.binary.clone()),
            model: model.to_owned(),
            cwd: self.dir.clone(),
            search_path: None,
        }
    }

    /// Every start the stand-in recorded, oldest first.
    fn starts(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.record)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// The last recorded start.
    fn last(&self) -> Value {
        self.starts().pop().expect("the stand-in was started")
    }
}

fn schema() -> Value {
    json!({"type": "object", "required": ["summary"], "properties": {"summary": {"type": "string"}}})
}

const TRANSCRIPT: &str = "user: please add compact task labels (secret-transcript-marker)";

fn request() -> AiRequest {
    AiRequest {
        feature_id: "fixture".into(),
        request_id: RequestId("req-1".to_owned()),
        subject_id: "pane-1".to_owned(),
        system: "Summarise the transcript.".to_owned(),
        input: TRANSCRIPT.to_owned(),
        output_schema: schema(),
        deadline: Duration::from_secs(30),
        schema_version: "fixture.v1".into(),
        pick: None,
        output_language: Some(OutputLanguage::Korean),
        answer: hide_ai::AnswerMode::default(),
        provider_efforts: Vec::new(),
    }
}

/// The request's language reaches the CLI inside the system prompt it is
/// given, not only the prompt the backend was handed.
fn assert_names_the_language(system: &str) {
    assert!(
        system.contains("Write every text a person reads in Korean"),
        "{system}"
    );
}

fn argv(start: &Value) -> Vec<String> {
    start["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap().to_owned())
        .collect()
}

fn assert_transcript_not_in_argv(start: &Value) {
    assert!(
        !argv(start)
            .iter()
            .any(|arg| arg.contains("secret-transcript-marker")),
        "the transcript reached the argument vector: {:?}",
        argv(start)
    );
}

fn missing(config: TextCliConfig) -> TextCliConfig {
    TextCliConfig {
        binary: Some(PathBuf::from("/nonexistent/hide-ai-no-such-cli")),
        ..config
    }
}

// ---- Grok ----

fn grok_fake(rules: Value) -> Fake {
    Fake::new(
        "grok",
        json!({
            "rules": rules,
            "file_args": ["--prompt-file"],
            "env_keys": ["GROK_MEMORY", "GROK_DISABLE_AUTOUPDATER"],
        }),
    )
}

#[test]
fn grok_reads_the_transcript_from_a_private_file_with_every_tool_off() {
    let fake = grok_fake(json!([{
        "stdout": json!({"text": "{\"summary\":\"ok\"}", "stopReason": "end_turn"}).to_string()
    }]));
    let backend = GrokCliBackend::new(fake.config("grok-4.6"));
    let response = backend.execute(&request(), &CancelToken::new()).unwrap();
    assert_eq!(response.value, json!({"summary": "ok"}));

    let start = fake.last();
    let args = argv(&start);
    assert_transcript_not_in_argv(&start);
    assert_eq!(
        start["stdin"], "",
        "headless Grok reads no stdin, so none is written"
    );
    assert_eq!(start["files"]["--prompt-file"]["text"], TRANSCRIPT);
    assert_eq!(
        start["files"]["--prompt-file"]["mode"], "0o600",
        "the prompt file is owner-only"
    );
    assert_eq!(start["files"]["--prompt-file"]["dir_mode"], "0o700");
    let path = &args[args.iter().position(|a| a == "--prompt-file").unwrap() + 1];
    assert!(
        !Path::new(path).exists(),
        "the prompt file is gone once the request ended"
    );
    assert!(args.windows(2).any(|p| p == ["--tools", ""]), "{args:?}");
    assert_names_the_language(
        &args[args
            .iter()
            .position(|a| a == "--system-prompt-override")
            .unwrap()
            + 1],
    );
    for flag in ["--no-subagents", "--disable-web-search"] {
        assert!(args.iter().any(|arg| arg == flag), "{flag}");
    }
    assert!(
        args.windows(2).any(|p| p == ["--max-turns", "1"]),
        "{args:?}"
    );
    assert!(
        args.windows(2).any(|p| p == ["--model", "grok-4.6"]),
        "{args:?}"
    );
    assert_eq!(start["env"]["GROK_MEMORY"], "0");
    assert_eq!(start["env"]["GROK_DISABLE_AUTOUPDATER"], "1");
}

#[test]
fn grok_login_and_models_come_from_what_grok_models_prints() {
    let signed_in = grok_fake(json!([{
        "when": ["models"],
        "stdout": "* grok-4.6 (default)\n- grok-4.5\n"
    }]));
    let backend = GrokCliBackend::new(signed_in.config(""));
    assert_eq!(backend.availability(), Availability::Ready);
    assert_eq!(
        backend.models(),
        ModelCatalog::Offered(vec!["grok-4.6".to_owned(), "grok-4.5".to_owned()])
    );

    let signed_out =
        grok_fake(json!([{"when": ["models"], "stdout": "You are not authenticated.\n"}]));
    let backend = GrokCliBackend::new(signed_out.config(""));
    assert_eq!(backend.availability(), Availability::NeedsLogin);
    assert_eq!(
        backend.models(),
        ModelCatalog::Unknown {
            reason: "grok_not_authenticated".to_owned()
        }
    );

    let garbled = grok_fake(json!([{"when": ["models"], "stdout": "usage: grok models\n"}]));
    assert!(matches!(
        GrokCliBackend::new(garbled.config("")).availability(),
        Availability::Unavailable { .. }
    ));
    assert_eq!(
        GrokCliBackend::new(missing(signed_in.config(""))).availability(),
        Availability::NotInstalled
    );
}

#[test]
fn grok_failures_are_classified_from_its_error_object_and_exit() {
    for (rule, expected) in [
        (
            json!({"exit": 1, "stdout": json!({"type": "error", "message": "401 unauthorized"}).to_string()}),
            AiError::NotAuthenticated,
        ),
        (
            json!({"exit": 1, "stderr": "429 too many requests"}),
            AiError::UsageLimited { retry_after: None },
        ),
        (
            json!({"exit": 0, "stdout": "not json at all"}),
            AiError::CompletionUnknown("grok_output_not_json".into()),
        ),
    ] {
        let fake = grok_fake(json!([rule]));
        let error = GrokCliBackend::new(fake.config(""))
            .execute(&request(), &CancelToken::new())
            .unwrap_err();
        assert_eq!(error, expected);
    }
}

// ---- Pi ----

fn pi_fake(rules: Value) -> Fake {
    Fake::new("pi", json!({"rules": rules}))
}

#[test]
fn pi_runs_with_everything_off_and_the_transcript_on_stdin() {
    let fake = pi_fake(json!([{"stdout": "```json\n{\"summary\":\"ok\"}\n```\n"}]));
    let backend = PiCliBackend::new(fake.config("anthropic/claude-x"));
    let response = backend.execute(&request(), &CancelToken::new()).unwrap();
    assert_eq!(response.value, json!({"summary": "ok"}));

    let start = fake.last();
    let args = argv(&start);
    assert_transcript_not_in_argv(&start);
    assert_eq!(start["stdin"], TRANSCRIPT);
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
        assert!(
            args.iter().any(|arg| arg == flag),
            "{flag} missing from {args:?}"
        );
    }
    assert!(
        args.windows(2).any(|p| p == ["--thinking", "off"]),
        "{args:?}"
    );
    assert!(
        args.windows(2)
            .any(|p| p == ["--model", "anthropic/claude-x"]),
        "{args:?}"
    );
    let system = &args[args.iter().position(|a| a == "--system-prompt").unwrap() + 1];
    assert!(system.starts_with("Summarise the transcript."));
    assert_names_the_language(system);
    assert!(system.contains("\"required\":[\"summary\"]"));
}

#[test]
fn pi_login_for_a_named_provider_is_the_auth_check_exit_code() {
    for (exit, expected) in [
        (0, Availability::Ready),
        (1, Availability::NeedsLogin),
        (2, Availability::NeedsLogin),
    ] {
        let fake =
            pi_fake(json!([{"when": ["auth", "check", "--provider", "anthropic"], "exit": exit}]));
        let backend = PiCliBackend::new(fake.config("anthropic/claude-x"));
        assert_eq!(backend.availability(), expected, "exit {exit}");
        let start = fake.last();
        assert!(
            argv(&start)
                .windows(2)
                .any(|p| p == ["--provider", "anthropic"])
        );
    }
    let broken = pi_fake(json!([{"when": ["auth"], "exit": 9}]));
    assert!(matches!(
        PiCliBackend::new(broken.config("anthropic/x")).availability(),
        Availability::Unavailable { .. }
    ));
}

#[test]
fn pi_with_its_own_default_model_is_ready_when_it_lists_models_and_offers_that_list() {
    let fake = pi_fake(
        json!([{"when": ["--list-models"], "stdout": "anthropic/claude-x  200k\nopenai/gpt-y  128k\n"}]),
    );
    let backend = PiCliBackend::new(fake.config(""));
    assert_eq!(backend.availability(), Availability::Ready);
    assert_eq!(
        backend.models(),
        ModelCatalog::Offered(vec![
            "anthropic/claude-x".to_owned(),
            "openai/gpt-y".to_owned()
        ])
    );

    let none = pi_fake(json!([{"when": ["--list-models"], "stdout": ""}]));
    let backend = PiCliBackend::new(none.config(""));
    assert_eq!(backend.availability(), Availability::NeedsLogin);
    assert!(matches!(backend.models(), ModelCatalog::Unknown { .. }));
    assert_eq!(
        PiCliBackend::new(missing(none.config(""))).availability(),
        Availability::NotInstalled
    );
}

#[test]
fn pi_failures_and_runaway_output_are_reported_not_buffered() {
    for (rule, expected) in [
        (
            json!({"exit": 1, "stderr": "No API key found"}),
            AiError::NotAuthenticated,
        ),
        (
            json!({"exit": 3, "stderr": "odd"}),
            AiError::CompletionUnknown("pi_failed:exit=3".into()),
        ),
        (
            json!({"exit": 0, "stdout": "I will not"}),
            AiError::InvalidOutput("pi_answer_not_json".into()),
        ),
        (
            json!({"exit": 0, "flood": 9 * 1024 * 1024}),
            AiError::InvalidOutput("pi_output_too_large".into()),
        ),
    ] {
        let fake = pi_fake(json!([rule]));
        let error = PiCliBackend::new(fake.config(""))
            .execute(&request(), &CancelToken::new())
            .unwrap_err();
        assert_eq!(error, expected);
    }
}

#[test]
fn a_child_that_outlives_the_deadline_is_ended_and_reported_as_a_timeout() {
    let fake = pi_fake(json!([{"stdout": "{}", "sleep": 30}]));
    let mut slow = request();
    slow.deadline = Duration::from_millis(300);
    let error = PiCliBackend::new(fake.config(""))
        .execute(&slow, &CancelToken::new())
        .unwrap_err();
    assert_eq!(error, AiError::Timeout);
}

/// Blocks until the stand-in has recorded a start. The limit only ends a wait
/// on a child that never starts; it is not measured.
fn wait_for_start(fake: &Fake) {
    let limit = std::time::Instant::now() + Duration::from_secs(30);
    while fake.starts().is_empty() {
        assert!(
            std::time::Instant::now() < limit,
            "the stand-in never started"
        );
        std::thread::park_timeout(Duration::from_millis(10));
    }
}

#[test]
fn a_raised_cancel_token_stops_the_child_and_reports_cancelled() {
    let fake = pi_fake(json!([{"stdout": "{}", "sleep": 30}]));
    let backend = PiCliBackend::new(fake.config(""));
    let cancel = CancelToken::new();
    let outcome = std::thread::scope(|scope| {
        let running = scope.spawn(|| backend.execute(&request(), &cancel));
        wait_for_start(&fake);
        cancel.cancel();
        running.join().unwrap()
    });
    assert_eq!(outcome.unwrap_err(), AiError::Cancelled);
}

// ---- Antigravity CLI (agy) ----

/// agy is run with a `PATH` that finds nothing, so its stand-in names its
/// interpreter by the absolute path of the program itself (a version
/// manager's `python3` is a script that needs `PATH`).
fn agy_fake(rules: Value) -> Fake {
    let found = std::process::Command::new("python3")
        .args(["-c", "import sys; print(sys.executable)"])
        .output()
        .expect("python3 runs the stand-in");
    let python = String::from_utf8(found.stdout).unwrap().trim().to_owned();
    let fake = Fake::new(
        "agy",
        json!({"rules": rules, "env_keys": ["PATH", "AGY_CLI_DISABLE_AUTO_UPDATE", "HOME"],
            "append_args": ["--log-file"]}),
    );
    let body = std::fs::read_to_string(&fake.binary).unwrap().replacen(
        "#!/usr/bin/env python3",
        &format!("#!{python}"),
        1,
    );
    std::fs::write(&fake.binary, body).unwrap();
    fake
}

/// A home where agy has signed in and the kit has made Hide AI's folder,
/// trusted as agy spells it.
fn agy_home(trusted: bool) -> (tempfile::TempDir, PathBuf) {
    let home = tempfile::tempdir().unwrap();
    let folder = hide_ai::read_only_workspace(home.path());
    std::fs::create_dir_all(folder.join(".agents")).unwrap();
    std::fs::write(
        hide_ai::read_only_hooks_file(&folder),
        hide_ai::READ_ONLY_HOOKS,
    )
    .unwrap();
    let canonical = hide_platform::fs::identity::canonical(&folder).unwrap();
    let settings = home.path().join(".gemini/antigravity-cli/settings.json");
    std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
    let list = if trusted {
        json!([canonical])
    } else {
        json!([])
    };
    std::fs::write(&settings, json!({"trustedWorkspaces": list}).to_string()).unwrap();
    (home, canonical)
}

fn agy_stream(response: &str, tool_state: Option<&str>) -> String {
    let mut lines = vec![json!({"event": "init", "conversation_id": "c1"}).to_string()];
    if let Some(state) = tool_state {
        lines.push(
            json!({"event": "step_update", "step_update": {"step_type": "tool", "state": state}})
                .to_string(),
        );
    }
    lines.push(
        json!({"event": "result", "result": {"status": "SUCCESS", "response": response,
            "usage": {"input_tokens": 12, "output_tokens": 4}}})
        .to_string(),
    );
    lines.join("\n") + "\n"
}

#[test]
fn agy_answers_in_its_trusted_folder_with_the_prompt_on_stdin_and_no_program_on_path() {
    let fake = agy_fake(
        json!([{ "when": ["stream-json"], "stdout": agy_stream("{\"summary\":\"ok\"}", Some("ERROR")) }]),
    );
    let (home, folder) = agy_home(true);
    let backend =
        hide_ai::AntigravityCliBackend::with_home(fake.config(""), Some(home.path().to_path_buf()));
    let response = backend.execute(&request(), &CancelToken::new()).unwrap();
    assert_eq!(response.value, json!({"summary": "ok"}));
    assert_eq!(response.usage.input_tokens, Some(12));

    let start = fake.last();
    assert_eq!(Path::new(start["cwd"].as_str().unwrap()), folder);
    assert_transcript_not_in_argv(&start);
    let args = argv(&start);
    assert!(
        args.windows(2)
            .any(|p| p == ["--input-format", "stream-json"]),
        "{args:?}"
    );
    assert!(!args.iter().any(|arg| arg == "--json-schema"), "{args:?}");
    let message: Value = serde_json::from_str(start["stdin"].as_str().unwrap().trim()).unwrap();
    let content = message["message"]["content"].as_str().unwrap();
    assert!(content.contains("secret-transcript-marker"));
    assert_names_the_language(content);
    // No browser opener and no hook shell can be found, and agy never updates itself here.
    let path = start["env"]["PATH"].as_str().unwrap();
    assert!(!Path::new(path).exists(), "{path}");
    assert_eq!(start["env"]["AGY_CLI_DISABLE_AUTO_UPDATE"], "1");
}

#[test]
fn agy_is_not_started_until_its_folder_is_trusted_and_a_tool_that_ran_refuses_the_answer() {
    let fake = agy_fake(
        json!([{ "when": ["stream-json"], "stdout": agy_stream("{\"summary\":\"ok\"}", Some("DONE")) }]),
    );
    let (untrusted, _) = agy_home(false);
    let backend = hide_ai::AntigravityCliBackend::with_home(
        fake.config(""),
        Some(untrusted.path().to_path_buf()),
    );
    assert_eq!(
        backend.availability(),
        Availability::Unsupported {
            reason: "antigravity_read_only_untrusted".to_owned()
        }
    );
    assert_eq!(
        backend
            .execute(&request(), &CancelToken::new())
            .unwrap_err(),
        AiError::Unsupported("antigravity_read_only_untrusted".to_owned())
    );
    assert!(
        fake.starts().is_empty(),
        "nothing ran before the folder was trusted"
    );
    let (home, _) = agy_home(true);
    let backend =
        hide_ai::AntigravityCliBackend::with_home(fake.config(""), Some(home.path().to_path_buf()));
    assert_eq!(
        backend
            .execute(&request(), &CancelToken::new())
            .unwrap_err(),
        AiError::InvalidOutput("antigravity_tool_ran".to_owned())
    );
}

#[test]
fn each_kind_of_agy_child_keeps_only_its_last_log_in_hides_folder() {
    // agy keeps one log file per process in its own folder and never removes
    // any, and appends to a file it is given, so every child is handed its
    // kind's file in Hide's folder and that file holds the last run alone.
    let usage = json!({"status": "SUCCESS", "response": "", "command": {"name": "usage", "data": {"groups": []}}}).to_string();
    let fake = agy_fake(json!([
        { "when": ["stream-json"], "stdout": agy_stream("{\"summary\":\"ok\"}", None) },
        { "when": ["models"], "stdout": "Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\n" },
        { "when": ["/usage"], "stdout": usage },
    ]));
    let (home, _) = agy_home(true);
    let backend =
        hide_ai::AntigravityCliBackend::with_home(fake.config(""), Some(home.path().to_path_buf()));
    for _ in 0..2 {
        backend.execute(&request(), &CancelToken::new()).unwrap();
        assert_eq!(backend.availability(), Availability::Ready);
        backend.usage_text(&CancelToken::new()).unwrap();
    }

    let folder = home.path().join(".hide/ai/antigravity-logs");
    assert!(hide_platform::fs::private::is_private(&folder).unwrap());
    let starts = fake.starts();
    assert_eq!(starts.len(), 6);
    for (kind, word) in [
        ("request.log", "stream-json"),
        ("models.log", "models"),
        ("usage.log", "/usage"),
    ] {
        let log = folder.join(kind);
        for start in starts
            .iter()
            .filter(|start| argv(start).iter().any(|a| a == word))
        {
            let args = argv(start);
            assert!(
                args.windows(2)
                    .any(|p| p[0] == "--log-file" && Path::new(&p[1]) == log),
                "{args:?}"
            );
        }
        let lines = std::fs::read_to_string(&log).unwrap().lines().count();
        assert_eq!(lines, 1, "{kind} holds the last run's log only");
    }
}

#[test]
fn no_agy_child_starts_while_its_log_file_cannot_be_emptied() {
    // Something that is not a folder where Hide keeps agy's logs: running
    // agy anyway would put its log back in agy's own folder, unbounded.
    let fake = agy_fake(json!([{ "stdout": "{}" }]));
    let (home, _) = agy_home(true);
    std::fs::create_dir_all(home.path().join(".hide/ai")).unwrap();
    std::fs::write(
        home.path().join(".hide/ai/antigravity-logs"),
        "not a folder",
    )
    .unwrap();
    let backend =
        hide_ai::AntigravityCliBackend::with_home(fake.config(""), Some(home.path().to_path_buf()));

    let request = backend.execute(&request(), &CancelToken::new());
    assert!(
        matches!(&request, Err(AiError::ProviderUnavailable(code)) if code.starts_with("antigravity_log_unavailable:")),
        "{request:?}"
    );
    let models = backend.availability();
    assert!(
        matches!(&models, Availability::Unavailable { reason } if reason.starts_with("antigravity_models_log_unavailable:")),
        "{models:?}"
    );
    let usage = backend.usage_text(&CancelToken::new());
    assert!(
        matches!(&usage, Err(hide_ai::UsageError::Failed(code)) if code.starts_with("antigravity_usage_log_unavailable:")),
        "{usage:?}"
    );
    assert!(fake.starts().is_empty(), "no agy child was started");
}

#[test]
fn agy_is_ready_from_its_model_list_and_its_usage_is_the_whole_frame() {
    let usage = json!({"status": "SUCCESS", "response": "", "command": {"name": "usage", "data": {"groups": []}}}).to_string();
    let fake = agy_fake(json!([
        { "when": ["models"], "stdout": "Fetching available models...\ngemini-3.8-flash-high\tGemini 3.8 Flash (High)\n" },
        { "when": ["/usage"], "stdout": usage },
    ]));
    let (home, _) = agy_home(true);
    let backend =
        hide_ai::AntigravityCliBackend::with_home(fake.config(""), Some(home.path().to_path_buf()));
    assert_eq!(backend.availability(), Availability::Ready);
    assert_eq!(
        backend.models(),
        ModelCatalog::Offered(vec!["gemini-3.8-flash-high".to_owned()])
    );
    let text = backend.usage_text(&CancelToken::new()).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&text).unwrap()["command"]["name"],
        "usage"
    );

    let logged_out = agy_fake(json!([{ "stderr": "Error: authentication failed\n", "exit": 1 }]));
    let backend = hide_ai::AntigravityCliBackend::with_home(
        logged_out.config(""),
        Some(home.path().to_path_buf()),
    );
    assert_eq!(backend.availability(), Availability::NeedsLogin);
    assert_eq!(
        backend.usage_text(&CancelToken::new()).unwrap_err(),
        hide_ai::UsageError::NotAuthenticated
    );
    // A home where agy never signed in starts nothing: a signed-out agy would ask to sign in.
    let never = tempfile::tempdir().unwrap();
    let backend = hide_ai::AntigravityCliBackend::with_home(
        logged_out.config(""),
        Some(never.path().to_path_buf()),
    );
    let before = logged_out.starts().len();
    assert_eq!(backend.availability(), Availability::NeedsLogin);
    assert_eq!(logged_out.starts().len(), before);
}
