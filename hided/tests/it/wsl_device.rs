//! A WSL distribution of this Windows PC as a device, end to end (PRD
//! wsl-device D-01, D-07, B7, B8, B22-B28): the candidate's Linux helper,
//! command and hooks and the pinned Linux Herdr, installed through
//! `wsl.exe` into a real WSL 2 distribution and reached over the one pipe
//! the link owns. The `windows wsl` lane sets the distribution up fresh and
//! runs this alone; it owns the distribution, so the test may stop it and
//! change its default user.
//!
//! The second walk is the operator's journey over the same link (PRD
//! wsl-device B8, B9, B12, B26): a project on the Windows drive added with
//! the operator's confirmation, the page its server answers inside WSL,
//! Hide AI answered by a CLI inside WSL, and all three again after a
//! reconnection. The core's own first run that sends these calls is the
//! operator's check on a real PC.
//!
//! `HIDE_E2E_WSL_DISTRO` names the distribution and `HIDE_E2E_WSL_PACKAGES`
//! the folder holding `hided-linux-x86_64`, `hide-linux-x86_64`,
//! `hide-agent-hooks-linux-x86_64` and `herdr-linux-x86_64`.
#![cfg(windows)]

use std::io::{BufRead, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use hide_node::ssh::host::wsl::WslAccess;
use hide_node_link::ai::{AiError, AiRequest, AiResponse, BackendSpec, Logged, ProviderId};
use hide_node_link::device::{
    Attempt, DeviceConnector, DialedTransport, EstablishError, Established, HOST_CONSENT_CONTRACT,
    HostConsent, Wake,
};
use hide_node_link::protocol::{Call, RootOpened, RootRef};
use hide_node_link::register::Registrable;
use hide_node_link::terminal::{GridSize, KeyTarget, TerminalControl, TerminalReport};
use hide_node_link::{ErrorCode, LinkError, NodeLink, call_as, call_as_with_progress};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(120);
const USER: &str = "hidetest";
/// Set for the owner process the crash step starts: the distribution it
/// connects to.
const OWNER: &str = "HIDE_E2E_WSL_OWNER";
/// The journey's folder on the Windows drive, and the same folder as WSL
/// reaches it.
const SCENARIO_WINDOWS: &str = r"C:\hide-wsl-scenario";
const SCENARIO_WSL: &str = "/mnt/c/hide-wsl-scenario";
/// The project's page, served inside WSL on this port.
const PAGE: &str = "hide-wsl-page";
const PAGE_BASE64: &str = "aGlkZS13c2wtcGFnZQ==";
const PAGE_PORT: u16 = 47613;
/// A stand-in for the Pi CLI: it answers where it ran and as whom.
const PI: &str = r#"#!/bin/sh
cat >/dev/null
printf '{"summary":"%s %s"}\n' "$(uname -s)" "$(id -un)"
"#;

#[derive(Default)]
struct Sink {
    output: Mutex<Vec<u8>>,
    reports: Mutex<Vec<TerminalReport>>,
}

impl hide_node::terminal::device::DeviceSink for Sink {
    fn output(&self, _: &str, _: &str, bytes: &[u8], _: bool) {
        self.output.lock().unwrap().extend_from_slice(bytes);
    }
    fn report(&self, _: &str, report: TerminalReport) {
        self.reports.lock().unwrap().push(report);
    }
}

fn wait_for<T>(what: &str, mut ready: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(value) = ready() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// `wsl.exe -d <distro> -u root -e sh -c <script>`, which must succeed;
/// answers its standard output.
fn as_root(distro: &str, script: &str) -> String {
    let output = Command::new("wsl.exe")
        .args(["-d", distro, "-u", "root", "-e", "sh", "-c", script])
        .output()
        .expect("wsl.exe runs");
    assert!(
        output.status.success(),
        "{script}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Sets a value of the distribution's own registry entry, as WSL's first
/// run does when it marks the setup done.
fn set_registry(id: &str, name: &str, value: u32) {
    let script = format!(
        "Set-ItemProperty -Path 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Lxss\\{{{id}}}' -Name {name} -Value {value} -Type DWord"
    );
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .status()
        .expect("powershell runs");
    assert!(status.success(), "{script}");
}

/// Makes `user` the distribution's default user where WSL reads it when the
/// distribution starts (`/etc/wsl.conf`), then stops the distribution so its
/// next start reads it. A `DefaultUid` written to the registry while the
/// distribution ran was not used (GitHub's Windows runner, 2026-10-28).
fn set_default_user(distro: &str, user: &str) {
    as_root(
        distro,
        &format!(
            "touch /etc/wsl.conf; if grep -q '^\\[user\\]' /etc/wsl.conf; then sed -i 's/^default=.*/default={user}/' /etc/wsl.conf; else printf '\\n[user]\\ndefault={user}\\n' >> /etc/wsl.conf; fi"
        ),
    );
    stop(distro);
}

fn stop(distro: &str) {
    // A distribution that is already stopped has nothing to end.
    let _ = Command::new("wsl.exe")
        .args(["--terminate", distro])
        .status()
        .expect("wsl.exe runs");
    wait_for("the distribution to stop", || {
        (!running(distro)).then_some(())
    });
}

fn running(distro: &str) -> bool {
    hide_platform::wsl::running_from(
        &hide_platform::wsl::program().unwrap(),
        Duration::from_secs(30),
    )
    .unwrap()
    .contains(distro)
}

fn consent(identity: Option<hide_node_link::device::HostIdentity>) -> HostConsent {
    HostConsent {
        contract: HOST_CONSENT_CONTRACT,
        helper_root: "~/.hide/host-helper".to_owned(),
        cli_dir: Some("~/.local/bin".to_owned()),
        granted_at_unix_ms: 1,
        identity,
    }
}

fn establish(
    transport: &dyn DialedTransport,
    consent: &HostConsent,
    wake: Wake,
) -> (Result<Established, EstablishError>, mpsc::Receiver<String>) {
    let (heard, closed) = mpsc::channel();
    let result = transport.establish(
        consent,
        &[],
        Attempt::unattended(
            wake,
            Box::new(move |reason| {
                let _ = heard.send(reason);
            }),
        ),
    );
    (result, closed)
}

fn connector(sink: Arc<Sink>) -> hide_node::ssh::Connector {
    let packages = PathBuf::from(
        std::env::var_os("HIDE_E2E_WSL_PACKAGES").expect("set HIDE_E2E_WSL_PACKAGES"),
    );
    hide_node::ssh::Connector::new(Some(packages))
        .with_terminals(sink)
        .with_pane_events(Arc::default())
        .with_wsl(WslAccess::of_this_pc().expect("this runner has WSL"))
}

fn distribution_id(distro: &str) -> String {
    hide_platform::wsl::distributions()
        .unwrap()
        .into_iter()
        .find(|found| found.name == distro)
        .unwrap()
        .id
}

/// The consent the first connection bound to the lane's account.
fn bound(distro: &str) -> HostConsent {
    consent(Some(hide_node_link::device::HostIdentity {
        user: USER.to_owned(),
        hostname: distro.to_owned(),
        port: 0,
        machine_id: Some(format!("wsl:{}", distribution_id(distro))),
        host_key_sha256: String::new(),
    }))
}

fn helper_running(distro: &str) -> bool {
    as_root(
        distro,
        "pgrep -f '[n]ode serve' >/dev/null && echo alive || echo gone",
    )
    .contains("alive")
}

fn herdr(transport: &dyn DialedTransport, method: &str, params: Value) -> Value {
    hide_herdr_client::request_with_connector(
        &*transport.herdr_api_connector(),
        method,
        params,
        Duration::from_secs(20),
    )
    .unwrap_or_else(|error| panic!("{method} through the link: {error}"))
}

/// Every pane id a Herdr answer names, in order.
fn panes(value: &Value) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![value];
    while let Some(value) = stack.pop() {
        match value {
            Value::Object(map) => {
                if let Some(pane) = map.get("pane_id").and_then(Value::as_str) {
                    found.push(pane.to_owned());
                }
                stack.extend(map.values());
            }
            Value::Array(values) => stack.extend(values),
            _ => {}
        }
    }
    found.sort();
    found.dedup();
    found
}

fn pane_of(value: &Value) -> Option<&str> {
    match value {
        Value::Object(map) => map
            .get("pane_id")
            .and_then(Value::as_str)
            .or_else(|| map.values().find_map(pane_of)),
        Value::Array(values) => values.iter().find_map(pane_of),
        _ => None,
    }
}

#[test]
#[ignore = "a real WSL 2 distribution of the runner's own; the windows wsl lane sets it up"]
fn a_wsl_distribution_connects_works_and_ends_with_its_link() {
    let distro = std::env::var("HIDE_E2E_WSL_DISTRO").expect("set HIDE_E2E_WSL_DISTRO");
    let sink = Arc::new(Sink::default());
    let connector = connector(sink.clone());

    // D-16: which WSL the runner gives, read without starting anything.
    let listed = connector.wsl_distributions().expect("the registry reads");
    let row = listed
        .iter()
        .find(|row| row.name == distro)
        .expect("the lane's distribution is listed");
    println!(
        "wsl_version={} setup_unfinished={}",
        row.wsl_version, row.setup_unfinished
    );
    assert_eq!(row.wsl_version, 2, "the lane runs WSL 2");
    let id = distribution_id(&distro);
    let transport = connector.wsl_transport("wsl-test", &distro).unwrap();

    // B28: a distribution whose first-run setup has not run is refused
    // before anything is written into it.
    if row.setup_unfinished {
        let (refused, _) = establish(&*transport, &consent(None), Wake::Allowed);
        assert!(
            matches!(&refused, Err(EstablishError::Unsupported(reason)) if reason.contains("Finish setting up")),
            "{:?}",
            refused.err()
        );
    }
    // What the first run would have done: the account's user, made the
    // default, and the setup marked done. Each try starts from a fresh
    // account, so a retry finds nothing an earlier try left there.
    as_root(
        &distro,
        &format!(
            "pkill -KILL -u {USER} 2>/dev/null; userdel -r -f {USER} 2>/dev/null; useradd -m -s /bin/bash {USER}"
        ),
    );
    set_registry(&id, "RunOOBE", 0);
    set_default_user(&distro, USER);
    let windows_settings = PathBuf::from(std::env::var_os("USERPROFILE").unwrap())
        .join(".claude")
        .join("settings.json");
    let windows_settings_before = std::fs::read(&windows_settings).ok();

    // B3, B4: an attempt with leave starts the stopped distribution, then
    // consent, install, start; Hello names Linux.
    assert!(!running(&distro));
    let (first, closed) = establish(&*transport, &consent(None), Wake::Allowed);
    let first = first.unwrap_or_else(|error| panic!("first connection: {error}"));
    assert!(first.installed, "the first connection installs the helper");
    assert_eq!(first.node.hello.os, "linux");
    assert_eq!(first.identity.user, USER);
    assert_eq!(first.identity.machine_id, Some(format!("wsl:{id}")));
    println!("upload={:?}", first.upload);
    // A fresh distribution has no Herdr answering, so the kit ran before
    // the terminals and started the pinned one inside WSL.
    let kit = first
        .kit
        .as_ref()
        .expect("the fresh distribution's kit ran");
    assert!(kit.is_ok(), "{:?}", kit.as_ref().err());
    let link = Arc::clone(&first.node.host);

    // The account's Herdr runs inside WSL and answers through the link.
    herdr(&*transport, "session.snapshot", json!({}));

    // B8: files and Git of a project by its Linux path.
    as_root(
        &distro,
        &format!(
            "su - {USER} -c 'mkdir -p project && cd project && git init -q && printf \"hello\\n\" > a.txt && git add a.txt && git -c user.name=t -c user.email=t@example.invalid commit -qm init && printf \"more\\n\" >> a.txt'"
        ),
    );
    let home = first
        .node
        .hello
        .home
        .clone()
        .expect("the helper names its home");
    let project = format!("{home}/project");
    let opened: RootOpened = call_as(
        &*link,
        Call::RootOpen {
            root: project.clone(),
        },
        TIMEOUT,
    )
    .unwrap();
    let root = RootRef {
        path: project.clone(),
        identity: opened.identity,
    };
    let bytes: Value = call_as(
        &*link,
        Call::Bytes {
            root: root.clone(),
            path: "a.txt".to_owned(),
            offset: 0,
            length: 64,
        },
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(bytes["data"], "aGVsbG8KbW9yZQo=", "{bytes}");
    // A file saved through the link is on the distribution's own disk.
    let document: hide_node_link::document::Document = call_as(
        &*link,
        Call::OpenDocument {
            root: root.clone(),
            path: "a.txt".to_owned(),
        },
        TIMEOUT,
    )
    .unwrap();
    let revision = document.revision.expect("an editable revision");
    let saved: hide_node_link::save::Saved = call_as(
        &*link,
        Call::Save {
            root: root.clone(),
            path: "a.txt".to_owned(),
            contents: "saved through the link\n".to_owned(),
            expected_revision: revision.clone(),
        },
        TIMEOUT,
    )
    .unwrap();
    assert_ne!(saved.revision, revision);
    assert_eq!(
        as_root(&distro, &format!("cat '{project}/a.txt'")),
        "saved through the link\n"
    );
    let changes: Value = call_as(
        &*link,
        Call::Changes {
            root,
            scope: String::new(),
            selected: Some("a.txt".to_owned()),
            committed: false,
            base: None,
            diffs: Vec::new(),
        },
        TIMEOUT,
    )
    .unwrap();
    assert!(changes.to_string().contains("a.txt"), "{changes}");

    // B7: a pane's terminal inside WSL, its input run once.
    let created = herdr(
        &*transport,
        "workspace.create",
        json!({"cwd": project, "label": "wsl"}),
    );
    let pane = pane_of(&created)
        .expect("the new workspace's pane")
        .to_owned();
    let terminals = first
        .node
        .terminals
        .as_ref()
        .expect("terminals ride the link");
    terminals.control(TerminalControl::Attach {
        pane: pane.clone(),
        size: Some(GridSize { rows: 24, cols: 80 }),
        manual: false,
    });
    wait_for("the pane's terminal", || {
        sink.reports
            .lock()
            .unwrap()
            .iter()
            .any(|report| matches!(report, TerminalReport::State { pane: reported, state } if reported == &pane && state.state == "controlling"))
            .then_some(())
    });
    terminals.key(
        KeyTarget::Pane(pane.clone()),
        b"printf 'wsl_%s\\n' answered\r".to_vec(),
        1,
    );
    wait_for("the command's output", || {
        String::from_utf8_lossy(&sink.output.lock().unwrap())
            .contains("wsl_answered")
            .then_some(())
    });

    let panes_before = herdr(&*transport, "session.snapshot", json!({}));
    // B27: the link's end ends its helper inside WSL; Herdr and the pane stay.
    link.close("the test ends the link");
    let reason = closed
        .recv_timeout(TIMEOUT)
        .expect("the link's end is heard");
    println!("link ended: {reason}");
    wait_for("the helper to end inside WSL", || {
        (!helper_running(&distro)).then_some(())
    });
    assert!(
        as_root(
            &distro,
            "pgrep -x herdr >/dev/null && echo alive || echo gone"
        )
        .contains("alive"),
        "Herdr outlives the link"
    );
    drop(first);

    // B26: the next connection reuses the install whole and finds the pane
    // it left; B23: a running distribution reconnects without leave to wake.
    let bound = bound(&distro);
    let (again, _closed) = establish(&*transport, &bound, Wake::NotAllowed);
    let again = again.unwrap_or_else(|error| panic!("reconnection: {error}"));
    assert!(!again.installed);
    assert_eq!(again.upload.sent, 0, "{:?}", again.upload);
    let snapshot = herdr(&*transport, "session.snapshot", json!({}));
    assert!(
        snapshot.to_string().contains(&pane),
        "the pane outlived the link"
    );
    assert_eq!(
        panes(&snapshot),
        panes(&panes_before),
        "the reconnection made no pane"
    );
    // B26: the reconnection replays no input: the line typed once over the
    // first link ran once, and a line typed over this link runs once.
    let pane_text = || {
        herdr(
            &*transport,
            "pane.read",
            json!({"pane_id": pane, "source": "recent", "lines": 200}),
        )["read"]["text"]
            .as_str()
            .expect("the pane's text")
            .to_owned()
    };
    assert_eq!(
        pane_text().matches("wsl_answered").count(),
        1,
        "{}",
        pane_text()
    );
    sink.reports.lock().unwrap().clear();
    let terminals = again
        .node
        .terminals
        .as_ref()
        .expect("terminals ride the new link");
    terminals.control(TerminalControl::Attach {
        pane: pane.clone(),
        size: Some(GridSize { rows: 24, cols: 80 }),
        manual: false,
    });
    wait_for("the pane's terminal over the new link", || {
        sink.reports
            .lock()
            .unwrap()
            .iter()
            .any(|report| matches!(report, TerminalReport::State { pane: reported, state } if reported == &pane && state.state == "controlling"))
            .then_some(())
    });
    terminals.key(
        KeyTarget::Pane(pane.clone()),
        b"printf 'wsl_%s\\n' again\r".to_vec(),
        2,
    );
    wait_for("the new link's command output", || {
        pane_text().contains("wsl_again").then_some(())
    });
    let text = pane_text();
    assert_eq!(
        (
            text.matches("wsl_answered").count(),
            text.matches("wsl_again").count()
        ),
        (1, 1),
        "{text}"
    );
    again.node.host.close("the test ends the second link");
    drop(again);
    wait_for("the second helper to end inside WSL", || {
        (!helper_running(&distro)).then_some(())
    });

    // B27: a process that owns a link and dies with no destructor run, as
    // a crashed hided does, still ends the helper inside WSL: its
    // `wsl.exe` sits in a job object that ends with the process, and the
    // helper reads the end of its input. The same Herdr keeps running.
    let herdr_pids = as_root(&distro, "pgrep -x herdr");
    let mut owner = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "wsl_device::a_link_owner_that_is_killed",
            "--ignored",
            "--nocapture",
        ])
        .env(OWNER, &distro)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("the owner process starts");
    let (connected, owner_connected) = mpsc::channel();
    let lines = std::io::BufReader::new(owner.stdout.take().unwrap());
    std::thread::spawn(move || {
        for line in lines.lines().map_while(Result::ok) {
            if line.contains("owner connected") {
                let _ = connected.send(());
            }
        }
    });
    owner_connected
        .recv_timeout(TIMEOUT)
        .expect("the owner process connects");
    assert!(helper_running(&distro), "the owner's helper runs");
    owner.kill().expect("the owner process is ended");
    let _ = owner.wait();
    wait_for("the killed owner's helper to end inside WSL", || {
        (!helper_running(&distro)).then_some(())
    });
    assert_eq!(
        as_root(&distro, "pgrep -x herdr"),
        herdr_pids,
        "the same Herdr outlives its owner's crash"
    );
    println!("killed owner: helper gone, herdr {}", herdr_pids.trim());

    // B30: the Windows account's agent settings are not the WSL device's.
    assert_eq!(
        std::fs::read(&windows_settings).ok(),
        windows_settings_before
    );

    // B28: another default user is another identity, asked again.
    as_root(&distro, "id -u other >/dev/null 2>&1 || useradd -m other");
    set_default_user(&distro, "other");
    let (changed, _) = establish(&*transport, &bound, Wake::Allowed);
    assert!(
        matches!(changed, Err(EstablishError::IdentityChanged { .. })),
        "{:?}",
        changed.err()
    );
    set_default_user(&distro, USER);

    // B22, B24: a stopped distribution stays stopped for an attempt without
    // leave; B3, B23: one with leave starts it and connects.
    assert!(!running(&distro));
    let (stopped, _) = establish(&*transport, &bound, Wake::NotAllowed);
    assert!(
        matches!(stopped, Err(EstablishError::Stopped(_))),
        "{:?}",
        stopped.err()
    );
    assert!(!running(&distro), "an attempt without leave started it");
    let (woken, _) = establish(&*transport, &bound, Wake::Allowed);
    let woken = woken.unwrap_or_else(|error| panic!("woken connection: {error}"));
    assert!(running(&distro));
    woken.node.host.close("the test is done");
}

/// The journey's own hang guard: past the slowest green run (42 to 65 s on
/// GitHub's Windows runner) and short of the lane's 120 s, which ends the
/// test saying nothing of where it stood (with-hide/hide-private#339).
const JOURNEY_GUARD: Duration = Duration::from_secs(105);
/// How long the distribution's process list may take once the guard fired.
const PROBE_GUARD: Duration = Duration::from_secs(10);
/// The most lines of the journey's record kept; the oldest go first.
const RECORD_CAP: usize = 256;

/// What the journey did, kept in memory and printed only when it fails: its
/// steps and the Hide AI request's progress, each with its time since the
/// start. A failed assertion prints the record as the test unwinds; a
/// journey still running at [`JOURNEY_GUARD`] fails there, printing the
/// record and what the distribution runs, read by an owned `ps` bounded by
/// [`PROBE_GUARD`], and ends the test process. A journey that ends in time
/// prints nothing.
struct Journey {
    started: Instant,
    record: Arc<Mutex<std::collections::VecDeque<String>>>,
    done: Option<mpsc::Sender<()>>,
    watch: Option<std::thread::JoinHandle<()>>,
}

impl Journey {
    fn start(distro: &str) -> Self {
        let started = Instant::now();
        let record = Arc::new(Mutex::new(std::collections::VecDeque::new()));
        let (done, ended) = mpsc::channel::<()>();
        let kept = Arc::clone(&record);
        let distro = distro.to_owned();
        let watch = std::thread::spawn(move || {
            if ended.recv_timeout(JOURNEY_GUARD) != Err(mpsc::RecvTimeoutError::Timeout) {
                return;
            }
            eprintln!("the journey still ran at {JOURNEY_GUARD:?}");
            print_record(&kept);
            eprintln!("processes in {distro}: {}", processes(&distro));
            std::process::exit(101);
        });
        Self {
            started,
            record,
            done: Some(done),
            watch: Some(watch),
        }
    }

    fn step(&self, name: &str) {
        self.note(&format!("step {name}"));
    }

    fn note(&self, what: &str) {
        let line = format!("{what} at {} ms", self.started.elapsed().as_millis());
        let mut record = self.record.lock().unwrap();
        if record.len() == RECORD_CAP {
            record.pop_front();
        }
        record.push_back(line);
    }
}

impl Drop for Journey {
    fn drop(&mut self) {
        // Ends the watch's wait at once; a watch past its guard is already
        // ending the process within PROBE_GUARD.
        self.done.take();
        if let Some(watch) = self.watch.take() {
            let _ = watch.join();
        }
        if std::thread::panicking() {
            print_record(&self.record);
        }
    }
}

fn print_record(record: &Mutex<std::collections::VecDeque<String>>) {
    let record = record
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    eprintln!("the journey's record:");
    for line in record.iter() {
        eprintln!("  {line}");
    }
}

/// What the distribution runs, read by a `ps` this test owns and ends
/// within [`PROBE_GUARD`].
fn processes(distro: &str) -> String {
    let mut command = Command::new("wsl.exe");
    command
        .args(["-d", distro, "-u", "root", "-e", "ps", "-eo"])
        .arg("pid,ppid,stat,etime,args")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match hide_platform::process::OwnedChild::spawn(&mut command) {
        Ok(mut child) => match child.capture_until(Instant::now() + PROBE_GUARD, 64 * 1024) {
            Ok(output) => format!(
                "\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ),
            Err(error) => format!("unread: {error}"),
        },
        Err(error) => format!("unread: {error}"),
    }
}

#[test]
#[ignore = "a real WSL 2 distribution of the runner's own; the windows wsl lane sets it up"]
fn a_wsl_device_keeps_a_windows_drive_project_its_page_and_hide_ai_across_a_reconnect() {
    let distro = std::env::var("HIDE_E2E_WSL_DISTRO").expect("set HIDE_E2E_WSL_DISTRO");
    let journey = Journey::start(&distro);
    let sink = Arc::new(Sink::default());
    let connector = connector(sink.clone());
    let transport = connector.wsl_transport("wsl-journey", &distro).unwrap();
    // The account the first-run setup would make, unless the other walk
    // already made it.
    as_root(
        &distro,
        &format!("id -u {USER} >/dev/null 2>&1 || useradd -m -s /bin/bash {USER}"),
    );
    set_registry(&distribution_id(&distro), "RunOOBE", 0);
    set_default_user(&distro, USER);

    // A project on the Windows drive holding the page, and the stand-in CLI
    // beside it.
    let windows = Path::new(SCENARIO_WINDOWS);
    let _ = std::fs::remove_dir_all(windows);
    std::fs::create_dir_all(windows.join("app")).unwrap();
    std::fs::write(windows.join("app").join("index.html"), PAGE).unwrap();
    std::fs::write(windows.join("pi"), PI).unwrap();
    let project = format!("{SCENARIO_WSL}/app");

    // B2: consent, install, connect.
    journey.step("first connection");
    let (first, closed) = establish(&*transport, &consent(None), Wake::Allowed);
    let first = first.unwrap_or_else(|error| panic!("connection: {error}"));
    assert_eq!(first.node.hello.os, "linux");
    let home = first
        .node
        .hello
        .home
        .clone()
        .expect("the helper names its home");
    as_root(
        &distro,
        &format!(
            "su - {USER} -c 'mkdir -p .local/bin' && install -m 755 -o {USER} -g {USER} {SCENARIO_WSL}/pi {home}/.local/bin/pi"
        ),
    );
    let link = Arc::clone(&first.node.host);

    journey.step("the Windows drive project");
    // B8, D-02: a folder outside the distribution's home is a project only
    // with the operator's confirmation of it, and its files read through
    // the link.
    let unconfirmed = call_as::<Registrable>(
        &*link,
        Call::Registrable {
            path: project.clone(),
            confirmed_outside_home: None,
        },
        TIMEOUT,
    );
    assert!(
        matches!(&unconfirmed, Err(LinkError::Refused(refused)) if matches!(refused.code, ErrorCode::NeedsConfirmation)),
        "{unconfirmed:?}"
    );
    assert_eq!(
        registrable(&*link, &project),
        Registrable {
            root: project.clone(),
            is_git: false
        }
    );
    let opened: RootOpened = call_as(
        &*link,
        Call::RootOpen {
            root: project.clone(),
        },
        TIMEOUT,
    )
    .unwrap();
    let bytes: Value = call_as(
        &*link,
        Call::Bytes {
            root: RootRef {
                path: project.clone(),
                identity: opened.identity,
            },
            path: "index.html".to_owned(),
            offset: 0,
            length: 64,
        },
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(bytes["data"], PAGE_BASE64, "{bytes}");

    // B9: the project's own server, started in its pane inside WSL, is
    // previewed through a forward on the link.
    journey.step("the page server in its pane");
    let created = herdr(
        &*transport,
        "workspace.create",
        json!({"cwd": project, "label": "journey"}),
    );
    let pane = pane_of(&created)
        .expect("the new workspace's pane")
        .to_owned();
    let terminals = first
        .node
        .terminals
        .as_ref()
        .expect("terminals ride the link");
    terminals.control(TerminalControl::Attach {
        pane: pane.clone(),
        size: Some(GridSize { rows: 24, cols: 80 }),
        manual: false,
    });
    wait_for("the pane's terminal", || controlling(&sink, &pane));
    terminals.key(
        KeyTarget::Pane(pane.clone()),
        format!("python3 -m http.server {PAGE_PORT} --bind 127.0.0.1\r").into_bytes(),
        1,
    );
    let runtime = tokio::runtime::Runtime::new().unwrap();
    journey.step("the page through the link");
    page_over(&runtime, &link);

    // B12: Hide AI answered by a CLI inside WSL, as the distribution's user.
    let answer = json!({"summary": format!("Linux {USER}")});
    journey.step("Hide AI");
    assert_eq!(ask_ai(&*link, 1, &journey), answer);

    // B26: after a reconnection the same server still answers in its pane,
    // the project is still registrable and Hide AI answers again.
    journey.step("the reconnection");
    link.close("the journey reconnects");
    closed
        .recv_timeout(TIMEOUT)
        .expect("the link's end is heard");
    wait_for("the helper to end inside WSL", || {
        (!helper_running(&distro)).then_some(())
    });
    drop(link);
    drop(first);
    let (again, _closed) = establish(&*transport, &bound(&distro), Wake::NotAllowed);
    let again = again.unwrap_or_else(|error| panic!("reconnection: {error}"));
    assert!(!again.installed);
    let link = Arc::clone(&again.node.host);
    assert_eq!(registrable(&*link, &project).root, project);
    journey.step("the page again");
    page_over(&runtime, &link);
    journey.step("Hide AI again");
    assert_eq!(ask_ai(&*link, 2, &journey), answer);
    link.close("the journey is done");
}

/// `path` judged with the operator's confirmation of it.
fn registrable(link: &dyn NodeLink, path: &str) -> Registrable {
    call_as(
        link,
        Call::Registrable {
            path: path.to_owned(),
            confirmed_outside_home: Some(path.to_owned()),
        },
        TIMEOUT,
    )
    .unwrap_or_else(|error| panic!("{path} with the operator's confirmation: {error}"))
}

fn controlling(sink: &Sink, pane: &str) -> Option<()> {
    sink.reports
        .lock()
        .unwrap()
        .iter()
        .any(|report| matches!(report, TerminalReport::State { pane: reported, state } if reported == pane && state.state == "controlling"))
        .then_some(())
}

/// The journey's page through a forward on `link`, as a browser display
/// previews a device's localhost page; waits for its server to answer.
fn page_over(runtime: &tokio::runtime::Runtime, link: &Arc<dyn NodeLink>) {
    use hided::page_forward::{PageForward, PageLink};
    let link = link
        .as_any()
        .and_then(|link| link.downcast_ref::<hide_node::ssh::RemoteHost>())
        .expect("a WSL link carries streams")
        .clone();
    let forward = runtime
        .block_on(PageForward::start(
            PageLink::Device(link),
            SocketAddr::from(([127, 0, 0, 1], PAGE_PORT)),
            None,
            false,
        ))
        .unwrap();
    wait_for("the page through the link", || {
        let mut page = TcpStream::connect(forward.local_addr()).ok()?;
        page.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
        page.write_all(b"GET /index.html HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .ok()?;
        let mut answer = Vec::new();
        let _ = page.read_to_end(&mut answer);
        String::from_utf8_lossy(&answer)
            .contains(PAGE)
            .then_some(())
    });
}

/// One Hide AI request to the device's Pi backend `instance`, which runs
/// the stand-in.
fn ask_ai(link: &dyn NodeLink, instance: u64, journey: &Journey) -> Value {
    let request: AiRequest = serde_json::from_value(json!({
        "feature_id": "wsl_journey",
        "request_id": format!("wsl-journey-{instance}"),
        "subject_id": "wsl",
        "system": "Say where you run.",
        "input": "where",
        "output_schema": {"type": "object", "required": ["summary"], "properties": {"summary": {"type": "string"}}},
        "deadline": {"secs": 60, "nanos": 0},
        "schema_version": "wsl_journey.v1",
        "output_language": null,
    }))
    .expect("a Hide AI request");
    let answer: Logged<Result<AiResponse, AiError>> = call_as_with_progress::<_, Value>(
        link,
        Call::AiExecute {
            backend: BackendSpec {
                instance,
                provider: ProviderId::PI,
                model: String::new(),
            },
            request,
        },
        TIMEOUT,
        |progress: Value| {
            journey.note(&format!("Hide AI progress {progress}"));
            true
        },
    )
    .expect("the device answers Hide AI");
    journey.note("Hide AI answered");
    answer
        .value
        .unwrap_or_else(|error| panic!("the stand-in's answer: {error:?}"))
        .value
}

/// The link owner the crash step kills: it connects as the lane's account
/// and waits on its input, so nothing of its own runs at its end.
#[test]
#[ignore = "started and killed by a_wsl_distribution_connects_works_and_ends_with_its_link"]
fn a_link_owner_that_is_killed() {
    let distro = std::env::var(OWNER).expect("started by the crash step");
    let connector = connector(Arc::default());
    let transport = connector.wsl_transport("wsl-owner", &distro).unwrap();
    let (owned, _closed) = establish(&*transport, &bound(&distro), Wake::NotAllowed);
    let _owned = owned.unwrap_or_else(|error| panic!("the owner's connection: {error}"));
    println!("owner connected");
    let _ = std::io::stdin().read_line(&mut String::new());
    panic!("the owner is killed, never let go");
}
