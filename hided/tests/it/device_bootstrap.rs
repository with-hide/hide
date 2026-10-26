//! A fresh account gets its helper and pinned Herdr over real SSH before
//! any readable Herdr session is required. The server outlives that link.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use hide_kit::device_herdr::HerdrUpdate;
use hide_node_link::device::{DeviceConnector, HOST_CONSENT_CONTRACT, HostConsent};
use hide_node_link::protocol::{Call, KitAction};
use hide_node_link::terminal::TerminalReport;
use hide_node_link::{NodeLink, call_as};
use serde_json::{Value, json};

use super::support::remote_delivery::{Environment, successful, wait_for};
use super::support::ssh_server::Ssh;

struct Sink;
impl hide_node::terminal::device::DeviceSink for Sink {
    fn output(&self, _: &str, _: &str, _: &[u8], _: bool) {}
    fn report(&self, _: &str, _: TerminalReport) {}
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
        // A kit reply can be lost after it started the detached server.
        // This socket belongs only to the fixture's private account.
        let socket_server = match hide_platform::ipc::LocalStream::connect(&self.socket) {
            Ok(stream) => Some(stream.peer_pid()),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) =>
            {
                None
            }
            Err(error) => return Err(error.into()),
        };
        if socket_server.is_some() {
            let mut command = self.environment.command(&self.pinned);
            command
                .env("HERDR_SOCKET_PATH", &self.socket)
                .args(["server", "stop"]);
            successful(command)?;
        }
        let socket_pid = socket_server.transpose()?;
        for pid in [socket_pid, self.server_pid].into_iter().flatten() {
            wait_for("bootstrap server confirmed exit", || {
                Ok((!hide_platform::process::is_alive(pid)).then_some(()))
            })?;
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
            socket,
            pinned,
            ssh: None,
            link: None,
            server_pid: None,
        })
    }

    fn connect(&mut self) -> Result<hide_node_link::device::Established> {
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
            .with_terminals(Arc::new(Sink))
            .transport(&local, "bootstrap", "bootstrap", None)
            .map_err(anyhow::Error::msg)?;
        let established = transport
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
