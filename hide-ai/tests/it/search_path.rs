//! A CLI found on the account's search is run with that search: a script CLI
//! whose interpreter only the login shell's folders reach is reported ready
//! and then answers, instead of being found and dying looking for its
//! interpreter (`docs/AI_PROVIDERS.md`, Where a CLI is found and run).
//!
//! Each stand-in is a script started as `#!/usr/bin/env hide-fixture-interpreter`,
//! the way pnpm's `codex` starts `node`. That interpreter is in a folder that
//! only the search path a test hands the backend reaches, never this process's
//! own `PATH`, and the login shell's folders are an explicit value built with
//! `cli_path_with`, so no test starts the developer's own shell.
#![cfg(unix)]

use std::ffi::OsString;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hide_ai::{
    AiBackend, AiRequest, Availability, CancelToken, ClaudeCliBackend, ClaudeConfig,
    CodexAppServerBackend, CodexConfig, NoopLogSink, PiCliBackend, RequestId, TextCliConfig,
};
use hide_platform::programs::cli_path_with;
use serde_json::json;

/// A private folder holding the stand-in login shell's two folders: the one
/// with the interpreter and the one with the CLIs.
struct Stage {
    /// Removed with the stage; a name no other test can share.
    _folder: tempfile::TempDir,
    root: PathBuf,
    interpreters: PathBuf,
    programs: PathBuf,
}

fn executable(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

impl Stage {
    fn new(name: &str) -> Self {
        let folder = tempfile::Builder::new()
            .prefix(&format!("hide-ai-search-{name}-"))
            .tempdir()
            .unwrap();
        let root = folder.path().to_path_buf();
        let interpreters = root.join("interpreters");
        let programs = root.join("programs");
        std::fs::create_dir_all(&interpreters).unwrap();
        std::fs::create_dir_all(&programs).unwrap();
        // The interpreter runs the script it is given as a shell script.
        executable(
            &interpreters.join("hide-fixture-interpreter"),
            "#!/bin/sh\nexec /bin/sh \"$@\"\n",
        );
        Self {
            _folder: folder,
            root,
            interpreters,
            programs,
        }
    }

    /// A CLI the interpreter runs: `body` is the script.
    fn cli(&self, name: &str, body: &str) -> PathBuf {
        let path = self.programs.join(name);
        executable(
            &path,
            &format!("#!/usr/bin/env hide-fixture-interpreter\n{body}\n"),
        );
        path
    }

    /// The search the account's login shell would give: its folders (the
    /// ones this stage made), then the inherited `PATH` and the usual
    /// install folders under a home that has none.
    fn search(&self) -> OsString {
        let shell_path =
            std::env::join_paths([self.interpreters.clone(), self.programs.clone()]).unwrap();
        cli_path_with(&self.root.join("home"), Some(&shell_path)).unwrap()
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn request() -> AiRequest {
    AiRequest {
        feature_id: "fixture".into(),
        request_id: RequestId("req-1".to_owned()),
        subject_id: "pane-1".to_owned(),
        system: "Answer as JSON.".to_owned(),
        input: "hello".to_owned(),
        output_schema: json!({
            "type": "object",
            "required": ["summary"],
            "properties": {"summary": {"type": "string"}}
        }),
        deadline: Duration::from_secs(30),
        schema_version: "fixture.v1".into(),
        pick: None,
        output_language: None,
        answer: hide_ai::AnswerMode::default(),
        provider_efforts: Vec::new(),
    }
}

const PI_BODY: &str = r#"case "$*" in
  *--list-models*) echo "anthropic/claude-x  200k" ;;
  *) cat > /dev/null; echo '{"summary":"ok"}' ;;
esac"#;

fn pi(stage: &Stage, search_path: Option<OsString>, binary: Option<PathBuf>) -> PiCliBackend {
    PiCliBackend::new(TextCliConfig {
        binary,
        model: String::new(),
        cwd: stage.root.clone(),
        search_path,
    })
}

#[test]
fn a_text_cli_whose_interpreter_only_the_search_reaches_is_ready_and_answers() {
    let stage = Stage::new("pi");
    let program = stage.cli("pi", PI_BODY);

    // Control: the same script named by file and run with this process's own
    // `PATH` cannot start its interpreter, so the stage proves something.
    let daemons_path = pi(&stage, None, Some(program));
    assert!(
        matches!(
            daemons_path.availability(),
            Availability::Unavailable { .. }
        ),
        "the interpreter must not be on this process's PATH"
    );

    // Found by name on the search, so run with it: the model-list probe and
    // the request both start the interpreter.
    let backend = pi(&stage, Some(stage.search()), None);
    assert_eq!(backend.availability(), Availability::Ready);
    let response = backend.execute(&request(), &CancelToken::new()).unwrap();
    assert_eq!(response.value, json!({"summary": "ok"}));
}

#[test]
fn claude_found_by_name_answers_its_probe_its_usage_read_and_its_request() {
    // The fake reads its behaviour from this process's `FAKE_*` variables.
    let _env = crate::FAKE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let stage = Stage::new("claude");
    stage.cli(
        "claude",
        &format!(
            "exec python3 \"{}\" \"$@\"",
            fixture("fake-claude.py").display()
        ),
    );
    let backend = ClaudeCliBackend::new(ClaudeConfig {
        search_path: Some(stage.search()),
        cwd: stage.root.clone(),
        ..ClaudeConfig::default()
    });

    assert_eq!(backend.availability(), Availability::Ready);
    let response = backend.execute(&request(), &CancelToken::new()).unwrap();
    assert_eq!(response.value["summary"], "fixture");
    // The usage read gets only the login variables, and still the search.
    let usage = backend.usage_text(&CancelToken::new());
    assert!(usage.is_ok(), "{usage:?}");
}

#[test]
fn the_codex_app_server_is_started_with_the_search_it_was_found_on() {
    // The fake reads its behaviour from this process's `FAKE_*` variables.
    let _env = crate::FAKE_ENV.lock().unwrap_or_else(|e| e.into_inner());
    let stage = Stage::new("codex");
    stage.cli(
        "codex",
        &format!(
            "exec python3 \"{}\" \"$@\"",
            fixture("fake-app-server.py").display()
        ),
    );
    let backend = CodexAppServerBackend::new(
        CodexConfig {
            search_path: Some(stage.search()),
            cwd: stage.root.clone(),
            ..CodexConfig::default()
        },
        Arc::new(NoopLogSink),
    );

    assert_eq!(backend.availability(), Availability::Ready);
    let response = backend.execute(&request(), &CancelToken::new()).unwrap();
    assert_eq!(response.value["summary"], "fixture");
}
