//! Drives `ClaudeCliBackend` against a scripted stand-in for the installed
//! `claude` CLI, so the print-mode client is exercised end to end without a
//! login, a network, or a paid request.

use std::path::PathBuf;
use std::time::Duration;

use hide_ai::{
    AiBackend, AiError, AiRequest, AiRouter, Availability, CancelToken, ClaudeCliBackend,
    ClaudeConfig, NoopLogSink, OutputLanguage, ProviderId, RequestId, RouterConfig,
};
use serde_json::{Value, json};

/// A Windows process cannot be started from a `.py` file, so there the
/// fixture is its batch launcher, which runs the same script.
fn fixture() -> PathBuf {
    let name = if cfg!(windows) {
        "fake-claude.cmd"
    } else {
        "fake-claude.py"
    };
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn backend() -> ClaudeCliBackend {
    ClaudeCliBackend::new(ClaudeConfig {
        binary: fixture(),
        model: "haiku".to_owned(),
        cwd: std::env::temp_dir(),
        search_path: None,
    })
}

fn schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["summary", "attention"],
        "properties": {
            "summary": {"type": "string"},
            "attention": {"type": "string", "enum": ["question", "none"]}
        }
    })
}

fn request(deadline: Duration) -> AiRequest {
    AiRequest {
        feature_id: "fixture".into(),
        request_id: RequestId("req-1".to_owned()),
        subject_id: "pane-1".to_owned(),
        system: "Classify the transcript. Answer only with the schema.".to_owned(),
        input: "user: add compact task labels".to_owned(),
        output_schema: schema(),
        deadline,
        schema_version: "fixture.v1".into(),
        pick: None,
        output_language: None,
        answer: hide_ai::AnswerMode::default(),
        provider_efforts: Vec::new(),
    }
}

fn with_mode<T>(mode: &str, body: impl FnOnce() -> T) -> T {
    let _guard = crate::FAKE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: FAKE_ENV serialises every writer of a FAKE_* variable in this
    // binary, and the fixture reads it once at spawn.
    unsafe { std::env::set_var("FAKE_MODE", mode) };
    let out = body();
    unsafe { std::env::remove_var("FAKE_MODE") };
    out
}

fn scratch(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("hide-ai-{name}-{}.txt", std::process::id()))
}

#[test]
fn a_completed_turn_returns_structured_output_with_usage() {
    with_mode("ok", || {
        let backend = backend();
        assert_eq!(backend.availability(), Availability::Ready);
        let response = backend
            .execute(&request(Duration::from_secs(30)), &CancelToken::new())
            .unwrap();
        assert_eq!(response.value["summary"], "fixture");
        assert_eq!(response.usage.input_tokens, Some(1188));
        assert_eq!(response.usage.output_tokens, Some(468));
    });
}

/// The ledger counts the whole call under the one model the frame names,
/// with the cache categories and the one-hour split of `usage`.
#[test]
fn a_completed_turn_reports_ledger_tokens_under_the_resolved_model() {
    with_mode("ok", || {
        let response = backend()
            .execute(&request(Duration::from_secs(30)), &CancelToken::new())
            .unwrap();
        let [tokens] = response.usage.models.as_slice() else {
            panic!("{:?}", response.usage.models);
        };
        assert_eq!(tokens.model, "claude-haiku-4-5-20251001");
        assert_eq!(
            (
                tokens.input,
                tokens.output,
                tokens.cache_read,
                tokens.cache_write
            ),
            (1188, 468, 4000, 600)
        );
        assert_eq!(tokens.cache_write_1h, 100);
        assert!(!tokens.lower_bound);
    });
}

