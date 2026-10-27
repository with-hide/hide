//! A fresh account gets its helper and pinned Herdr over real SSH before
//! any readable Herdr session is required. The server outlives that link.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use hide_kit::device_herdr::HerdrUpdate;
use hide_node_link::device::{
    DeviceConnector, DialedTransport, HOST_CONSENT_CONTRACT, HostConsent,
};
use hide_node_link::protocol::{Call, KitAction};
use hide_node_link::terminal::{GridSize, KeyTarget, TerminalControl, TerminalReport};
use hide_node_link::{NodeLink, call_as};
use serde_json::{Value, json};

use super::support::remote_delivery::{Environment, successful, wait_for};
use super::support::ssh_server::Ssh;

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

/// Stops even a detached server when an assertion fails. The private
/// account owns its config as well as its socket; no operator config loads.
struct Device {
    root: tempfile::TempDir,
    environment: Environment,
    socket: PathBuf,
    pinned: PathBuf,
    ssh: Option<Ssh>,
    link: Option<Arc<dyn NodeLink>>,
    server_pid: Option<u32>,
    transport: Option<Arc<dyn DialedTransport>>,
    default_socket: PathBuf,
    sink: Arc<Sink>,
}

impl Drop for Device {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("bootstrap fixture cleanup failed: {error:#}");
        }
    }
}

