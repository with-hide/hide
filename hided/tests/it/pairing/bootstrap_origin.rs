//! D-36/B46/D-70: a guest pane cannot become the host's operator by
//! clearing its hints and sending requests directly to the host socket.
//! The real local socket and kernel process tree are necessary here: a
//! request's declared pane or installation is precisely what is absent.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use hide_platform::process::run_to_end;
use serde_json::{Value, json};

const TEST: &str =
    "pairing::bootstrap_origin::a_guest_holder_child_cannot_become_the_host_operator_without_hints";
// Only this test's private children receive a role. Production reads none
// of it; the ordinary suite always executes the complete socket journey.
const ROLE: &str = "HIDE_TEST_BOOTSTRAP_ORIGIN_ROLE";
const PROBE: &str = "probe";
const HOLDER: &str = "holder";
/// Linux: a process of this account whose image the kernel names to no
/// process of the account, as the one a root sshd keeps for each login is.
#[cfg(target_os = "linux")]
const UNNAMED: &str = "unnamed";

fn command(program: &Path, role: &str, folder: &Path) -> Command {
    let mut command = Command::new(program);
    command
        .args(["--exact", TEST, "--nocapture"])
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().to_uppercase().starts_with("HIDE_") {
            command.env_remove(key);
        }
    }
    command.env(ROLE, role);
    command
}

fn run(mut command: Command) -> Vec<Value> {
    let output = run_to_end(
        &mut command,
        Duration::from_secs(60), // Hang guard for the socket exchange and subprocess exit.
        &std::sync::atomic::AtomicBool::new(false),
    )
    .unwrap();
    assert!(output.succeeded(), "{output:?}");
    let text = output.stdout;
    let line = text
        .lines()
        .find_map(|line| line.strip_prefix("BOOTSTRAP_REPLIES="))
        .unwrap_or_else(|| panic!("no answers: {text}"));
    serde_json::from_str(line).unwrap()
}

fn configuration() -> Value {
    serde_json::from_slice(&std::fs::read("probe.json").unwrap()).unwrap()
}

/// Only invoked by this test's private subprocess dispatch.
fn socket_probe() {
    let config = configuration();
    let mut answers = Vec::new();
    for request in config["requests"].as_array().unwrap() {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut stream = hide_platform::ipc::LocalStream::connect_until(
            Path::new(config["socket"].as_str().unwrap()),
            deadline,
        )
        .unwrap();
        writeln!(stream, "{request}").unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        answers.push(serde_json::from_str::<Value>(&line).unwrap());
    }
    println!("BOOTSTRAP_REPLIES={}", json!(answers));
}

/// Its executable lives where the guest's holder runs; the socket peer is
/// its child running the original test executable, with no production hints.
fn holder_process() {
    let config = configuration();
    #[cfg(unix)]
    if config["delete_image"] == true {
        // The same-account ancestor stays alive, but its image can no
        // longer be proved from the kernel's path. Never call it outside.
        std::fs::remove_file(std::env::current_exe().unwrap()).unwrap();
    }
    let folder = std::env::current_dir().unwrap();
    let answers = run(command(
        Path::new(config["probe"].as_str().unwrap()),
        config["child"].as_str().unwrap_or(PROBE),
        &folder,
    ));
    println!("BOOTSTRAP_REPLIES={}", json!(answers));
}

/// The socket peer's parent is this process, which no longer lets its own
/// account trace it, so the kernel refuses its image path to the daemon:
/// what a login's process under a root sshd is to the account it logged in.
#[cfg(target_os = "linux")]
fn unnamed_process() {
    // SAFETY: `prctl(PR_SET_DUMPABLE, 0)` changes only this process's own
    // flag and takes no pointer.
    assert_eq!(unsafe { libc::prctl(libc::PR_SET_DUMPABLE, 0) }, 0);
    let config = configuration();
    let folder = std::env::current_dir().unwrap();
    let answers = run(command(
        Path::new(config["probe"].as_str().unwrap()),
        PROBE,
        &folder,
    ));
    println!("BOOTSTRAP_REPLIES={}", json!(answers));
}

fn requests() -> Value {
    let base = json!({"nonce": "12345678901234567890123456789012", "pane_id": ""});
    let asks = [
        (
            "access",
            json!({"ask":"request", "name":"guest-probe", "key":"a".repeat(64)}),
        ),
        ("device", json!({"verb":"share", "held":false})),
        ("phone", json!({"verb":"pair"})),
        ("device", json!({"verb":"carry"})),
    ];
    json!(asks.map(|(field, ask)| {
        let mut request = base.clone();
        request[field] = ask;
        request
    }))
}