/// The three flags that decide whether the answer is right at all have to
/// reach the child, not merely the vector the backend builds. Dropping one
/// produces a confident wrong answer, so the vector is the assertion.
#[test]
fn the_child_receives_the_system_prompt_and_the_emptied_tool_and_setting_sources() {
    with_mode("ok", || {
        let args_file = scratch("claude-args");
        let stdin_file = scratch("claude-stdin");
        unsafe {
            std::env::set_var("FAKE_ARGS_FILE", &args_file);
            std::env::set_var("FAKE_STDIN_FILE", &stdin_file);
        }
        let request = AiRequest {
            output_language: Some(OutputLanguage::Korean),
            answer: hide_ai::AnswerMode::default(),
            ..request(Duration::from_secs(30))
        };
        backend().execute(&request, &CancelToken::new()).unwrap();
        unsafe {
            std::env::remove_var("FAKE_ARGS_FILE");
            std::env::remove_var("FAKE_STDIN_FILE");
        }
        let args: Vec<String> =
            serde_json::from_slice(&std::fs::read(&args_file).unwrap()).unwrap();
        let prompt = std::fs::read_to_string(&stdin_file).unwrap();
        let _ = std::fs::remove_file(&args_file);
        let _ = std::fs::remove_file(&stdin_file);

        assert_eq!(
            args,
            ClaudeCliBackend::print_arguments(
                "haiku",
                None,
                &request.system_prompt(),
                Some(&request.output_schema)
            )
        );
        assert_eq!(args.first().map(String::as_str), Some("-p"));
        // The system prompt the child gets is the request's with its
        // language named.
        let system = &args[args.iter().position(|a| a == "--system-prompt").unwrap() + 1];
        assert!(system.starts_with(&request.system), "{system}");
        assert!(
            system.contains("Write every text a person reads in Korean"),
            "{system}"
        );
        assert!(
            args.windows(2).any(|pair| pair == ["--tools", ""]),
            "{args:?}"
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--setting-sources", ""]),
            "{args:?}"
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--output-format", "json"]),
            "{args:?}"
        );
        assert!(args.iter().any(|arg| arg == "--no-session-persistence"));
        assert!(args.iter().any(|arg| arg == "--strict-mcp-config"));
        assert!(args.iter().any(|arg| arg == "--disable-slash-commands"));
        assert!(!args.iter().any(|arg| arg == "--bare"));

        // The transcript travels on stdin only.
        assert_eq!(prompt, request.input);
        assert!(
            !args.iter().any(|arg| arg.contains("compact task labels")),
            "{args:?}"
        );
    });
}

/// A plain-JSON request asks the CLI for one model turn: no `--json-schema`
/// (which Claude Code answers with a second turn through a synthetic tool),
/// and the schema in the system prompt instead.
#[test]
fn a_plain_json_request_sends_no_schema_flag_and_names_the_schema_in_the_prompt() {
    with_mode("fenced_result", || {
        let args_file = scratch("claude-plain-args");
        unsafe { std::env::set_var("FAKE_ARGS_FILE", &args_file) };
        let request = AiRequest {
            answer: hide_ai::AnswerMode::PlainJson,
            ..request(Duration::from_secs(30))
        };
        let response = backend().execute(&request, &CancelToken::new()).unwrap();
        unsafe { std::env::remove_var("FAKE_ARGS_FILE") };
        let args: Vec<String> =
            serde_json::from_slice(&std::fs::read(&args_file).unwrap()).unwrap();
        let _ = std::fs::remove_file(&args_file);

        assert!(!args.iter().any(|arg| arg == "--json-schema"), "{args:?}");
        let system = &args[args.iter().position(|a| a == "--system-prompt").unwrap() + 1];
        assert!(system.starts_with(&request.system), "{system}");
        assert!(system.contains(&schema().to_string()), "{system}");
        // The answer is read out of the fenced `result` text.
        assert_eq!(response.value["summary"], "fixture");
    });
}

/// Without a schema the shape is the router's to check, but a `result` that
/// holds no JSON at all is an unusable answer, retried like any other.
#[test]
fn a_plain_json_result_without_json_is_invalid_output() {
    with_mode("prose_result", || {
        let request = AiRequest {
            answer: hide_ai::AnswerMode::PlainJson,
            ..request(Duration::from_secs(30))
        };
        let error = backend()
            .execute(&request, &CancelToken::new())
            .unwrap_err();
        assert_eq!(
            error,
            AiError::InvalidOutput("claude_result_not_json".to_owned())
        );
    });
}

/// A model that thinks although thinking was turned off shows it in the
/// usage, which the router logs next to the output tokens.
#[test]
fn thinking_tokens_in_the_usage_are_reported_and_absent_ones_are_not() {
    with_mode("thinking_leak", || {
        let response = backend()
            .execute(&request(Duration::from_secs(30)), &CancelToken::new())
            .unwrap();
        assert_eq!(response.usage.thinking_tokens, Some(384));
    });
    with_mode("ok", || {
        let response = backend()
            .execute(&request(Duration::from_secs(30)), &CancelToken::new())
            .unwrap();
        assert_eq!(response.usage.thinking_tokens, None);
    });
}