impl Device {
    fn stop(&mut self) -> Result<()> {
        if let Some(link) = self.link.take() {
            link.close("bootstrap test ended");
        }
        self.transport.take();
        // A kit reply can be lost after it started the detached server.
        // This socket belongs only to the fixture's private account.
        for socket in [self.socket.clone(), self.default_socket.clone()] {
            let socket_server = match hide_platform::ipc::LocalStream::connect(&socket) {
                Ok(stream) => Some(stream.peer_pid()),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound
                            | std::io::ErrorKind::NotADirectory
                            | std::io::ErrorKind::ConnectionRefused
                    ) =>
                {
                    None
                }
                Err(error) => return Err(error.into()),
            };
            if socket_server.is_some() {
                let mut command = self.environment.command(&self.pinned);
                command
                    .env("HERDR_SOCKET_PATH", &socket)
                    .args(["server", "stop"]);
                successful(command)?;
            }
            let socket_pid = socket_server.transpose()?;
            for pid in [socket_pid, self.server_pid].into_iter().flatten() {
                wait_for("bootstrap server confirmed exit", || {
                    Ok((!hide_platform::process::is_alive(pid)).then_some(()))
                })?;
            }
        }
        self.server_pid = None;
        if let Some(ssh) = self.ssh.as_mut() {
            ssh.stop()?;
        }
        self.ssh = None;
        Ok(())
    }
    fn fresh() -> Result<Self> {
        let pinned = PathBuf::from(
            std::env::var_os("HIDE_E2E_HERDR_BIN").context("set HIDE_E2E_HERDR_BIN")?,
        );
        let root = tempfile::Builder::new().prefix("hdb-").tempdir_in("/tmp")?;
        let socket = root.path().join("r/config/herdr/herdr.sock");
        let mut environment = Environment::new(&root.path().join("r"), &socket, &pinned, None)?;
        for key in ["HERDR_SOCKET_PATH", "HERDR_BIN_PATH", "HERDR_SESSION"] {
            environment.unset(key);
        }
        Ok(Self {
            root,
            environment,
            default_socket: socket.clone(),
            socket,
            pinned,
            ssh: None,
            link: None,
            server_pid: None,
            transport: None,
            sink: Arc::default(),
        })
    }

    fn prepare(&mut self) -> Result<()> {
        let root = self.root.path();
        let packages = root.join("packages");
        std::fs::create_dir(&packages)?;
        // Stage real candidate code, as the existing SSH mailbox fixture
        // does, with debug data removed only from this private copy.
        let helper = packages.join("hided");
        std::fs::copy(env!("CARGO_BIN_EXE_hided"), &helper)?;
        let mut strip = self.environment.command("/usr/bin/strip");
        strip.arg(&helper);
        successful(strip)?;
        #[cfg(target_os = "macos")]
        {
            let mut sign = self.environment.command("/usr/bin/codesign");
            sign.args(["--force", "--sign", "-"]).arg(&helper);
            successful(sign)?;
        }
        std::fs::copy(&self.pinned, packages.join("herdr"))?;
        let local = root.join("local");
        let keys = local.join(".ssh");
        std::fs::create_dir_all(&keys)?;
        for name in ["host", "client"] {
            let mut keygen = self.environment.command("/usr/bin/ssh-keygen");
            keygen
                .args(["-q", "-t", "ed25519", "-N", "", "-f"])
                .arg(keys.join(name));
            successful(keygen)?;
        }
        let ssh = Ssh::start(
            self.environment.clone(),
            self.socket.clone(),
            &keys.join("host"),
            &keys.join("client"),
        )?;
        let public = std::fs::read_to_string(keys.join("host.pub"))?;
        std::fs::write(
            keys.join("known_hosts"),
            format!("[127.0.0.1]:{} {}\n", ssh.port, public.trim()),
        )?;
        std::fs::write(
            keys.join("config"),
            format!(
                "Host bootstrap\n  HostName 127.0.0.1\n  Port {}\n  User fixture\n  IdentityFile {}\n  IdentityAgent none\n",
                ssh.port,
                keys.join("client").display()
            ),
        )?;
        self.ssh = Some(ssh);
        let transport = hide_node::ssh::Connector::new(Some(packages))
            .with_helper_root("~/.hide/host-helper".to_owned())
            .with_terminals(self.sink.clone())
            .with_pane_events(Arc::default())
            .transport(&local, "bootstrap", "bootstrap", None)
            .map_err(|reason| anyhow::anyhow!("{reason}"))?;
        self.transport = Some(transport);
        Ok(())
    }

    fn connect(&mut self) -> Result<hide_node_link::device::Established> {
        if self.transport.is_none() {
            self.prepare()?;
        }
        let established = self
            .transport
            .as_ref()
            .unwrap()
            .establish(
                &HostConsent {
                    contract: HOST_CONSENT_CONTRACT,
                    helper_root: "~/.hide/host-helper".to_owned(),
                    cli_dir: Some("~/.local/bin".to_owned()),
                    granted_at_unix_ms: 1,
                    identity: None,
                },
                &[],
                Box::new(|_| {}),
            )
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        self.link = Some(Arc::clone(&established.node.host));
        Ok(established)
    }

    fn snapshot(&self) -> Result<Value> {
        hide_herdr_client::request_with_timeout(
            &self.socket,
            "session.snapshot",
            json!({}),
            Duration::from_secs(5),
        )
        .map_err(|error| anyhow::anyhow!("{error}"))
    }

    fn usable_terminal(&self, connected: &hide_node_link::device::Established) -> Result<()> {
        let bridges = hide_kit::layout::workspace_bridges(&self.root.path().join("r/state/hide"));
        let mut pane_service = false;
        for bridge in std::fs::read_dir(bridges)? {
            let socket = bridge?.path().join("bootstrap.sock");
            if hide_platform::ipc::LocalStream::connect(&socket).is_ok() {
                pane_service = true;
            }
        }
        ensure!(pane_service, "retry left the pane service unavailable");
        let created = hide_herdr_client::request_with_timeout(
            &self.socket,
            "workspace.create",
            json!({"cwd": self.environment.home.join("project"), "label": "retry"}),
            Duration::from_secs(5),
        )
        .map_err(|error| anyhow::anyhow!("{error}"))?;
        fn pane(value: &Value) -> Option<&str> {
            match value {
                Value::Object(map) => map
                    .get("pane_id")
                    .and_then(Value::as_str)
                    .or_else(|| map.values().find_map(pane)),
                Value::Array(values) => values.iter().find_map(pane),
                _ => None,
            }
        }
        let pane = pane(&created).context("new workspace pane")?.to_owned();
        let terminals = connected
            .node
            .terminals
            .as_ref()
            .map_err(|reason| anyhow::anyhow!("{reason}"))?;
        terminals.control(TerminalControl::Attach {
            pane: pane.clone(),
            size: Some(GridSize { rows: 24, cols: 80 }),
            manual: false,
        });
        wait_for("retry terminal controlling", || {
            Ok(self
                .sink
                .reports
                .lock()
                .unwrap()
                .iter()
                .any(|report| {
                    matches!(
                        report, TerminalReport::State { pane: reported, state }
                    if reported == &pane && state.state == "controlling"
                    )
                })
                .then_some(()))
        })?;
        terminals.key(
            KeyTarget::Pane(pane),
            b"printf 'retry_%s\\n' usable\r".to_vec(),
            1,
        );
        wait_for("retry terminal command output", || {
            Ok(String::from_utf8_lossy(&self.sink.output.lock().unwrap())
                .contains("retry_usable")
                .then_some(()))
        })?;
        Ok(())
    }
}

