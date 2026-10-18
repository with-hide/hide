//! Exercise the shipped helper boundary without an operator home or session.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use hide_host::protocol::{Call, KitAction, PROTOCOL_VERSION};
use hide_platform::process::{OwnedChild, kill_tree, start_time, terminate};
use serde_json::{Value, json};

const HELPER: &str = env!("CARGO_BIN_EXE_hided");
const DEADLINE: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: usize = 64 * 1024;
// Only this test binary's explicitly selected owner fixture consumes this key.
const OWNER_HOME: &str = "HIDE_TEST_HOST_HELPER_OWNER_HOME";
const PRIVATE_BODY: &str = "private conversation is deliberately not JSON";

fn command(executable: &Path, home: &Path) -> Command {
    let mut command = Command::new(executable);
    // No inherited pane, provider override or owner metadata can reach a fixture.
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("HIDE_") || key.to_string_lossy().starts_with("HERDR_")
        {
            command.env_remove(key);
        }
    }
    command
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("PATH", "")
        // The helper asks the login shell for the account's PATH and logs a
        // missing `SHELL` or an ask that failed; a stand-in that answers
        // keeps the test off the shell and startup files of the machine
        // that runs it. Ubuntu's `/etc/profile` sets no PATH, so `/bin/sh`
        // there answers an empty one, which is logged as unread.
        .env("SHELL", login_shell(home))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// A login shell in `home` that answers the helper's ask with the system's