/// Print mode thinks by default, which made a short JSON answer take tens of
/// seconds; a model turn always runs with thinking off, even when the
/// operator's shell exports a budget of its own.
#[test]
fn a_model_turn_runs_with_thinking_off_whatever_the_shell_exports() {
    with_mode("ok", || {
        let thinking_file = scratch("claude-thinking");
        unsafe {
            std::env::set_var("FAKE_THINKING_FILE", &thinking_file);
            std::env::set_var("MAX_THINKING_TOKENS", "31999");
        }
        let result = backend().execute(&request(Duration::from_secs(30)), &CancelToken::new());
        unsafe {
            std::env::remove_var("FAKE_THINKING_FILE");
            std::env::remove_var("MAX_THINKING_TOKENS");
        }
        let received = std::fs::read_to_string(&thinking_file).unwrap();
        let _ = std::fs::remove_file(&thinking_file);

        result.unwrap();
        assert_eq!(received, "0");
    });
}

/// `result` carries the same JSON the schema bound, and it is still not the
/// answer: without `structured_output` nothing validated the shape.
#[test]
fn a_result_frame_without_structured_output_is_invalid_output() {
    for mode in ["no_structured_output", "null_structured_output"] {
        with_mode(mode, || {
            let error = backend()
                .execute(&request(Duration::from_secs(30)), &CancelToken::new())
                .unwrap_err();
            assert_eq!(
                error,
                AiError::InvalidOutput("claude_result_without_structured_output".to_owned()),
                "{mode}"
            );
        });
    }
}

/// The child ran, so the prompt was submitted; with no result frame nothing
/// says whether the turn completed.
#[test]
fn a_child_that_prints_no_result_frame_is_completion_unknown() {
    for mode in ["no_result_frame", "init_frame_only"] {
        with_mode(mode, || {
            match backend().execute(&request(Duration::from_secs(30)), &CancelToken::new()) {
                Err(AiError::CompletionUnknown(reason)) => {
                    assert!(reason.contains("exit=1"), "{mode}: {reason}");
                }
                other => panic!("{mode}: {other:?}"),
            }
        });
    }
}

/// A completion-unknown outcome is spent: no second attempt here, and no
/// attempt on the other provider.
#[test]
fn a_completion_unknown_outcome_is_never_retried_or_moved() {
    with_mode("no_result_frame", || {
        let args_file = scratch("claude-unknown-args");
        unsafe { std::env::set_var("FAKE_ARGS_FILE", &args_file) };
        let router = AiRouter::new(
            vec![std::sync::Arc::new(backend())],
            RouterConfig {
                priority: vec![ProviderId::CLAUDE],
                ..RouterConfig::default()
            },
            std::sync::Arc::new(NoopLogSink),
        );
        let error = router
            .execute(&request(Duration::from_secs(30)), &CancelToken::new())
            .unwrap_err();
        unsafe { std::env::remove_var("FAKE_ARGS_FILE") };
        let _ = std::fs::remove_file(&args_file);
        assert!(matches!(error, AiError::CompletionUnknown(_)), "{error:?}");
        // The provider was not parked by it: the next intent is new.
        let state = router.provider_state().unwrap();
        assert_eq!(state.active, Some(ProviderId::CLAUDE));
        assert_eq!(state.degraded, None);
    });
}

/// Each row was observed by answering the CLI's own API request with that
/// status; the fixture reproduces the frame that came back.
#[test]
fn each_measured_failure_shape_maps_to_its_class() {
    let cases: [(&str, AiError); 7] = [
        ("api_401", AiError::NotAuthenticated),
        ("api_403", AiError::NotAuthenticated),
        ("api_429", AiError::UsageLimited { retry_after: None }),
        ("api_500", AiError::Transient("claude_api_500".to_owned())),
        ("api_400", AiError::Rejected("claude_api_400".to_owned())),
        (
            "structured_output_retries",
            AiError::InvalidOutput("claude_structured_output_retries".to_owned()),
        ),
        (
            "context_limit",
            AiError::Rejected("claude_context_limit:prompt_too_long".to_owned()),
        ),
    ];
    for (mode, expected) in cases {
        with_mode(mode, || {
            let error = backend()
                .execute(&request(Duration::from_secs(30)), &CancelToken::new())
                .unwrap_err();
            assert_eq!(error, expected, "{mode}");
        });
    }
}

#[test]
fn an_expired_deadline_kills_the_child_and_reports_timeout() {
    with_mode("slow", || {
        let started = std::time::Instant::now();
        let error = backend()
            .execute(&request(Duration::from_millis(400)), &CancelToken::new())
            .unwrap_err();
        assert_eq!(error, AiError::Timeout);
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the child was not killed"
        );
    });
}