/// A running compatible server needs no bootstrap ahead of helper
/// readiness. Its kit is left to the core's normal post-connection pass.
#[test]
#[ignore = "real private SSH and pinned Herdr; the remote mailbox lane supplies the binary"]
fn a_readable_server_connects_its_services_before_the_automatic_kit() -> Result<()> {
    let mut device = Device::fresh()?;
    let user_bin = device.environment.home.join(".local/bin");
    std::fs::create_dir_all(&user_bin)?;
    std::os::unix::fs::symlink(&device.pinned, user_bin.join("herdr"))?;
    let mut command = device.environment.command(&device.pinned);
    command
        .arg("server")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut server = hide_platform::process::OwnedChild::spawn(&mut command)?;
    wait_for("compatible server readiness", || {
        Ok(device.snapshot().ok().map(|_| ()))
    })?;
    let established = device.connect()?;
    ensure!(
        established.kit.is_none(),
        "a readable server blocked readiness on the kit"
    );
    ensure!(
        established.node.terminals.is_ok(),
        "terminal services were not ready"
    );
    device.snapshot()?;
    device.stop()?;
    server.kill_tree()?;
    wait_for("compatible server confirmed exit", || {
        Ok(server.try_wait()?.map(|_| ()))
    })?;
    Ok(())
}

#[test]
#[ignore = "real candidate helper and pinned Herdr over loopback SSH; set HIDE_E2E_HERDR_BIN"]
fn a_fresh_device_bootstraps_before_terminal_start_and_keeps_herdr_after_removal() -> Result<()> {
    let mut device = Device::fresh()?;
    ensure!(
        !device.socket.exists(),
        "fresh account already has a server"
    );
    ensure!(!device.environment.home.join(".local/bin/herdr").exists());
    let established = device.connect()?;
    let report = established
        .kit
        .as_ref()
        .context("connection kit report")?
        .as_ref()
        .map_err(|reason| anyhow::anyhow!("{reason}"))?;
    let Some(HerdrUpdate::Started { pid, version }) = report.herdr.as_deref() else {
        anyhow::bail!(
            "fresh device did not start its pinned server: {:?}",
            report.herdr
        );
    };
    let pid = *pid;
    device.server_pid = Some(pid);
    ensure!(version == &super::private_herdr::pinned_version());
    ensure!(
        established.node.terminals.is_ok(),
        "terminals started before Herdr was ready: {:?}",
        established.node.terminals.as_ref().err()
    );
    let snapshot = device.snapshot()?;
    ensure!(
        snapshot
            .pointer("/snapshot/protocol")
            .and_then(Value::as_u64)
            == Some(hide_herdr_client::HERDR_PROTOCOL_REVISION),
        "unreadable fresh session: {snapshot}"
    );
    let _: hide_node_link::protocol::KitRemoved = call_as(
        established.node.host.as_ref(),
        Call::Kit {
            action: KitAction::Remove,
            cli_dir: "~/.local/bin".to_owned(),
            herdr_socket: Some(device.socket.to_string_lossy().into_owned()),
            preserve_herdr_server: false,
            retirement_projects: vec![],
        },
        Duration::from_secs(30),
    )
    .map_err(|error| anyhow::anyhow!("{error}"))?;
    established.node.host.close("device removed");
    drop(established);
    device.link.take();
    device.transport.take();
    wait_for("helper exit after removal", || {
        Ok((device.ssh.as_ref().unwrap().open() == 0).then_some(()))
    })?;
    device.ssh.as_mut().unwrap().stop()?;
    ensure!(
        hide_platform::process::is_alive(pid),
        "server ended with its node or SSH link"
    );
    device.snapshot()?;
    device.stop()?;
    Ok(())
}

/// A status read without a running server still selects the environment's
/// socket. Bootstrap must not create a server at the account default.
#[test]
#[ignore = "real candidate helper and pinned Herdr over loopback SSH; set HIDE_E2E_HERDR_BIN"]
fn bootstrap_keeps_the_environments_socket_when_no_server_is_running() -> Result<()> {
    let mut device = Device::fresh()?;
    let default = device.socket.clone();
    device.socket = device.root.path().join("chosen.sock");
    device
        .environment
        .set("HERDR_SOCKET_PATH", device.socket.as_os_str());
    let user_bin = device.environment.home.join(".local/bin");
    std::fs::create_dir_all(&user_bin)?;
    std::os::unix::fs::symlink(&device.pinned, user_bin.join("herdr"))?;
    let established = device.connect()?;
    let report = established
        .kit
        .as_ref()
        .context("bootstrap report")?
        .as_ref()
        .map_err(|reason| anyhow::anyhow!("{reason}"))?;
    let Some(HerdrUpdate::Started { pid, .. }) = report.herdr.as_deref() else {
        anyhow::bail!("bootstrap did not start Herdr: {:?}", report.herdr);
    };
    device.server_pid = Some(*pid);
    device.snapshot()?;
    ensure!(
        !default.exists(),
        "bootstrap touched the account default socket"
    );
    ensure!(established.node.terminals.is_ok());
    device.stop()
}

