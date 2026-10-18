//! The real agy binary reads the `hide` block Hide writes (PRD
//! antigravity-first-class B36). Ignored by default: the OS contract lane
//! installs agy on a runner and names it with `HIDE_AGY_BINARY`, and the
//! runner's throwaway home is the only home it may run in. Never run it on a
//! machine where agy is signed in: a home other than the account's loses
//! agy's keychain sign-in, and agy then asks to sign in again.
//!
//! agy reads every `hooks.json` when it starts, before it signs in, and logs
//! `loaded <n> named hooks from <m> hooks.json file(s)` (agy 1.4.0); a block
//! it cannot read is logged as a failure instead. Signed in, `/hooks` also
//! lists each loaded entry, which the test checks when agy answers.

use std::path::Path;
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use hide_agent_hooks::guidance::{self, GuidanceAgent};

#[test]
#[ignore = "runs the real agy named by HIDE_AGY_BINARY in a throwaway home; the os contract lane installs it"]
#[allow(clippy::disallowed_methods)] // env_clear: the real agy runs in a throwaway home with only what the test names
fn the_real_agy_loads_hide_s_hook_block() {
    let agy = std::env::var_os("HIDE_AGY_BINARY")
        .expect("HIDE_AGY_BINARY names the agy binary this test runs");
    let home = tempfile::tempdir().unwrap();
    for folder in [".gemini/config", ".gemini/antigravity-cli"] {
        std::fs::create_dir_all(home.path().join(folder)).unwrap();
    }
    let helper = Path::new(env!("CARGO_BIN_EXE_hide-agent-hooks"));
    guidance::install(GuidanceAgent::Antigravity, home.path(), helper).unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let log = workspace.path().join("agy.log");

    let mut command = Command::new(&agy);
    command
        .env_clear()
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("TMPDIR", workspace.path())
        .env("TEMP", workspace.path())
        .env("TMP", workspace.path())
        // Nothing on PATH: a signed-out agy finds no browser to open.
        .env("PATH", workspace.path().join("no-programs"))
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "1")
        .args(["-p", "/hooks", "--output-format", "json", "--log-file"])
        .arg(&log)
        .current_dir(workspace.path());
    for name in ["SystemRoot", "windir", "ComSpec"] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    // The nightly's CI account (`agy-contract.yml`, `live`). agy runs against
    // the Gemini API without signing in when its settings say
    // `"modelProvider": "gemini"` and `GEMINI_API_KEY` holds the key (agy's
    // changelog, in the 1.4.0 binary).
    let key = std::env::var_os("HIDE_AGY_API_KEY").filter(|key| !key.is_empty());
    if let Some(key) = &key {
        std::fs::write(
            home.path().join(".gemini/antigravity-cli/settings.json"),
            r#"{"modelProvider": "gemini"}"#,
        )
        .unwrap();
        command.env("GEMINI_API_KEY", key);
    }
    let finished = hide_platform::process::run_to_end(
        &mut command,
        Duration::from_secs(120),
        &AtomicBool::new(false),
    )
    .expect("agy runs to its end");
    let logged = std::fs::read_to_string(&log).unwrap_or_default();
    let report = format!(
        "exit {:?}\nstdout {}\nstderr {}\nlog {}",
        finished.code, finished.stdout, finished.stderr, logged
    );
    assert!(
        !logged.contains("Failed to parse hooks file")
            && !logged.contains("parsing existing hooks"),
        "agy could not read Hide's block:\n{report}"
    );
    assert!(
        logged.contains("loaded 1 named hooks from 1 hooks.json file(s)"),
        "agy did not report loading Hide's block:\n{report}"
    );
    // Signed in (the nightly's account), `/hooks` lists Hide's four events.
    let answer: Option<serde_json::Value> = serde_json::from_str(finished.stdout.trim()).ok();
    let signed_in = answer.filter(|answer| answer["status"] == "SUCCESS");
    // A key agy does not sign in with fails here: a runner has no browser
    // for agy's own sign-in, and nothing on PATH could open one.
    assert!(
        key.is_none() || signed_in.is_some(),
        "agy did not run on HIDE_AGY_API_KEY (GEMINI_API_KEY with modelProvider gemini); it wants a browser sign-in, which a CI runner cannot give:\n{report}"
    );
    if let Some(answer) = signed_in {
        let listed = answer["response"].as_str().unwrap_or_default();
        for event in ["SessionStart", "PreInvocation", "PreToolUse", "Stop"] {
            assert!(
                listed
                    .lines()
                    .any(|line| line.starts_with(&format!("hide\tenabled\t{event}\t"))),
                "{event} is not among agy's hooks:\n{report}"
            );
        }
    }
}