/// Waits until a slow fake has created `marker`, which it does once it is
/// running and waiting. The fake is another process, so the file is polled.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_for_start(marker: &std::path::Path) {
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    while !marker.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "the child never started: {}",
            marker.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The cancel is raised only once the child is running, so it is the
/// running child that is killed, not one still starting.
fn cancel_once_started(marker: PathBuf) -> (CancelToken, std::thread::JoinHandle<()>) {
    let cancel = CancelToken::new();
    let canceller = cancel.clone();
    let raised = std::thread::spawn(move || {
        wait_for_start(&marker);
        canceller.cancel();
    });
    (cancel, raised)
}

#[test]
fn a_cancelled_request_kills_the_child() {
    with_mode("slow", || {
        let marker = scratch("started");
        let _ = std::fs::remove_file(&marker);
        // SAFETY: `with_mode` holds the lock every writer of FAKE_* takes.
        unsafe { std::env::set_var("FAKE_STARTED_FILE", &marker) };
        let (cancel, raised) = cancel_once_started(marker.clone());
        let error = backend()
            .execute(&request(Duration::from_secs(30)), &cancel)
            .unwrap_err();
        unsafe { std::env::remove_var("FAKE_STARTED_FILE") };
        let _ = std::fs::remove_file(&marker);
        raised.join().unwrap();
        assert_eq!(error, AiError::Cancelled);
    });
}

#[test]
fn availability_reports_login_install_and_probe_state() {
    with_mode("ok", || {
        assert_eq!(backend().availability(), Availability::Ready)
    });
    with_mode("no_account", || {
        assert_eq!(backend().availability(), Availability::NeedsLogin);
    });
    with_mode("auth_broken", || match backend().availability() {
        Availability::Unavailable { reason } => {
            assert!(reason.contains("auth_status_unreadable"), "{reason}");
        }
        other => panic!("{other:?}"),
    });
    // Without the field nothing is known, and neither answer is invented.
    with_mode("auth_without_field", || {
        assert_eq!(
            backend().availability(),
            Availability::Unavailable {
                reason: "auth_status_without_logged_in".to_owned()
            }
        );
    });
    let missing = ClaudeCliBackend::new(ClaudeConfig {
        binary: PathBuf::from("/nonexistent/claude-binary-that-does-not-exist"),
        ..ClaudeConfig::default()
    });
    assert_eq!(missing.availability(), Availability::NotInstalled);
    assert_eq!(
        missing
            .execute(&request(Duration::from_secs(1)), &CancelToken::new())
            .unwrap_err(),
        AiError::ProviderUnavailable("claude_not_installed".to_owned())
    );
}

/// A `/usage` read is the one child that gets a whitelisted environment: the
/// child records what it received, and the record is what is asserted.
mod usage {
    use super::*;
    use hide_ai::{USAGE_ENVIRONMENT, UsageError};

    /// The fake CLI's working folder, which tells it what to answer and
    /// keeps what it received; it goes when the test drops it. The fake's
    /// interpreter has started once before, so no read's deadline holds it.
    fn usage_dir(mode: &str) -> tempfile::TempDir {
        interpreter_started();
        let dir = tempfile::Builder::new()
            .prefix(&format!("hide-ai-usage-{mode}-"))
            .tempdir()
            .unwrap();
        std::fs::write(dir.path().join("usage-mode"), mode).unwrap();
        dir
    }

    /// The fake CLI started once before a read's deadline (docs/TESTING.md,
    /// Writing a Rust test, rule 8): a Windows runner's first `python` of a
    /// job took 21-30 s where later ones take about a second, so the first
    /// reads in the job timed out before their child had started
    /// (with-hide/hide-private#230). The lock keeps another test's `FAKE_*`
    /// paths out of that start.
    fn interpreter_started() {
        let _guard = crate::FAKE_ENV.lock().unwrap_or_else(|e| e.into_inner());
        crate::stand_ins::ready_with_args(&fixture(), &["auth", "status", "--json"]);
    }

    fn usage_backend(cwd: &std::path::Path) -> ClaudeCliBackend {
        ClaudeCliBackend::new(ClaudeConfig {
            binary: fixture(),
            model: "haiku".to_owned(),
            cwd: cwd.to_path_buf(),
            search_path: None,
        })
    }

