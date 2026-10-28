//! A WSL distribution of this Windows PC as a device, end to end (PRD
//! wsl-device D-01, D-07, B7, B8, B22-B28): the candidate's Linux helper,
//! command and hooks and the pinned Linux Herdr, installed through
//! `wsl.exe` into a real WSL 2 distribution and reached over the one pipe
//! the link owns. The `windows wsl` lane sets the distribution up fresh and
//! runs this alone; it owns the distribution, so the test may stop it and
//! change its default user.
//!
//! `HIDE_E2E_WSL_DISTRO` names the distribution and `HIDE_E2E_WSL_PACKAGES`
//! the folder holding `hided-linux-x86_64`, `hide-linux-x86_64`,
//! `hide-agent-hooks-linux-x86_64` and `herdr-linux-x86_64`.
#![cfg(windows)]

use std::io::BufRead;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use hide_node::ssh::host::wsl::WslAccess;
use hide_node_link::device::{
    DeviceConnector, DialedTransport, EstablishError, Established, HOST_CONSENT_CONTRACT,
    HostConsent, Wake,
};
use hide_node_link::protocol::{Call, RootOpened, RootRef};
use hide_node_link::terminal::{GridSize, KeyTarget, TerminalControl, TerminalReport};
use hide_node_link::{NodeLink, call_as};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(120);
const USER: &str = "hidetest";
/// Set for the owner process the crash step starts: the distribution it
/// connects to.
const OWNER: &str = "HIDE_E2E_WSL_OWNER";

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
        wake,
        Box::new(move |reason| {
            let _ = heard.send(reason);
        }),
    );
    (result, closed)
}

fn connector(sink: Arc<Sink>) -> hide_node::ssh::Connector {
    let packages = PathBuf::from(
        std::env::var_os("HIDE_E2E_WSL_PACKAGES").expect("set HIDE_E2E_WSL_PACKAGES"),
    );
    hide_node::ssh::Connector::new(Some(packages))
        .with_helper_root("~/.hide/host-helper".to_owned())
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
        host_key_sha256: format!("wsl:{}", distribution_id(distro)),
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
    assert_eq!(first.identity.host_key_sha256, format!("wsl:{id}"));
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
