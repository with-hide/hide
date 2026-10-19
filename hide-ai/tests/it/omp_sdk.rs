//! Runs the installed public SDK against a synthetic login and localhost model.
//! The catalog alone is substituted to redirect the vendor transport locally.
//! CI installs each supported exact version; no operator state is used.
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use hide_ai::{
    AiBackend, AiError, AiRequest, AiRouter, Availability, CancelToken, ModelCatalog, NoopLogSink,
    OmpConfig, OmpSdkBackend, ProviderId, RequestId, RouterConfig,
};
use hide_platform::process::OwnedChild;
use serde_json::{Value, json};

fn request(model: &str) -> AiRequest {
    AiRequest {
        feature_id: "omp-fixture".into(),
        request_id: RequestId(model.into()),
        subject_id: model.into(),
        system: "EXACT_SYSTEM".into(),
        input: "EXACT_INPUT".into(),
        output_schema: json!({"type":"object","required":["value"],"properties":{"value":{"type":"string"}},"additionalProperties":false}),
        deadline: Duration::from_secs(15),
        schema_version: "fixture.v1".into(),
        pick: None,
        output_language: None,
        answer: hide_ai::AnswerMode::PlainJson,
        provider_efforts: vec![],
    }
}

fn lines(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// The entire test holds FAKE_ENV, and nextest isolates each test process.
/// Drop restores every variable even on a failed assertion.
struct Home(Vec<(String, Option<std::ffi::OsString>)>);
impl Home {
    fn new(path: &Path) -> Self {
        let mut saved = vec![];
        for name in [
            "HOME",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
            "TMPDIR",
            "TMP",
            "TEMP",
        ] {
            saved.push((name.into(), std::env::var_os(name)));
            // SAFETY: this test holds FAKE_ENV before starting its threads.
            unsafe {
                std::env::set_var(name, path);
            }
        }
        Self(saved)
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        for (key, value) in &self.0 {
            // SAFETY: all test threads and children have ended before Home.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

// A separately killed Rust owner exercises the production backend, without
// its destructors. The parent test alone supplies these synthetic paths.
#[test]
#[ignore = "helper subprocess for the SDK ownership contract"]
fn abrupt_owner_helper() {
    let Some(root) = std::env::var_os("HIDE_TEST_OMP_OWNER_ROOT") else {
        return;
    };
    let mut config = OmpConfig::new("openai/hang");
    config.cli.binary = Some(PathBuf::from(root).join("bin/omp"));
    config.runtime = Some(PathBuf::from(
        std::env::var_os("HIDE_TEST_BUN_BIN").unwrap(),
    ));
    let _ = OmpSdkBackend::new(config).execute(&request("hang"), &CancelToken::new());
    panic!("the test owner must be terminated while inference is pending");
}

#[test]
#[ignore = "requires HIDE_TEST_BUN_BIN and HIDE_TEST_OMP_PACKAGE; synthetic SDK contract runs in omp-contract CI"]
fn real_sdk_preserves_the_background_provider_boundary() {
    let _guard = super::FAKE_ENV.lock().unwrap();
    let bun = PathBuf::from(std::env::var_os("HIDE_TEST_BUN_BIN").expect("Bun fixture runtime"));
    let installed =
        PathBuf::from(std::env::var_os("HIDE_TEST_OMP_PACKAGE").expect("official npm package"));
    assert!(bun.is_file() && installed.join("package.json").is_file());
    let folder = tempfile::tempdir().unwrap();
    let home = folder.path().join("home");
    std::fs::create_dir(&home).unwrap();
    let _home = Home::new(&home);
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/omp-sdk.mjs");
    let mut command = Command::new(&bun);
    hide_platform::process::restrict_to_login_environment(&mut command);
    command
        .args(["--no-env-file"])
        .arg(fixture)
        .arg(&installed)
        .arg(folder.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    let mut child = OwnedChild::spawn(&mut command).unwrap();
    let owner_stdin = child.take_stdin().unwrap();
    let stdout = child.take_stdout().unwrap();
    let (sender, events) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let ready = events
        .recv_timeout(Duration::from_secs(30))
        .expect("SDK fixture starts");
    let ready: Value = serde_json::from_str(&ready).unwrap();
    let root = PathBuf::from(ready["root"].as_str().unwrap());
    let record = PathBuf::from(ready["record"].as_str().unwrap());
    let db = PathBuf::from(ready["db"].as_str().unwrap());

    // Ambient CLI/session modules would execute these canaries. The library
    // boundary never imports them, even from the real auth home or cwd.
    for relative in [".omp/AGENTS.md", ".agents/AGENTS.md", "AGENTS.md"] {
        let path = home.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "AMBIENT_CONTEXT_CANARY").unwrap();
    }
    let marker = home.join("ambient-executed");
    let agent_dir = db.parent().unwrap();
    for path in [
        home.join(".omp/extensions/evil.mjs"),
        home.join(".omp/tools/evil.mjs"),
        home.join(".agents/extensions/evil.mjs"),
        agent_dir.join("extensions/evil.mjs"),
        agent_dir.join("tools/evil.mjs"),
    ] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!(
                "import {{writeFileSync}} from 'node:fs'; writeFileSync({},'bad');",
                json!(marker)
            ),
        )
        .unwrap();
    }
    std::fs::write(
        home.join(".omp/mcp.json"),
        json!({"mcpServers":{"evil":{"command":"must-never-start"}}}).to_string(),
    )
    .unwrap();
    for path in [home.join(".env"), agent_dir.join(".env")] {
        std::fs::write(path, "PI_REQ_DEBUG=1\nOMP_REQ_DEBUG=1\nPI_AI_CLAUDE_CODE_VERSION=AMBIENT_VERSION_CANARY\nOMP_AI_CLAUDE_CODE_VERSION=AMBIENT_VERSION_CANARY\n").unwrap();
    }
    let preload = home.join("preload.mjs");
    std::fs::write(
        &preload,
        format!(
            "import {{writeFileSync}} from 'node:fs'; writeFileSync({},'bad');",
            json!(marker)
        ),
    )
    .unwrap();
    for name in [".bunfig.toml", "bunfig.toml"] {
        std::fs::write(home.join(name), format!("preload = [{}]\n", json!(preload))).unwrap();
    }
    std::fs::write(
        agent_dir.join("mcp.json"),
        json!({"mcpServers":{"evil":{"command":"must-never-start"}}}).to_string(),
    )
    .unwrap();
    std::fs::write(agent_dir.join("AGENTS.md"), "AMBIENT_CONTEXT_CANARY").unwrap();
    std::fs::write(
        agent_dir.join("config.yml"),
        "fallbackChains:\n  openai: [must-never-be-requested]\n",
    )
    .unwrap();
    let config = |model: &str| {
        let provider = if model == "api-anthropic" || model.starts_with("stream-http") {
            "anthropic"
        } else {
            "openai"
        };
        let mut config = OmpConfig::new(&format!("{provider}/{model}"));
        config.cli.binary = Some(root.join("bin/omp"));
        config.runtime = Some(bun.clone());
        config
    };
    let backend = OmpSdkBackend::new(config("ok"));
    let home_env = home.join(".env");
    let original_env = std::fs::read_to_string(&home_env).unwrap();
    for line in ["PI_COMPILED=1\n", "PI_NATIVES_DIR=ambient-native-code\n"] {
        std::fs::write(&home_env, format!("{original_env}{line}")).unwrap();
        assert!(
            matches!(backend.execute(&request("ok"), &CancelToken::new()), Err(AiError::ProviderUnavailable(reason)) if reason.contains("native_loader_overrides"))
        );
        assert!(
            lines(&record).is_empty(),
            "native overrides fail before inference"
        );
    }
    std::fs::write(home_env, original_env).unwrap();
    assert_eq!(backend.availability(), Availability::Ready);
    let ModelCatalog::Fixed(models) = backend.models() else {
        panic!("static SDK catalog is labelled fixed");
    };
    assert!(models.contains(&"openai/ok".into()), "{models:?}");
    assert!(!models.iter().any(|model| model.contains("image")));
    assert!(lines(&record).is_empty(), "discovery sends no inference");
    let answer = backend
        .execute(&request("ok"), &CancelToken::new())
        .unwrap();
    assert_eq!(answer.value, json!({"value":"fixture"}));
    assert!(
        answer.usage.models.is_empty(),
        "unknown usage, never duplicated or zeroed"
    );
    let sent = lines(&record);
    let expected_system = format!(
        "EXACT_SYSTEM\n\nAnswer with exactly one JSON object that matches this JSON Schema, with no prose and no code fence:\n{}",
        request("ok").output_schema
    );
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0]["messages"],
        json!([
            {"role":"system","content":expected_system},
            {"role":"user","content":"EXACT_INPUT"}
        ])
    );
    assert!(
        sent[0]
            .get("tools")
            .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
    );
    assert!(!sent[0].to_string().contains("AMBIENT_CONTEXT_CANARY"));

    for model in ["api-anthropic", "api-responses", "api-codex"] {
        let before = lines(&record).len();
        let answer = OmpSdkBackend::new(config(model))
            .execute(&request(model), &CancelToken::new())
            .unwrap();
        assert_eq!(answer.value, json!({"value":"fixture"}), "{model}");
        assert_eq!(
            lines(&record).len(),
            before + 1,
            "one inference for {model}"
        );
        let sent = lines(&record).pop().unwrap();
        if model == "api-anthropic" {
            let blocks = sent["system"].as_array().unwrap();
            assert_eq!(
                blocks.len(),
                3,
                "only upstream protocol framing and feature system"
            );
            let billing = blocks[0]["text"].as_str().unwrap();
            let fingerprint = billing
                .strip_prefix(
                    "x-anthropic-billing-header: cc_version=2.1.280.be3; cc_entrypoint=cli; cch=",
                )
                .unwrap();
            let fingerprint = fingerprint.strip_suffix(';').unwrap();
            assert_eq!(fingerprint.len(), 5);
            assert!(fingerprint.bytes().all(|b| b.is_ascii_hexdigit()));
            assert_eq!(
                blocks[1]["text"],
                "You are Claude Code, Anthropic's official CLI for Claude."
            );
            assert_eq!(blocks[2]["text"], expected_system);
            assert_eq!(sent["messages"][0]["content"], "EXACT_INPUT");
        } else {
            assert_eq!(sent["instructions"], expected_system);
            assert_eq!(sent["input"][0]["content"][0]["text"], "EXACT_INPUT");
        }
        assert!(
            sent.get("tools")
                .is_none_or(|tools| tools.as_array().is_some_and(Vec::is_empty))
        );
    }

    for (model, class) in [
        ("http400", "rejected"),
        ("http413", "rejected"),
        ("http401", "not_authenticated"),
        ("http429", "usage_limited"),
        ("http500", "transient"),
        ("bad-json", "invalid_output"),
    ] {
        let before = lines(&record).len();
        let error = OmpSdkBackend::new(config(model))
            .execute(&request(model), &CancelToken::new())
            .unwrap_err();
        let observed = match error {
            AiError::Rejected(_) => "rejected",
            AiError::NotAuthenticated => "not_authenticated",
            AiError::UsageLimited { .. } => "usage_limited",
            AiError::Transient(_) => "transient",
            AiError::InvalidOutput(_) => "invalid_output",
            _ => "other",
        };
        assert_eq!(observed, class, "{model}: {error:?}");
        assert_eq!(
            lines(&record).len(),
            before + 1,
            "SDK never retries a model request"
        );
    }
    let router = AiRouter::new(
        vec![Arc::new(OmpSdkBackend::new(config("bad-schema")))],
        RouterConfig {
            priority: vec![ProviderId::OMP],
            max_invalid_output_attempts: 1,
            ..Default::default()
        },
        Arc::new(NoopLogSink),
    );
    assert!(
        router
            .execute(&request("bad-schema"), &CancelToken::new())
            .is_err(),
        "existing schema validator rejects wrong value type"
    );
    for model in [
        "terminal-error",
        "oversize",
        "stream-http400",
        "stream-http413",
    ] {
        let before = lines(&record).len();
        assert!(
            matches!(
                OmpSdkBackend::new(config(model)).execute(&request(model), &CancelToken::new()),
                Err(AiError::CompletionUnknown(_))
            ),
            "{model}"
        );
        assert_eq!(
            lines(&record).len(),
            before + 1,
            "lost/oversize responses never replay"
        );
    }

    // Wait for a submitted HTTP request before cancelling, without a sleep.
    while events.try_recv().is_ok() {}
    let cancel = CancelToken::new();
    let token = cancel.clone();
    let hanging = OmpSdkBackend::new(config("hang"));
    let task = std::thread::spawn(move || hanging.execute(&request("hang"), &token));
    assert!(
        events
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
            .contains("hang")
    );
    cancel.cancel();
    assert_eq!(task.join().unwrap().unwrap_err(), AiError::Cancelled);
    // Expiry after confirmed submission also ends the child, without replay.
    let mut short = request("hang");
    short.deadline = Duration::from_secs(3);
    let hanging = OmpSdkBackend::new(config("hang"));
    let task = std::thread::spawn(move || hanging.execute(&short, &CancelToken::new()));
    assert!(
        events
            .recv_timeout(Duration::from_secs(3))
            .unwrap()
            .contains("hang")
    );
    assert_eq!(task.join().unwrap().unwrap_err(), AiError::Timeout);

    // End only the immediate owner, bypassing Drop and tree cleanup. Bun's
    // Unix parent watch / Windows kill-on-close job must end the real bridge.
    let mut owner_command = Command::new(std::env::current_exe().unwrap());
    owner_command
        .args([
            "--ignored",
            "--exact",
            "omp_sdk::abrupt_owner_helper",
            "--nocapture",
        ])
        .env("HIDE_TEST_OMP_OWNER_ROOT", &root)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    let mut owner = OwnedChild::spawn(&mut owner_command).unwrap();
    assert!(
        events
            .recv_timeout(Duration::from_secs(15))
            .unwrap()
            .contains("hang")
    );
    let abrupt = lines(&folder.path().join("children.jsonl")).pop().unwrap();
    let bridge_pid = abrupt["pid"].as_u64().unwrap() as u32;
    hide_platform::process::terminate(owner.id()).unwrap();
    assert!(!owner.wait().unwrap().success());
    let until = Instant::now() + Duration::from_secs(5);
    while hide_platform::process::is_alive(bridge_pid) && Instant::now() < until {
        std::thread::park_timeout(Duration::from_millis(10));
    }
    assert!(
        !hide_platform::process::is_alive(bridge_pid),
        "bridge must die with its owner"
    );
    // Abrupt owner death skips private-file destructors too. Remove this
    // synthetic owner's files explicitly; normal returns are checked below.
    let argv = abrupt["argv"].as_array().unwrap();
    let request_path = Path::new(argv.last().unwrap().as_str().unwrap());
    let body: Value =
        serde_json::from_str(&std::fs::read_to_string(request_path).unwrap()).unwrap();
    for path in [
        Path::new(argv[1].as_str().unwrap()),
        request_path,
        Path::new(body["auth_cache"].as_str().unwrap()),
    ] {
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    // Unproven packages and missing exports fail before paid inference.
    let manifest = root.join("package.json");
    let original = std::fs::read_to_string(&manifest).unwrap();
    let mut meta: Value = serde_json::from_str(&original).unwrap();
    meta["version"] = json!("99.0.0");
    std::fs::write(&manifest, meta.to_string()).unwrap();
    assert!(matches!(
        backend.execute(&request("ok"), &CancelToken::new()),
        Err(AiError::ProviderUnavailable(_))
    ));
    std::fs::write(&manifest, original).unwrap();
    let original = std::fs::read_to_string(&manifest).unwrap();
    let mut meta: Value = serde_json::from_str(&original).unwrap();
    meta["engines"]["bun"] = json!(">=99.0.0");
    std::fs::write(&manifest, meta.to_string()).unwrap();
    assert!(
        matches!(backend.execute(&request("ok"), &CancelToken::new()), Err(AiError::ProviderUnavailable(reason)) if reason.contains("bun_does_not_meet_package_engines"))
    );
    std::fs::write(&manifest, original).unwrap();
    let catalog_manifest = root.join("node_modules/@oh-my-pi/pi-catalog/package.json");
    let catalog_original = std::fs::read_to_string(&catalog_manifest).unwrap();
    let mut meta: Value = serde_json::from_str(&catalog_original).unwrap();
    meta["exports"] = json!({});
    std::fs::write(&catalog_manifest, meta.to_string()).unwrap();
    assert!(
        matches!(
            backend.execute(&request("ok"), &CancelToken::new()),
            Err(AiError::ProviderUnavailable(_))
        ),
        "missing public export fails before submission"
    );
    std::fs::write(catalog_manifest, catalog_original).unwrap();
    let catalog_manifest = root.join("node_modules/@oh-my-pi/pi-catalog/package.json");
    let original = std::fs::read_to_string(&catalog_manifest).unwrap();
    let mut meta: Value = serde_json::from_str(&original).unwrap();
    meta["version"] = json!("99.0.0");
    std::fs::write(&catalog_manifest, meta.to_string()).unwrap();
    assert!(
        matches!(backend.execute(&request("ok"), &CancelToken::new()), Err(AiError::ProviderUnavailable(reason)) if reason.contains("dependency_version_mismatch"))
    );
    std::fs::write(catalog_manifest, original).unwrap();
    let npm = home.join("npm");
    std::fs::create_dir_all(npm.join("node_modules/@oh-my-pi")).unwrap();
    let shim = npm.join("omp.cmd");
    std::fs::write(&shim, "shim must never execute").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&root, npm.join("node_modules/@oh-my-pi/pi-coding-agent")).unwrap();
    #[cfg(windows)]
    {
        // A regular package manifest is enough for locator coverage; exports
        // are deliberately absent so the SDK load stops before inference.
        let target = npm.join("node_modules/@oh-my-pi/pi-coding-agent");
        std::fs::create_dir(&target).unwrap();
        std::fs::copy(root.join("package.json"), target.join("package.json")).unwrap();
    }
    let mut shim_config = config("ok");
    shim_config.cli.binary = Some(shim);
    let result = OmpSdkBackend::new(shim_config).execute(&request("ok"), &CancelToken::new());
    #[cfg(unix)]
    assert!(result.is_ok());
    #[cfg(windows)]
    assert!(
        matches!(result, Err(AiError::ProviderUnavailable(reason)) if !reason.contains("sdk_package_required"))
    );
    let standalone = home.join("omp-standalone");
    std::fs::write(&standalone, "standalone is never invoked").unwrap();
    let mut missing = config("ok");
    missing.cli.binary = Some(standalone);
    assert!(
        matches!(OmpSdkBackend::new(missing).execute(&request("ok"), &CancelToken::new()), Err(AiError::ProviderUnavailable(reason)) if reason.contains("sdk_package_required"))
    );
    let mut large = request("ok");
    large.input = "x".repeat(1024 * 1024);
    assert!(matches!(
        backend.execute(&large, &CancelToken::new()),
        Err(AiError::Rejected(_))
    ));
    let before = lines(&record).len();
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert_eq!(
        backend.execute(&request("ok"), &cancelled).unwrap_err(),
        AiError::Cancelled
    );
    assert_eq!(lines(&record).len(), before);
    assert!(!marker.exists(), "no custom module was executed");
    assert!(
        !home.join(".omp/logs/http-400-requests").exists(),
        "no SDK request dumps under real auth home"
    );
    for child in lines(&folder.path().join("children.jsonl")) {
        assert!(!hide_platform::process::is_alive(
            child["pid"].as_u64().unwrap() as u32
        ));
        assert!(
            !Path::new(child["cwd"].as_str().unwrap()).exists(),
            "private request directory removed"
        );
        assert!(
            !child["argv"].to_string().contains("EXACT_INPUT"),
            "input stays out of process arguments"
        );
    }
    assert!(!std::fs::read_dir(&home).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("hide-ai-prompt-")
    }));
    drop(owner_stdin);
    assert!(child.wait().unwrap().success());
    reader.join().unwrap();
}