    #[test]
    fn the_result_text_comes_back_whatever_it_says() {
        let dir = usage_dir("text");
        let text = usage_backend(dir.path())
            .usage_text(&CancelToken::new())
            .unwrap();
        assert!(
            text.contains("Current week (all models): 1% used"),
            "{text}"
        );

        let dir = usage_dir("cost");
        let text = usage_backend(dir.path())
            .usage_text(&CancelToken::new())
            .unwrap();
        assert!(text.starts_with("Total cost:"), "{text}");
    }

    #[test]
    fn the_child_gets_only_the_whitelisted_environment_and_the_configured_cwd() {
        let dir = usage_dir("text");
        // A variable the CLI must not see, planted where the child would
        // otherwise inherit it.
        let _guard = crate::FAKE_ENV.lock().unwrap_or_else(|e| e.into_inner());
        unsafe {
            std::env::set_var("HERDR_ENV", "1");
            std::env::set_var("CLAUDECODE", "1");
            std::env::set_var("FAKE_MODE", "no_account");
        }
        let result = usage_backend(dir.path()).usage_text(&CancelToken::new());
        unsafe {
            std::env::remove_var("HERDR_ENV");
            std::env::remove_var("CLAUDECODE");
            std::env::remove_var("FAKE_MODE");
        }
        result.unwrap();

        let args: Vec<String> =
            serde_json::from_slice(&std::fs::read(dir.path().join("usage-args.json")).unwrap())
                .unwrap();
        assert_eq!(args, ClaudeCliBackend::usage_arguments());
        assert_eq!(args[..2], ["-p", "/usage"]);
        assert!(args.iter().any(|arg| arg == "--no-session-persistence"));
        // The operator's hooks would leave a `session-env` folder per read.
        assert!(args.windows(2).any(|p| p == ["--setting-sources", ""]));

        // The fixture's own interpreter shim adds variables of its own
        // (`PWD`, `PYENV_*`), so the assertion is on the three planted keys
        // that must not cross and the ones the account needs. Python reports
        // Windows names in upper case, and Windows names do not differ by case.
        let keys: Vec<String> =
            serde_json::from_slice(&std::fs::read(dir.path().join("usage-env.json")).unwrap())
                .unwrap();
        for planted in ["HERDR_ENV", "CLAUDECODE", "FAKE_MODE"] {
            assert!(!keys.iter().any(|key| key == planted), "{planted} {keys:?}");
        }
        let has = |name: &str| {
            keys.iter().any(|key| {
                if cfg!(windows) {
                    key.eq_ignore_ascii_case(name)
                } else {
                    key == name
                }
            })
        };
        // What the CLI cannot start or find its login without, spelled out
        // here rather than read back from the list under test. `PATHEXT` is
        // proven by hide-platform's contract test instead: the batch launcher
        // here supplies its own through cmd.exe.
        let needed: &[&str] = if cfg!(windows) {
            &[
                "SystemRoot",
                "USERPROFILE",
                "TEMP",
                "TMP",
                "APPDATA",
                "LOCALAPPDATA",
                "PATH",
            ]
        } else {
            &["HOME", "PATH"]
        };
        for kept in needed {
            assert!(has(kept), "{kept} {keys:?}");
        }
        for kept in USAGE_ENVIRONMENT {
            if std::env::var_os(kept).is_some() {
                assert!(has(kept), "{kept} {keys:?}");
            }
        }
    }

    #[test]
    fn each_failure_shape_is_its_own_class_and_none_carries_output() {
        let dir = usage_dir("exit");
        assert_eq!(
            usage_backend(dir.path()).usage_text(&CancelToken::new()),
            Err(UsageError::Failed("usage_exit_2".to_owned()))
        );
        let dir = usage_dir("is_error");
        assert_eq!(
            usage_backend(dir.path()).usage_text(&CancelToken::new()),
            Err(UsageError::Failed("usage_is_error".to_owned()))
        );
        let dir = usage_dir("not_json");
        assert_eq!(
            usage_backend(dir.path()).usage_text(&CancelToken::new()),
            Err(UsageError::NoResultFrame)
        );
        let missing = ClaudeCliBackend::new(ClaudeConfig {
            binary: PathBuf::from("/nonexistent/claude-binary-that-does-not-exist"),
            ..ClaudeConfig::default()
        });
        assert_eq!(
            missing.usage_text(&CancelToken::new()),
            Err(UsageError::NotInstalled)
        );
    }

