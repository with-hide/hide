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
}

impl Drop for Device {
    fn drop(&mut self) {
        if let Some(link) = self.link.take() {
            link.close("bootstrap test ended");
        }
        let mut command = self.environment.command(&self.pinned);
        command
            .env("HERDR_SOCKET_PATH", &self.socket)
            .args(["server", "stop"]);
        let _ = super::support::remote_delivery::capture(command);
        if let Some(mut ssh) = self.ssh.take() {
            let _ = ssh.stop();
        }
    }
}

impl Device {
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
    let pid = *pid;
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
    Ok(())
}