/// The stand-in `hide` beside the helper in a signed-in turn: it registers
/// the checkout, hands the pane one letter and records every call beside the
/// home it is run in.
#[cfg(unix)]
const TURN_HIDE: &str = r#"#!/bin/sh
echo "$@" >> "${HOME%/*}/hide-calls"
case "$1 $2" in
  "workspace bootstrap") echo '{"ok":true,"reference":"/x"}' ;;
  "inbox --hook") echo '{"ok":true,"result":{"context":"LETTER-CI-7731: the operator says hello.","ids":["letter-ci"]}}' ;;
  "inbox --confirm") echo '{"ok":true,"result":{"confirmed":["letter-ci"]}}' ;;
  *) exit 1 ;;
esac
"#;

/// One real agy turn on the CI account's key (PRD antigravity-first-class
/// B36, D-17): Hide's block in the throwaway home's `hooks.json`, the helper
/// beside a stand-in `hide`, and a prompt asking for one direct
/// `herdr agent start`. agy fires Hide's hooks, injects the pane's letter
/// into the conversation (agy records it, and the helper confirms it only
/// after agy took the answer), and the start is refused with Hide's reason
/// before agy's own permission check. The turn runs on `HIDE_AGY_MODEL`,
/// the model the key's free tier serves (`gemini-3.1-pro`, agy's default in
/// API-key mode, has a quota of 0), and the refusal needs that model to make
/// the call it is asked for. A turn the provider answered nothing because
/// the key's requests for the day were spent leaves the refusal
/// unmeasured: it says so in a `Not measured` warning and passes. Unix only:
/// the stand-in is a shell script.
#[cfg(unix)]
#[test]
#[ignore = "runs one real agy turn on HIDE_AGY_API_KEY in a throwaway home; the os contract lane's signed-in job runs it"]
#[allow(clippy::disallowed_methods)] // env_clear: the real agy runs in a throwaway home with only what the test names
fn a_signed_in_agy_turn_fires_hide_s_hooks() {
    use std::io::Write;

    use crate::{programs, stand_ins};

    let agy = std::env::var_os("HIDE_AGY_BINARY")
        .expect("HIDE_AGY_BINARY names the agy binary this test runs");
    let key = std::env::var_os("HIDE_AGY_API_KEY")
        .filter(|key| !key.is_empty())
        .expect("HIDE_AGY_API_KEY holds the CI account's key");
    let model = std::env::var("HIDE_AGY_MODEL")
        .expect("HIDE_AGY_MODEL names the model the CI key's quota serves");
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let home = root.join("home");
    let bin = root.join("bin");
    let workspace = root.join("repo");
    for folder in [
        home.join(".gemini/config"),
        home.join(".gemini/antigravity-cli"),
        bin.clone(),
        workspace.join(".git"),
    ] {
        std::fs::create_dir_all(folder).unwrap();
    }
    std::fs::write(workspace.join(".git/HEAD"), "ref: refs/heads/topic\n").unwrap();
    std::fs::write(
        home.join(".gemini/antigravity-cli/settings.json"),
        r#"{"modelProvider": "gemini"}"#,
    )
    .unwrap();
    let helper = bin.join("hide-agent-hooks");
    stand_ins::place(programs::hook(), &helper);
    stand_ins::program(&bin.join("hide"), TURN_HIDE);
    guidance::install(GuidanceAgent::Antigravity, &home, &helper).unwrap();
    let log = root.join("agy.log");
    let prompt = serde_json::json!({
        "event": "user",
        "message": { "content": "Run exactly one shell command with your shell tool, unchanged, and nothing else: herdr agent start probe --kind claude --pane w1:p1 . Do not read files or run any other command. Then report the exact error text the tool returned, word for word." },
    });

    let mut command = Command::new(&agy);
    command
        .env_clear()
        .env("HOME", &home)
        .env("TMPDIR", &root)
        // The shell agy runs its tools and hooks in; no browser opener is
        // needed, since the key signs agy in.
        .env("PATH", "/usr/bin:/bin")
        .env("AGY_CLI_DISABLE_AUTO_UPDATE", "1")
        .env("GEMINI_API_KEY", &key)
        .env("HERDR_PANE_ID", "ci-agy:p1")
        .env("HERDR_SOCKET_PATH", root.join("no-herdr.sock"))
        .args([
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
        ])
        .args(["--model", &model])
        // Inside the test's 240 s guard (`.config/nextest.toml`), so a
        // slow turn ends with agy's own report.
        .args(["--print-timeout", "180s", "-p", "", "--log-file"])
        .arg(&log)
        .current_dir(&workspace)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = hide_platform::process::OwnedChild::spawn(&mut command).unwrap();
    let mut stdin = child.take_stdin().unwrap();
    writeln!(stdin, "{prompt}").unwrap();
    drop(stdin);
    let output = child
        .capture_until(
            std::time::Instant::now() + Duration::from_secs(200),
            1024 * 1024,
        )
        .expect("agy ends its turn");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let calls = std::fs::read_to_string(root.join("hide-calls")).unwrap_or_default();
    let report = format!(
        "exit {:?}\nhide calls {calls}\nstderr {stderr}\nlog {}",
        output.status.code(),
        std::fs::read_to_string(&log).unwrap_or_default(),
    );
    let events: Vec<serde_json::Value> = stdout
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let conversation = events
        .iter()
        .find_map(|event| event["conversation_id"].as_str())
        .unwrap_or_else(|| panic!("agy started no conversation:\n{report}"));

    // The hooks fired: the guidance and Memory at the first prompt, the
    // letter at its first model call.
    for asked in [
        format!(
            "workspace memory --event SessionStart --runtime antigravity --session {conversation}"
        ),
        format!("inbox --hook --session {conversation}"),
        "inbox --confirm letter-ci".to_owned(),
    ] {
        assert!(
            calls.lines().any(|line| line == asked),
            "{asked} was not asked:\n{report}"
        );
    }
    // Injected: agy recorded the letter as a message of the conversation.
    let transcript = std::fs::read_to_string(
        home.join(".gemini/antigravity-cli/brain")
            .join(conversation)
            .join(".system_generated/logs/transcript_full.jsonl"),
    )
    .unwrap_or_default();
    assert!(
        transcript.contains("LETTER-CI-7731"),
        "the letter is not in the conversation:\n{report}"
    );
    // One request of the key's day per model response agy records.
    let responses = transcript
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|step| step["type"] == "PLANNER_RESPONSE")
        .count();
    // The start reaches Hide only through the model's tool call. When the
    // provider answered nothing because the key's requests for the day were
    // spent, the refusal is not measured: said where the run's reader and the
    // nightly report see it, and passed (docs/TESTING.md, A real agent on
    // CI). A model that answered and then ran out reads as any other failure.
    // The line starts on its own, since libtest may have left its own text on
    // the current one.
    if responses == 0
        && let Some(quotas) = spent_quota(&stderr)
    {
        println!(
            "\n::warning title=Not measured::agy_binary::a_signed_in_agy_turn_fires_hide_s_hooks%0Aprovider quota exhausted ({}): the hooks fired and the letter went in, the refused start was not reached",
            quotas.join(", ")
        );
        return;
    }
    // Refused: the start ended as agy's tool error with Hide's reason.
    let refused = events.iter().any(|event| {
        let step = &event["step_update"];
        step["tool_name"] == "run_command"
            && step["state"] == "ERROR"
            && step["tool_info"]["error"]["message"]
                .as_str()
                .is_some_and(|message| {
                    message.contains("denied by pre-tool hook")
                        && message.contains("hide agent spawn --parent here")
                })
    });
    assert!(
        refused,
        "agy's start was not refused with Hide's reason:\n{stdout}\n{report}"
    );
    // What a passing turn spent, for the budget in `agy-contract.yml`.
    println!("agy turn: {responses} model responses");
}

