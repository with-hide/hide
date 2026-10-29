//! The official remote terminal session fixture probe, against a real
//! device's owned fixture pane. It moved here from the core
//! (`herdr-core/src/remote.rs`) with the terminal path (PRD
//! core-host-node-terminal D-10): the session is now the device's node's,
//! driven through the node link the way the screen's router drives it, and
//! the probe keeps its fixture variables, markers and assertions.
//!
//! It connects with the SSH configuration under `HOME` and installs this
//! build's node on the device, so it runs only against an isolated device
//! account: a private sshd on its own port whose sessions get a private
//! HOME, with its own Herdr. Before anything connects, [`isolated_device`]
//! refuses any other target (see there). The fixture pane must be fresh: the
//! scroll to the top of its history has to land on the probe's own first
//! line.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hide_node::terminal::device::DeviceSink;
use hide_node_link::device::{DeviceConnector, HOST_CONSENT_CONTRACT, HostConsent};
use hide_node_link::terminal::{GridSize, KeyTarget, TerminalControl, TerminalReport};
use serde_json::json;

const SSH_OPERATION_TIMEOUT: Duration = Duration::from_secs(15);
const TAIL_MARKER: &str = "HIDE_SCROLL_080";
const HISTORY_MARKER: &str = "HIDE_SCROLL_001";

/// What the probe heard from the device, and who waits on it. Herdr sends
/// a repaint as the cells that changed, so the markers are read from the
/// screen those bytes draw, at the view's grid, not from the bytes.
struct Heard {
    pane: String,
    screen: Mutex<vt100::Parser>,
    full_frame_seen: Mutex<bool>,
    scrolled: Mutex<bool>,
    states: Mutex<Vec<String>>,
    progress: Mutex<Option<Sender<&'static str>>>,
}

impl Heard {
    fn tell(&self, what: &'static str) {
        if let Some(progress) = self.progress.lock().unwrap().as_ref() {
            let _ = progress.send(what);
        }
    }
}

impl DeviceSink for Heard {
    fn output(&self, _device: &str, pane: &str, bytes: &[u8], full: bool) {
        if pane != self.pane {
            return;
        }
        let mut screen = self.screen.lock().unwrap();
        screen.process(bytes);
        *self.full_frame_seen.lock().unwrap() |= full;
        let shown = screen.screen().contents();
        if shown.contains(TAIL_MARKER) {
            self.tell("tail");
        }
        if *self.scrolled.lock().unwrap() && shown.contains(HISTORY_MARKER) {
            self.tell("history");
        }
    }

    fn report(&self, _device: &str, report: TerminalReport) {
        if let TerminalReport::State { pane, state } = report
            && pane == self.pane
        {
            self.states.lock().unwrap().push(state.state.clone());
            match state.state.as_str() {
                "controlling" => self.tell("controlling"),
                "released" | "ended" | "idle" => self.tell("released"),
                _ => {}
            }
        }
    }
}

fn wait_for(receiver: &std::sync::mpsc::Receiver<&'static str>, what: &str, within: Duration) {
    let deadline = std::time::Instant::now() + within;
    loop {
        let left = deadline
            .checked_duration_since(std::time::Instant::now())
            .unwrap_or_else(|| panic!("the device never reported {what}"));
        match receiver.recv_timeout(left) {
            Ok(heard) if heard == what => return,
            Ok(_) => {}
            Err(_) => panic!("the device never reported {what}"),
        }
    }
}

/// The isolated device account the probe may reach.
#[derive(Debug)]
struct IsolatedDevice {
    alias: String,
    helper_root: String,
    cli_dir: String,
    herdr_socket: String,
}