#[test]
#[ignore = "real candidate helper and pinned Herdr over loopback SSH; set HIDE_E2E_HERDR_BIN"]
fn bootstrap_uses_the_environment_session_when_no_probe_binary_exists() -> Result<()> {
    let mut device = Device::fresh()?;
    let default = device.socket.clone();
    device.socket = default.parent().unwrap().join("sessions/work/herdr.sock");
    device.environment.set("HERDR_SESSION", "work");
    let connected = device.connect()?;
    let report = connected
        .kit
        .as_ref()
        .context("bootstrap report")?
        .as_ref()
        .map_err(|reason| anyhow::anyhow!("{reason}"))?;
    let Some(HerdrUpdate::Started { pid, .. }) = report.herdr.as_deref() else {
        anyhow::bail!("named environment did not start Herdr: {:?}", report.herdr);
    };
    device.server_pid = Some(*pid);
    device.snapshot()?;
    ensure!(!default.exists(), "bootstrap touched the default session");
    ensure!(connected.node.terminals.is_ok());
    device.stop()
}

/// A required upload is retried through establishment. An incomplete pin
/// must not replace the last valid copy or make the helper look ready.
#[test]
#[ignore = "real candidate helper and pinned Herdr over loopback SSH; set HIDE_E2E_HERDR_BIN"]
fn a_failed_pinned_upload_preserves_current_and_reconnect_sends_it_again() -> Result<()> {
    let mut device = Device::fresh()?;
    device.prepare()?;
    let root = device.environment.home.join(".hide/host-helper/herdr");
    let old = root.join("0000000000000000");
    std::fs::create_dir_all(&old)?;
    std::fs::copy(&device.pinned, old.join("herdr"))?;
    std::os::unix::fs::symlink("0000000000000000", root.join("current"))?;
    let pin: Value = serde_json::from_str(include_str!("../../../contracts/herdr-bundle.json"))?;
    let digest = if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        pin["linux_aarch64"]["sha256"].as_str()
    } else if cfg!(target_os = "linux") {
        pin["linux_x86_64"]["sha256"].as_str()
    } else {
        pin["sha256"].as_str()
    }
    .context("platform pin digest")?;
    let blocked = root.join(&digest[..16]);
    std::fs::write(&blocked, "a directory cannot be created here")?;
    let failed = device.connect();
    ensure!(
        failed.is_err(),
        "a failed required upload returned a ready helper"
    );
    ensure!(std::fs::read_link(root.join("current"))? == PathBuf::from("0000000000000000"));
    ensure!(old.join("herdr").is_file());
    std::fs::remove_file(blocked)?;
    let connected = device.connect()?;
    let report = connected
        .kit
        .as_ref()
        .context("retry report")?
        .as_ref()
        .map_err(|reason| anyhow::anyhow!("{reason}"))?;
    let Some(HerdrUpdate::Started { pid, .. }) = report.herdr.as_deref() else {
        anyhow::bail!("retry did not deliver and start Herdr: {:?}", report.herdr);
    };
    device.server_pid = Some(*pid);
    ensure!(connected.node.terminals.is_ok());
    device.snapshot()?;
    device.stop()
}

#[test]
#[ignore = "real candidate helper and pinned Herdr over loopback SSH; set HIDE_E2E_HERDR_BIN"]
fn reconnect_after_a_failed_start_restores_a_usable_terminal() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut device = Device::fresh()?;
    // Reading a missing socket succeeds as a probe, while the account
    // cannot bind a new socket until its folder is writable again.
    let folder = device.root.path().join("locked");
    std::fs::create_dir(&folder)?;
    device.socket = folder.join("herdr.sock");
    device
        .environment
        .set("HERDR_SOCKET_PATH", device.socket.as_os_str());
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o500))?;
    let first = device.connect();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))?;
    let failed = first?;
    let report = failed
        .kit
        .as_ref()
        .context("first report")?
        .as_ref()
        .map_err(|reason| anyhow::anyhow!("{reason}"))?;
    ensure!(
        matches!(report.herdr.as_deref(), Some(HerdrUpdate::Failed { stage, .. }) if stage == "start"),
        "first start did not fail: {:?}",
        report.herdr
    );
    failed.node.host.close("retry bootstrap");
    drop(failed);
    device.link.take();
    let connected = device.connect()?;
    let report = connected
        .kit
        .as_ref()
        .context("retry report")?
        .as_ref()
        .map_err(|reason| anyhow::anyhow!("{reason}"))?;
    let Some(HerdrUpdate::Started { pid, .. }) = report.herdr.as_deref() else {
        anyhow::bail!("retry did not start Herdr: {:?}", report.herdr);
    };
    device.server_pid = Some(*pid);
    device.usable_terminal(&connected)?;
    device.stop()
}