/// The quotas named when agy's turn ended on the provider refusing a quota
/// the key had and spent, by `contracts/provider-quota.json`: the
/// `AGY_ERROR:` report agy writes to stderr carries Gemini's code and
/// status, and each violation of its `google.rpc.QuotaFailure` is one of the
/// spent quotas the contract names with a `quotaValue` above 0. A quota of 0
/// (a model the key is not served), any other quota, a 503 that never
/// recovered and every other error are failures.
fn spent_quota(stderr: &str) -> Option<Vec<String>> {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../contracts/provider-quota.json")).unwrap();
    let gemini = &contract["gemini"];
    let report: serde_json::Value = stderr
        .lines()
        .find_map(|line| serde_json::from_str(line.strip_prefix("AGY_ERROR: ")?).ok())?;
    if report["error_code"] != gemini["error_code"] || report["status"] != gemini["status"] {
        return None;
    }
    let spent = gemini["spent_quota_ids"].as_array().unwrap();
    let (_, failure) = report["short_error"]
        .as_str()?
        .split_once("google.rpc.QuotaFailure violations:")?;
    // agy prints the details as Go maps, whose keys are sorted, so each
    // violation reads `quotaId:<id> quotaMetric:<metric> quotaValue:<n>]`,
    // and the API leaves out a value of 0.
    let mut quotas = Vec::new();
    for violation in failure.split("quotaId:").skip(1) {
        let mut fields = violation.split(']').next()?.split_whitespace();
        let id = fields.next()?;
        let value: u64 = fields
            .find_map(|field| field.strip_prefix("quotaValue:"))?
            .parse()
            .ok()?;
        if value == 0 || !spent.iter().any(|quota| quota == id) {
            return None;
        }
        quotas.push(id.to_owned());
    }
    (!quotas.is_empty()).then_some(quotas)
}