/// Reads the device from `variable` and refuses, before anything connects,
/// a target that could be an operator's account. The install spells `~/`
/// from the SFTP home, which is the account's own home even where the
/// private sshd gives its commands another `HOME`: on 2026-10-09 a probe
/// with the `~/` defaults uploaded its node into a real account's
/// `~/.hide/host-helper`. So the alias must resolve to the private sshd's
/// port, named in `HERDR_TEST_REMOTE_SSH_PORT` and never 22; the private
/// HOME (`HERDR_TEST_REMOTE_HOME`) lives under `/tmp`, where no account's
/// own home does; the helper and command folders are absolute and inside
/// it; and the Herdr socket is named under `/tmp`, so the probe never asks
/// for the account's default Herdr.
fn isolated_device(
    ssh_config: &Path,
    variable: impl Fn(&str) -> Option<String>,
) -> Result<IsolatedDevice, String> {
    let read = |name: &str| {
        variable(name)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{name} is not set"))
    };
    let alias = read("HERDR_TEST_SSH_ALIAS")?;
    let port = read("HERDR_TEST_REMOTE_SSH_PORT")?;
    let port: u16 = port
        .parse()
        .map_err(|_| format!("HERDR_TEST_REMOTE_SSH_PORT {port:?} is not a port"))?;
    if port == 22 {
        return Err("HERDR_TEST_REMOTE_SSH_PORT is 22, an account's own sshd".to_owned());
    }
    let under_tmp = |name: &str| {
        let path = read(name)?;
        if plain_inside(&path, "/tmp") || plain_inside(&path, "/private/tmp") {
            Ok(path)
        } else {
            Err(format!("{name} {path:?} is not a private path under /tmp"))
        }
    };
    let home = under_tmp("HERDR_TEST_REMOTE_HOME")?;
    let inside_home = |name: &str| {
        let path = read(name)?;
        if plain_inside(&path, &home) {
            Ok(path)
        } else {
            Err(format!(
                "{name} {path:?} is not an absolute folder inside HERDR_TEST_REMOTE_HOME"
            ))
        }
    };
    let helper_root = inside_home("HERDR_TEST_REMOTE_HELPER_ROOT")?;
    let cli_dir = inside_home("HERDR_TEST_REMOTE_CLI_DIR")?;
    let herdr_socket = under_tmp("HERDR_TEST_REMOTE_HERDR_SOCKET")?;
    let resolved = hide_node::ssh::SshAlias::from_config_file(ssh_config, &alias)
        .map_err(|error| format!("SSH alias {alias:?} does not resolve: {error}"))?;
    if resolved.port != port {
        return Err(format!(
            "SSH alias {alias:?} resolves to port {}, not the private sshd's {port}",
            resolved.port
        ));
    }
    Ok(IsolatedDevice {
        alias,
        helper_root,
        cli_dir,
        herdr_socket,
    })
}

/// An absolute path strictly inside `folder`, with no empty, `.` or `..`
/// part that could step out of it.
fn plain_inside(path: &str, folder: &str) -> bool {
    !path.chars().any(char::is_control)
        && path
            .strip_prefix(folder)
            .and_then(|rest| rest.strip_prefix('/'))
            .is_some_and(|rest| {
                rest.split('/')
                    .all(|part| !part.is_empty() && part != "." && part != "..")
            })
}

/// One variable set to a value, or unset.
type Change<'a> = (&'a str, Option<&'a str>);