// Four runtime threads keep the real daemon responsive while the test
// waits for the deliberately synchronous local socket subprocesses.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_guest_holder_child_cannot_become_the_host_operator_without_hints() {
    match std::env::var(ROLE) {
        Ok(role) if role == PROBE => return socket_probe(),
        Ok(role) if role == HOLDER => return holder_process(),
        #[cfg(target_os = "linux")]
        Ok(role) if role == UNNAMED => return unnamed_process(),
        Ok(role) => panic!("unknown bootstrap test role: {role}"),
        Err(std::env::VarError::NotPresent) => {}
        Err(error) => panic!("unreadable bootstrap test role: {error}"),
    }
    let root = crate::fixtures::fixture_root("bootstrap-origin-");
    let env = super::host_env(root.path());
    let state = env.state_dir.clone();
    let home = env.home.clone();
    let running = hided::start_daemon(env).await.unwrap();
    let probe = std::env::current_exe().unwrap();
    let socket = hided::pane_auth::bootstrap_socket_path(&state).unwrap();
    let work = root.path().join("probe");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::write(
        work.join("probe.json"),
        json!({"socket":socket, "probe":probe, "requests":requests()}).to_string(),
    )
    .unwrap();

    let key = hide_peer::protocol::PeerKey::from_bytes([73; 32]);
    let guest = hide_kit::layout::guest_install_root(&state, &key);
    let helper = hide_kit::layout::holders_root(&home);
    // Revoking a guest does not revoke the panes the operator left running.
    let removed = hide_kit::layout::guest_removed(&state, &key);
    std::fs::create_dir_all(removed.parent().unwrap()).unwrap();
    std::fs::write(removed, "after_last_pane").unwrap();
    // The guest has not reported its first pane on the tap. Neither a
    // current node nor its internal records are required for this proof.
    for place in [guest.join("holders/v1-fixture"), helper.join("v1-fixture")] {
        std::fs::create_dir_all(&place).unwrap();
        // The running image is trusted, never its mutable name. Start it
        // through an alias on Unix as well as the /tmp fixture alias.
        let holder = place.join(if cfg!(windows) {
            "renamed.exe"
        } else {
            "renamed"
        });
        std::fs::copy(&probe, &holder).unwrap();
        #[cfg(unix)]
        let holder = {
            let alias = place.join("alias");
            std::os::unix::fs::symlink(&holder, &alias).unwrap();
            alias
        };
        for (request, expected) in requests().as_array().unwrap().iter().zip([
            "access_from_pane",
            "device_from_pane",
            "phone_pair_from_pane",
            "device_from_pane",
        ]) {
            let mut config = configuration_at(&work);
            config["requests"] = json!([request]);
            std::fs::write(work.join("probe.json"), config.to_string()).unwrap();
            let answers = run(command(&holder, HOLDER, &work));
            assert_eq!(answers.len(), 1);
            let answer = &answers[0];
            assert_eq!(answer["ok"], false, "{answer}");
            assert_eq!(answer["reason"], expected, "{answer}");
            assert!(answer["next_action"].is_string(), "{answer}");
        }
    }
    // A failed host ownership read must not promote a caller to Operator
    // in any of the four entry points, including the carrier fast path.
    let record = hide_kit::layout::pane_record(&state, "unreadable");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(&record, "not a holder record").unwrap();
    let mut config = configuration_at(&work);
    config["requests"] = requests();
    std::fs::write(work.join("probe.json"), config.to_string()).unwrap();
    for answer in run(command(&probe, PROBE, &work)) {
        assert_eq!(answer["ok"], false, "{answer}");
        assert_eq!(answer["reason"], "caller_origin_unavailable", "{answer}");
        assert!(answer["next_action"].is_string(), "{answer}");
    }
    std::fs::remove_file(record).unwrap();
    #[cfg(unix)]
    {
        let unreadable = work.join("unreadable-image");
        std::fs::copy(&probe, &unreadable).unwrap();
        let mut config = configuration_at(&work);
        config["delete_image"] = json!(true);
        std::fs::write(work.join("probe.json"), config.to_string()).unwrap();
        for answer in run(command(&unreadable, HOLDER, &work)) {
            assert_eq!(answer["ok"], false, "{answer}");
            assert_eq!(answer["reason"], "caller_origin_unavailable", "{answer}");
        }
    }
    // A genuine outside terminal is still allowed to request a bot. Its
    // process is not descended from either of this host's holder roots.
    let mut config = configuration_at(&work);
    config["requests"] = json!([requests()[0].clone()]);
    std::fs::write(work.join("probe.json"), config.to_string()).unwrap();
    let outside = run(command(&probe, PROBE, &work));
    assert_eq!(outside[0]["ok"], true, "{outside:?}");
    assert_eq!(outside[0]["access"]["state"], "pending", "{outside:?}");
    // A command that arrives over SSH descends from the process a root sshd
    // keeps for its login, which is this account's and whose image Linux
    // names to no process of the account. It is no holder, so the terminal
    // above it is still an outside one; under a holder it is still a pane's.
    #[cfg(target_os = "linux")]
    {
        let mut login = requests()[0].clone();
        login["access"]["name"] = json!("login-probe");
        login["access"]["key"] = json!("b".repeat(64));
        let mut config = configuration_at(&work);
        config["requests"] = json!([login]);
        std::fs::write(work.join("probe.json"), config.to_string()).unwrap();
        let outside = run(command(&probe, UNNAMED, &work));
        assert_eq!(outside[0]["ok"], true, "{outside:?}");
        assert_eq!(outside[0]["access"]["state"], "pending", "{outside:?}");
        config["child"] = json!(UNNAMED);
        std::fs::write(work.join("probe.json"), config.to_string()).unwrap();
        let holder = helper.join("v1-fixture/alias");
        let inside = run(command(&holder, HOLDER, &work));
        assert_eq!(inside[0]["ok"], false, "{inside:?}");
        assert_eq!(inside[0]["reason"], "access_from_pane", "{inside:?}");
    }
    running.stop();
}

fn configuration_at(folder: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(folder.join("probe.json")).unwrap()).unwrap()
}