#[test]
fn only_a_spent_quota_leaves_a_signed_in_turn_unmeasured() {
    let error = |report: &str| format!("error: agent executor error\n{report}\n");
    let spent = include_str!("../fixtures/agy-errors/quota-spent.txt");
    assert_eq!(
        spent_quota(&error(spent)),
        Some(vec![
            "GenerateRequestsPerDayPerProjectPerModel-FreeTier".to_owned()
        ])
    );
    // A model the key has no quota for is a wrong model, not a spent day.
    let zero = include_str!("../fixtures/agy-errors/quota-zero.txt");
    assert_eq!(spent_quota(&error(zero)), None);
    // Another quota, such as the input tokens a turn sends in a minute, can
    // be what Hide's own context spent.
    for other in [
        "GenerateRequestsPerMinutePerProjectPerModel-FreeTier",
        "GenerateContentInputTokensPerModelPerMinute-FreeTier",
    ] {
        let other = spent.replace("GenerateRequestsPerDayPerProjectPerModel-FreeTier", other);
        assert_eq!(spent_quota(&error(&other)), None, "{other}");
    }
    // The provider overloaded past agy's own retries (run 38958746826 logged
    // this 503 before its quota answer).
    let overloaded = r#"AGY_ERROR: {"short_error":"agent executor error: generating and executing: Error 503, Message: This model is currently experiencing high demand. Spikes in demand are usually temporary. Please try again later., Status: UNAVAILABLE, Details: []","status":"UNAVAILABLE","error_code":503,"code_kind":"http","retryable":true}"#;
    assert_eq!(spent_quota(&error(overloaded)), None);
    // The same words outside agy's report are not its answer.
    assert_eq!(spent_quota(&spent.replacen("AGY_ERROR: ", "", 1)), None);
}

/// Writes the `hide` block this build installs into the `.agents/hooks.json`
/// of the scratch workspace `HIDE_AGY_LIVE_WORKSPACE`, for the live check on
/// a signed-in machine, which isolates by workspace and never by home (PRD
/// D-18). It installs into a throwaway home and copies the block out, so the
/// account's own `~/.gemini` is not touched.
#[test]
#[ignore = "writes Hide's agy block into the scratch workspace HIDE_AGY_LIVE_WORKSPACE for a live check"]
fn writes_hide_s_block_into_a_live_check_workspace() {
    let workspace = std::env::var_os("HIDE_AGY_LIVE_WORKSPACE")
        .expect("HIDE_AGY_LIVE_WORKSPACE names the scratch workspace");
    let helper = std::env::var_os("HIDE_AGY_LIVE_HELPER").map_or_else(
        || Path::new(env!("CARGO_BIN_EXE_hide-agent-hooks")).to_path_buf(),
        Into::into,
    );
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".gemini/config")).unwrap();
    guidance::install(GuidanceAgent::Antigravity, home.path(), &helper).unwrap();
    let installed: serde_json::Value = serde_json::from_slice(
        &std::fs::read(home.path().join(".gemini/config/hooks.json")).unwrap(),
    )
    .unwrap();
    let target = Path::new(&workspace).join(".agents/hooks.json");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(
        &target,
        serde_json::to_vec_pretty(&serde_json::json!({ "hide": installed["hide"] })).unwrap(),
    )
    .unwrap();
}