#[test]
fn the_probe_refuses_every_device_but_the_isolated_account_before_connecting() {
    let folder = tempfile::tempdir().unwrap();
    let config = folder.path().join("config");
    std::fs::write(
        &config,
        "Host private\n  HostName 127.0.0.1\n  Port 22841\n  User me\n\n\
         Host own\n  HostName 127.0.0.1\n  User me\n",
    )
    .unwrap();
    let isolated = [
        ("HERDR_TEST_SSH_ALIAS", "private"),
        ("HERDR_TEST_REMOTE_SSH_PORT", "22841"),
        ("HERDR_TEST_REMOTE_HOME", "/tmp/hcn/home"),
        (
            "HERDR_TEST_REMOTE_HELPER_ROOT",
            "/tmp/hcn/home/.hide/host-helper",
        ),
        ("HERDR_TEST_REMOTE_CLI_DIR", "/tmp/hcn/home/.local/bin"),
        ("HERDR_TEST_REMOTE_HERDR_SOCKET", "/tmp/hcn/h.sock"),
    ];
    let check = |changes: &[Change]| {
        let mut variables: HashMap<&str, Option<&str>> = isolated
            .iter()
            .map(|(name, value)| (*name, Some(*value)))
            .collect();
        variables.extend(changes.iter().copied());
        isolated_device(&config, |name| {
            variables.get(name).copied().flatten().map(str::to_owned)
        })
    };

    let device = check(&[]).expect("the isolated account is accepted");
    assert_eq!(device.helper_root, "/tmp/hcn/home/.hide/host-helper");
    assert_eq!(device.cli_dir, "/tmp/hcn/home/.local/bin");
    assert_eq!(device.herdr_socket, "/tmp/hcn/h.sock");

    let refusals: [(&[Change], &str); 11] = [
        (
            &[("HERDR_TEST_REMOTE_HELPER_ROOT", Some("~/.hide/host-helper"))],
            "HERDR_TEST_REMOTE_HELPER_ROOT",
        ),
        (
            &[("HERDR_TEST_REMOTE_CLI_DIR", Some("~/.local/bin"))],
            "HERDR_TEST_REMOTE_CLI_DIR",
        ),
        (
            &[(
                "HERDR_TEST_REMOTE_HELPER_ROOT",
                Some("/opt/me/.hide/host-helper"),
            )],
            "HERDR_TEST_REMOTE_HELPER_ROOT",
        ),
        (
            &[(
                "HERDR_TEST_REMOTE_HELPER_ROOT",
                Some("/tmp/hcn/home/../../opt/me/.hide/host-helper"),
            )],
            "HERDR_TEST_REMOTE_HELPER_ROOT",
        ),
        (
            &[("HERDR_TEST_REMOTE_HOME", Some("/opt/me"))],
            "HERDR_TEST_REMOTE_HOME",
        ),
        (
            &[("HERDR_TEST_REMOTE_HOME", None)],
            "HERDR_TEST_REMOTE_HOME",
        ),
        (
            &[("HERDR_TEST_REMOTE_HERDR_SOCKET", None)],
            "HERDR_TEST_REMOTE_HERDR_SOCKET",
        ),
        (
            &[(
                "HERDR_TEST_REMOTE_HERDR_SOCKET",
                Some("~/.config/herdr/herdr.sock"),
            )],
            "HERDR_TEST_REMOTE_HERDR_SOCKET",
        ),
        (
            &[
                ("HERDR_TEST_SSH_ALIAS", Some("own")),
                ("HERDR_TEST_REMOTE_SSH_PORT", Some("22")),
            ],
            "is 22",
        ),
        (
            &[("HERDR_TEST_SSH_ALIAS", Some("own"))],
            "resolves to port 22",
        ),
        (&[("HERDR_TEST_SSH_ALIAS", Some("absent"))], "absent"),
    ];
    for (changes, refusal) in refusals {
        let error = check(changes).expect_err("refused before connecting");
        assert!(error.contains(refusal), "{changes:?}: {error}");
    }
}