    #[test]
    fn a_cancelled_read_kills_the_child() {
        let dir = usage_dir("slow");
        let (cancel, raised) = cancel_once_started(dir.path().join("usage-started"));
        let started = std::time::Instant::now();
        assert_eq!(
            usage_backend(dir.path()).usage_text(&cancel),
            Err(UsageError::Cancelled)
        );
        raised.join().unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the child was not killed"
        );
    }
}

#[test]
fn the_model_list_is_asked_of_the_cli_without_a_model_turn_and_leaks_no_account() {
    with_mode("ok", || {
        let args_file = scratch("claude-models-args");
        let stdin_file = scratch("claude-models-stdin");
        unsafe {
            std::env::set_var("FAKE_ARGS_FILE", &args_file);
            std::env::set_var("FAKE_STDIN_FILE", &stdin_file);
        }
        let catalog = backend().models();
        unsafe {
            std::env::remove_var("FAKE_ARGS_FILE");
            std::env::remove_var("FAKE_STDIN_FILE");
        }
        let args: Vec<String> =
            serde_json::from_slice(&std::fs::read(&args_file).unwrap()).unwrap();
        let request = std::fs::read_to_string(&stdin_file).unwrap();
        let _ = std::fs::remove_file(&args_file);
        let _ = std::fs::remove_file(&stdin_file);

        assert_eq!(args, ClaudeCliBackend::models_arguments());
        assert!(args.windows(2).any(|p| p == ["--tools", ""]), "{args:?}");
        assert!(request.contains(r#""subtype":"initialize""#), "{request}");
        assert_eq!(
            catalog,
            hide_ai::ModelCatalog::Offered(vec!["opus".to_owned(), "sonnet".to_owned()]),
            "the account's models, without the CLI's own 'default'"
        );
        assert!(
            !format!("{catalog:?}").contains("example"),
            "no account field reaches the answer"
        );
    });
}

#[test]
fn a_model_list_the_cli_cannot_give_is_reported_not_replaced_by_a_guess() {
    with_mode("init_broken", || match backend().models() {
        hide_ai::ModelCatalog::Unknown { reason } => {
            assert!(reason.starts_with("claude_models_unreadable"), "{reason}");
        }
        other => panic!("{other:?}"),
    });
}

/// Drives the real, logged-in `claude`: its model list through the
/// `initialize` request, then one tiny request through the router. It writes
/// the model values and the outcome class to `HIDE_AI_LIVE_EVIDENCE` and
/// nothing else, so no account or organisation field can reach the record.
///
/// ```sh
/// HIDE_AI_LIVE_EVIDENCE=<folder> cargo test -p hide-ai --test it -- --ignored claude_cli::real_claude
/// ```
#[test]
#[ignore = "needs a logged-in claude on PATH"]
fn real_claude_lists_its_models_and_answers_one_request() {
    let evidence = PathBuf::from(
        std::env::var("HIDE_AI_LIVE_EVIDENCE").expect("HIDE_AI_LIVE_EVIDENCE names the folder"),
    );
    std::fs::create_dir_all(&evidence).unwrap();
    let backend = std::sync::Arc::new(ClaudeCliBackend::new(ClaudeConfig {
        model: "haiku".to_owned(),
        ..ClaudeConfig::default()
    }));

    let catalog = backend.models();
    let models = match &catalog {
        hide_ai::ModelCatalog::Offered(models) => models.clone(),
        other => panic!("the live model list was not offered: {other:?}"),
    };
    assert!(!models.is_empty() && !models.iter().any(|model| model == "default"));

    let router = AiRouter::new(
        vec![backend.clone()],
        RouterConfig {
            priority: vec![ProviderId::CLAUDE],
            ..RouterConfig::default()
        },
        std::sync::Arc::new(NoopLogSink),
    );
    let mut ask = request(Duration::from_secs(60));
    ask.input = "user: please rename the build script".to_owned();
    let outcome = router.execute(&ask, &CancelToken::new());
    let class = match &outcome {
        Ok(_) => "ok",
        Err(error) => error.class(),
    };
    let statuses: Vec<Value> = router
        .statuses()
        .iter()
        .map(|status| {
            json!({
                "provider": status.provider.as_str(),
                "selectable": status.selectable,
            })
        })
        .collect();
    std::fs::write(
        evidence.join("claude-live.json"),
        serde_json::to_string_pretty(&json!({
            "models": models,
            "request_outcome": class,
            "answered_by": router.last_answered().map(|provider| provider.as_str()),
            "statuses": statuses,
        }))
        .unwrap(),
    )
    .unwrap();
    assert_eq!(class, "ok", "{outcome:?}");
}