/// folders, as a login profile sets them. Written once, before any helper
/// starts it.
fn login_shell(home: &Path) -> PathBuf {
    let shell = home.join("login-shell");
    if !shell.exists() {
        fs::write(
            &shell,
            "#!/bin/sh\n[ \"$1\" = -ilc ] || exit 64\nPATH=/usr/bin:/bin\nexport PATH\neval \"$2\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&shell, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    shell
}

fn transcript(home: &Path, agent: &str) -> PathBuf {
    let (root, header) = match agent {
        "claude" => (
            home.join(".claude/projects/project"),
            json!({"type":"user", "sessionId":"private-native-claude"}),
        ),
        "codex" => (
            home.join(".codex/sessions/2026/01/01"),
            json!({"type":"session_meta", "payload":{"id":"private-native-codex"}}),
        ),
        _ => unreachable!(),
    };
    fs::create_dir_all(&root).unwrap();
    let path = root.join("private-native.jsonl");
    fs::write(&path, format!("{header}\n{PRIVATE_BODY}\n")).unwrap();
    path
}

fn activity(id: u64, agent: &str, kind: &str, reference: &str) -> Value {
    json!({"id":id, "op":"session_activity", "request":{
        "agent":agent, "reference_kind":kind, "reference_value":reference
    }})
}

fn serve(
    executable: &Path,
    home: &Path,
    requests: &[Value],
    guarded: bool,
) -> BTreeMap<u64, Value> {
    let started = Instant::now();
    let deadline = started + DEADLINE;
    let mut command = command(executable, home);
    command.args(["node", "serve"]);
    let mut child = if guarded {
        OwnedChild::spawn_guarded(command, deadline).unwrap()
    } else {
        OwnedChild::spawn(&mut command).unwrap()
    };
    let input = requests
        .iter()
        .map(|request| format!("{request}\n"))
        .collect::<String>();
    // Keep writes below the smallest pipe buffer; capture has its own byte cap.
    assert!(input.len() < 4096);
    child
        .take_stdin()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.capture_until(deadline, OUTPUT_LIMIT).unwrap();
    assert!(output.status.success());
    assert!(
        output.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        child.try_wait().unwrap().is_some(),
        "normal EOF must reap the helper"
    );
    assert!(started.elapsed() < DEADLINE);
    let mut answers = BTreeMap::new();
    // The server is concurrent: request IDs, never arrival order, pair answers.
    for line in output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let answer: Value = serde_json::from_slice(line).unwrap();
        let id = answer["id"].as_u64().unwrap();
        assert!(answers.insert(id, answer).is_none(), "duplicate answer ID");
    }
    assert_eq!(answers.len(), requests.len());
    answers
}

fn assert_activity(answer: &Value, path: &Path) {
    let metadata = fs::metadata(path).unwrap();
    let modified = metadata
        .modified()
        .unwrap()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    assert_eq!(
        answer["ok"],
        json!({"modified_at_unix_ms":modified, "bytes":metadata.len()})
    );
}

#[test]
fn serve_dispatches_activity_success_and_path_free_refusals() {
    let home = tempfile::tempdir().unwrap();
    let home_path = home.path().canonicalize().unwrap();
    let claude = transcript(&home_path, "claude");
    let codex = transcript(&home_path, "codex");
    let outside = home_path.join("private-outside.jsonl");
    fs::copy(&codex, &outside).unwrap();
    let missing = codex.with_file_name("private-missing.jsonl");
    let unconfirmed = codex.with_file_name("private-unconfirmed.jsonl");
    fs::write(&unconfirmed, format!("{PRIVATE_BODY}\n")).unwrap();
    let requests = [
        activity(91, "claude", "path", &claude.to_string_lossy()),
        activity(4, "codex", "path", &codex.to_string_lossy()),
        activity(70, "codex", "path", &outside.to_string_lossy()),
        activity(8, "codex", "opaque", "private-native-codex"),
        activity(42, "codex", "path", &missing.to_string_lossy()),
        activity(12, "codex", "path", &unconfirmed.to_string_lossy()),
    ];
    for guarded in [false, true] {
        let answers = serve(Path::new(HELPER), &home_path, &requests, guarded);
        assert_activity(&answers[&91], &claude);
        assert_activity(&answers[&4], &codex);
        for (id, code, reason) in [
            (70, "io", "label_session_outside_roots"),
            (8, "unsupported", "session_kind_unsupported"),
            (42, "not_found", "session_file_missing"),
            (12, "io", "label_session_metadata_unconfirmed"),
        ] {
            assert_eq!(
                answers[&id]["error"],
                json!({"code":code, "message":reason})
            );
        }
        let serialized = serde_json::to_string(&answers).unwrap();
        for private in [
            home_path.to_string_lossy().as_ref(),
            "private-native",
            PRIVATE_BODY,
        ] {
            assert!(
                !serialized.contains(private),
                "activity exposed private content"
            );
        }
    }
}

#[test]
fn serve_preserves_kit_protocol_alongside_activity() {
    let home = tempfile::tempdir().unwrap();
    let home_path = home.path().canonicalize().unwrap();
    let codex = transcript(&home_path, "codex");
    // A disposable copy has the kit's real install layout, never an installed helper.
    let build = home_path.join("fixture-helper/0123456789abcdef");
    fs::create_dir_all(&build).unwrap();
    let helper = build.join(Path::new(HELPER).file_name().unwrap());
    // Materialize independent bytes: an APFS clone of a debug executable can
    // stall before main on its first execution. This
    // fixture tests the helper protocol, not cloned-file launch latency.
    let mut source = fs::File::open(HELPER).unwrap();
    let mut destination = fs::File::create(&helper).unwrap();
    std::io::copy(&mut source, &mut destination).unwrap();
    destination
        .set_permissions(source.metadata().unwrap().permissions())
        .unwrap();
    drop(destination);
    let reinstall = json!({"op":"kit", "action":{
        "kind":"reinstall", "components":["cli"], "agents_on":["pi"], "agents_off":["cursor"]
    }, "cli_dir":"relative-refused", "herdr_socket":null});
    let parsed: Call = serde_json::from_value(reinstall.clone()).unwrap();
    assert!(
        matches!(parsed, Call::Kit {action: KitAction::Reinstall {ref components, ref agents_on, ref agents_off, ..}, ..}
        if components == &[hide_kit::ComponentId::Cli]
            && agents_on == &["pi"]
            && agents_off == &["cursor"])
    );
    let mut reinstall_request = reinstall;
    reinstall_request["id"] = json!(6);
    let answers = serve(
        &helper,
        &home_path,
        &[
            json!({"id":25,"op":"hello"}),
            activity(3, "codex", "path", &codex.to_string_lossy()),
            json!({"id":17,"op":"kit", "action":{"kind":"status"}, "cli_dir":"~/bin", "herdr_socket":"~/.hide/fixture.sock"}),
            reinstall_request,
        ],
        true,
    );
    assert_eq!(answers[&25]["ok"]["protocol"], PROTOCOL_VERSION);
    let identity = serde_json::from_value::<hide_host::protocol::MachineIdentity>(
        answers[&25]["ok"]["machine_identity"].clone(),
    )
    .unwrap()
    .into_result()
    .unwrap();
    assert_eq!(
        identity,
        hide_platform::host::machine_id().unwrap(),
        "the private helper and local daemon use the same native machine identity"
    );
    assert_activity(&answers[&3], &codex);
    let components = answers[&17]["ok"]["components"].as_array().unwrap();
    assert!(components.iter().any(|part| part["id"] == "cli"));
    assert!(
        !components.iter().any(|part| part["id"] == "codex_per_pane"),
        "the kit no longer carries a Codex daemon part"
    );
    // A recognized reinstall reaches path validation; it is not an unknown operation.
    assert_eq!(answers[&6]["error"]["code"], "invalid_path");
}

struct FixtureProcess {
    pid: u32,
    birth: u64,
}

impl FixtureProcess {
    fn live(&self) -> bool {
        start_time(self.pid).ok() == Some(self.birth)
    }
}

impl Drop for FixtureProcess {
    fn drop(&mut self) {
        if self.live()
            && let Err(error) = kill_tree(self.pid)
        {
            eprintln!(
                "host_helper.fixture_cleanup_failed pid={} error={error}",
                self.pid
            );
        }
    }
}

#[test]
#[ignore = "selected only by the abrupt-owner-loss fixture"]
fn owner_fixture() {
    let _owner_watch = hide_platform::process::OwnerWatch::from_launch()
        .unwrap()
        .expect("the owner fixture must itself have an owner");
    let home = PathBuf::from(std::env::var_os(OWNER_HOME).expect("fixture home"));
    let mut command = command(Path::new(HELPER), &home);
    command
        .args(["node", "serve"])
        .stdin(Stdio::inherit())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = OwnedChild::spawn_guarded(command, Instant::now() + DEADLINE).unwrap();
    let identity = json!({"pid":child.id(),"birth":start_time(child.id()).unwrap()});
    fs::write(home.join("ready.json"), identity.to_string()).unwrap();
    // The outer test holds the writer even after killing this owner.
    // Without OwnerWatch the actual helper would remain blocked on the same stdin.
    let mut byte = [0];
    std::io::stdin().read_exact(&mut byte).unwrap();
    drop(child);
}

/// The helper the owner fixture names in `ready.json`, once the file holds
/// a whole identity; it is written by another process, so it is polled.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn announced_helper(home: &Path, owner: &mut OwnedChild, deadline: Instant) -> FixtureProcess {
    loop {
        if let Ok(bytes) = fs::read(home.join("ready.json"))
            && let Ok(identity) = serde_json::from_slice::<Value>(&bytes)
        {
            return FixtureProcess {
                pid: identity["pid"].as_u64().unwrap().try_into().unwrap(),
                birth: identity["birth"].as_u64().unwrap(),
            };
        }
        if Instant::now() >= deadline {
            let output = owner.capture_until(deadline, OUTPUT_LIMIT);
            panic!("owner fixture did not start the helper: {output:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Whether `helper` ended by `deadline`. It is not this test's child, so
/// nothing reports its end but its identity going away.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn ended_by(helper: &FixtureProcess, deadline: Instant) -> bool {
    while helper.live() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    !helper.live()
}

#[test]
fn helper_exits_when_owner_dies_with_stdin_still_open() {
    let home = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let deadline = started + DEADLINE;
    let mut command = command(&std::env::current_exe().unwrap(), home.path());
    command
        .args([
            "--exact",
            "node_session_activity::owner_fixture",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(OWNER_HOME, home.path());
    let mut owner = OwnedChild::spawn_guarded(command, deadline).unwrap();
    let held_stdin = owner.take_stdin().unwrap();
    let helper = announced_helper(home.path(), &mut owner, deadline);
    assert!(helper.live(), "fixture must name a live helper");
    // Kill only the owner, not its tree: OwnerWatch must perform the cleanup.
    terminate(owner.id()).unwrap();
    assert!(
        ended_by(&helper, deadline),
        "helper survived its owner with stdin open"
    );
    drop(held_stdin);
    let output = owner.capture_until(deadline, OUTPUT_LIMIT).unwrap();
    assert!(!output.status.success());
    assert!(owner.try_wait().unwrap().is_some());
    assert!(started.elapsed() < DEADLINE);
}

#[test]
fn kit_protocol_carries_device_checkout_paths_and_accepts_older_requests() {
    let old: Call = serde_json::from_value(
        json!({"op":"kit", "action":{"kind":"status"}, "cli_dir":"~/bin", "herdr_socket":null}),
    )
    .unwrap();
    assert!(matches!(old, Call::Kit { retirement_projects, .. } if retirement_projects.is_empty()));
    let current: Call = serde_json::from_value(json!({"op":"kit", "action":{"kind":"apply"}, "cli_dir":"~/bin", "herdr_socket":null, "retirement_projects":["/checkout/on-this-device"]})).unwrap();
    assert!(
        matches!(current, Call::Kit { retirement_projects, .. } if retirement_projects == ["/checkout/on-this-device"])
    );
}