#[test]
#[ignore = "requires an owned remote fixture and HERDR_TEST_REMOTE_TERMINAL_* variables"]
fn official_remote_terminal_session_fixture_probe() {
    let home = PathBuf::from(std::env::var_os("HOME").expect("HOME is configured"));
    let device = isolated_device(&home.join(".ssh/config"), |name| std::env::var(name).ok())
        .unwrap_or_else(|refusal| panic!("refusing to reach the device: {refusal}"));
    let workspace_id = std::env::var("HERDR_TEST_REMOTE_TERMINAL_WORKSPACE_ID")
        .expect("HERDR_TEST_REMOTE_TERMINAL_WORKSPACE_ID names the owned fixture workspace");
    let pane_id = std::env::var("HERDR_TEST_REMOTE_TERMINAL_PANE_ID")
        .expect("HERDR_TEST_REMOTE_TERMINAL_PANE_ID names the owned fixture pane");
    let cwd = std::env::var("HERDR_TEST_REMOTE_TERMINAL_CWD")
        .expect("HERDR_TEST_REMOTE_TERMINAL_CWD names the owned fixture directory");
    // The node this build installs on the device: a folder holding `hided`
    // (and the kit's `hide` and `hide-agent-hooks`) for the device's system.
    let helper_dir = PathBuf::from(
        std::env::var_os("HERDR_TEST_REMOTE_HELPER_DIR")
            .expect("HERDR_TEST_REMOTE_HELPER_DIR names the folder of the build the device runs"),
    );
    assert!(
        cwd.starts_with("/tmp/hide-verify-"),
        "remote terminal fixture must use the owned fixture namespace"
    );

    let size = GridSize {
        rows: 30,
        cols: 100,
    };
    let heard = Arc::new(Heard {
        pane: pane_id.clone(),
        screen: Mutex::new(vt100::Parser::new(size.rows, size.cols, 0)),
        full_frame_seen: Mutex::default(),
        scrolled: Mutex::default(),
        states: Mutex::default(),
        progress: Mutex::default(),
    });
    let transport = hide_node::ssh::Connector::new(Some(helper_dir))
        .with_terminals(Arc::clone(&heard) as Arc<dyn DeviceSink>)
        .transport(
            &home,
            "probe",
            &device.alias,
            Some(device.herdr_socket.clone()),
        )
        .expect("SSH alias resolves");
    let snapshot = hide_herdr_client::request_with_connector(
        &*transport.herdr_api_connector(),
        "session.snapshot",
        json!({}),
        SSH_OPERATION_TIMEOUT,
    )
    .expect("fixture session snapshot");
    let snapshot = snapshot["snapshot"]
        .as_object()
        .map(|_| &snapshot["snapshot"])
        .expect("session.snapshot response contains a snapshot");
    let owned_workspace = snapshot["workspaces"]
        .as_array()
        .and_then(|workspaces| {
            workspaces
                .iter()
                .find(|workspace| workspace["workspace_id"].as_str() == Some(workspace_id.as_str()))
        })
        .expect("owned fixture workspace is present");
    assert!(
        owned_workspace["label"]
            .as_str()
            .is_some_and(|label| label.starts_with("hide-verify-")),
        "remote terminal refused a workspace outside the owned fixture namespace"
    );
    assert!(snapshot["panes"].as_array().is_some_and(|panes| {
        let canonical_cwd = cwd
            .strip_prefix("/tmp/")
            .map(|suffix| format!("/private/tmp/{suffix}"));
        panes.iter().any(|pane| {
            pane["pane_id"].as_str() == Some(pane_id.as_str())
                && pane["workspace_id"].as_str() == Some(workspace_id.as_str())
                && (pane["cwd"].as_str() == Some(cwd.as_str())
                    || pane["cwd"].as_str() == canonical_cwd.as_deref())
        })
    }));

    // The device's node link, with its terminal service started.
    let consent = HostConsent {
        contract: HOST_CONSENT_CONTRACT,
        helper_root: device.helper_root.clone(),
        cli_dir: Some(device.cli_dir.clone()),
        granted_at_unix_ms: 0,
        identity: None,
    };
    let (closed_sender, closed) = channel();
    let established = transport
        .establish(
            &consent,
            &[],
            hide_node_link::device::Attempt::unattended(
                hide_node_link::device::Wake::Allowed,
                Box::new(move |reason| {
                    let _ = closed_sender.send(reason);
                }),
            ),
        )
        .expect("the device's node link is established");
    let terminals = established
        .node
        .terminals
        .expect("the device's node started its terminal service");
    let (progress, heard_progress) = channel();
    *heard.progress.lock().unwrap() = Some(progress);

    terminals.view(&pane_id, size, true);
    terminals.control(TerminalControl::Attach {
        pane: pane_id.clone(),
        size: Some(size),
        manual: false,
    });
    wait_for(&heard_progress, "controlling", Duration::from_secs(15));
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    terminals.key(
        KeyTarget::Pane(pane_id.clone()),
        b"for i in {1..80}; do printf 'HIDE_SCROLL_%03d\\n' $i; done\r".to_vec(),
        now,
    );
    wait_for(&heard_progress, "tail", Duration::from_secs(15));
    // Only a full frame at the view's grid reaches the screen first, so
    // output at all means the session runs at 100 by 30.
    assert!(
        *heard.full_frame_seen.lock().unwrap(),
        "the remote terminal drew a full frame at the view's grid"
    );
    *heard.scrolled.lock().unwrap() = true;
    terminals.control(TerminalControl::Scroll {
        pane: pane_id.clone(),
        lines: 1000,
        column: None,
        row: None,
        modifiers: 0,
    });
    wait_for(&heard_progress, "history", Duration::from_secs(15));
    terminals.control(TerminalControl::Release {
        pane: pane_id.clone(),
        message: "probe finished".to_owned(),
    });
    wait_for(&heard_progress, "released", Duration::from_secs(5));
    eprintln!("pane states: {:?}", heard.states.lock().unwrap().as_slice());
    drop(terminals);
    drop(established.node.host);
    let _ = closed.recv_timeout(Duration::from_secs(5));
}
