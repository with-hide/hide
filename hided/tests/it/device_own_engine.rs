//! A device of the core's own engine (PRD engine-bundle-3-core-owns D-22,
//! B35) over the system `ssh` and a real `sshd`: its node gets the pane
//! holder and no Herdr, starts a pane by the node link's pane requests, and
//! a pane whose node ended with its link runs on under its holder, so the
//! next link's node finds it with the same process. A caller in one of its
//! panes is proved by that pane's holder record, as a caller in a Herdr
//! pane is by its Herdr (D-9), so `hide` works there with no Herdr.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use hide_node_link::NodeLink;
use hide_node_link::device::{
    DeviceConnector, DialedTransport, HOST_CONSENT_CONTRACT, HostConsent,
};
use hide_node_link::holders::{HolderProof, HolderStart, PaneHolders};
use hide_node_link::link_panes::LinkPanes;
use hide_node_link::panes::{NodeEvent, ProofAnswer};
use hide_node_link::protocol::Call;

use super::support::remote_delivery::{Environment, wait_for};
use super::support::shipped;
use super::support::sshd::Sshd;

const PANE: &str = "own-device-pane";
/// The pane whose program asks the node's pane socket for a credential.
const ASKER: &str = "own-device-asker";
/// The credential this test answers a proof with, as the core would.
const TOKEN: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

/// The link's pane events, answered as the core answers a proved caller.
#[derive(Default)]
struct Events {
    heard: std::sync::Mutex<Vec<NodeEvent>>,
}

impl hide_node::ssh::PaneEvents for Events {
    fn event(&self, _node: &str, link: &hide_node::ssh::RemoteHost, event: NodeEvent) {
        if let NodeEvent::PaneProof { request, .. } = &event {
            let (link, request) = (link.clone(), *request);
            // The link's reader delivers this; it must never wait on the link.
            std::thread::spawn(move || {
                let answer = ProofAnswer::Issued {
                    token: TOKEN.to_owned(),
                    issued_new: true,
                };
                if let Err(error) = link.call(
                    Call::PaneProofAnswer { request, answer },
                    Duration::from_secs(20),
                ) {
                    eprintln!("the proof answer did not reach the node: {error}");
                }
            });
        }
        self.heard.lock().unwrap().push(event);
    }

    fn closed(&self, _node: &str, _link: &hide_node::ssh::RemoteHost) {}
}

/// The screen side of the device's terminals, which this test does not draw.
struct Unseen;
impl hide_node::terminal::device::DeviceSink for Unseen {
    fn output(&self, _: &str, _: &str, _: &[u8], _: bool) {}
    fn report(&self, _: &str, _: hide_node_link::terminal::TerminalReport) {}
}

/// One device account behind a loopback `sshd`, whose pane this test
/// started and ends on every path by its recorded program.
struct Device {
    /// Holds the account's folders until the fixture ends.
    _root: tempfile::TempDir,
    environment: Environment,
    ssh: Option<Sshd>,
    transport: Option<Arc<dyn DialedTransport>>,
    started: Vec<HolderProof>,
    state: PathBuf,
    events: Arc<Events>,
}

impl Drop for Device {
    fn drop(&mut self) {
        self.transport.take();
        // The holder outlives every node by design; the pane this test
        // started is ended by its program's session, recorded by pid and
        // start time, and its holder ends with it.
        for proof in self.started.drain(..) {
            match hide_platform::process::Session::left_by(proof.program, proof.started)
                .and_then(|mut session| session.end(Duration::from_secs(10)))
            {
                Ok(hide_platform::process::SessionEnd::Ended) => {}
                other => eprintln!("the device pane did not end: {other:?}"),
            }
        }
        if let Some(ssh) = self.ssh.as_mut()
            && let Err(error) = ssh.stop()
        {
            eprintln!("own device fixture cleanup failed: {error:#}");
        }
    }
}

impl Device {
    fn start() -> Result<Self> {
        let holder = PathBuf::from(
            std::env::var_os("HIDE_TEST_HOLDER_BIN")
                .context("set HIDE_TEST_HOLDER_BIN to the built hide-holder")?,
        );
        // A holder's socket path is limited to about a hundred bytes.
        let root = tempfile::Builder::new().prefix("hdo-").tempdir_in("/tmp")?;
        // The node binds its pane socket under its state folder, which must
        // stay short enough for a Unix socket path.
        let state = root.path().join("d");
        let environment = Environment::new(&root.path().join("r"), Some(&state))?;
        let packages = root.path().join("packages");
        std::fs::create_dir(&packages)?;
        shipped::place(
            std::path::Path::new(env!("CARGO_BIN_EXE_hided")),
            &packages.join("hided"),
            &environment,
        )?;
        shipped::place(&holder, &packages.join("hide-holder"), &environment)?;
        let local = root.path().join("local");
        std::fs::create_dir_all(local.join(".ssh"))?;
        let ssh = Sshd::start(&root.path().join("sshd"), &environment.values())?;
        std::fs::write(local.join(".ssh/config"), ssh.client_config("own-device"))?;
        let events = Arc::new(Events::default());
        let slot: hide_node::ssh::PaneEventsSlot = Arc::default();
        let _ = slot.set(Arc::clone(&events) as Arc<dyn hide_node::ssh::PaneEvents>);
        let transport = hide_node::ssh::Connector::new(Some(packages))
            .with_askpass_program(PathBuf::from(env!("CARGO_BIN_EXE_hide")))
            .with_terminals(Arc::new(Unseen))
            .with_pane_events(slot)
            .transport(&local, "own-device", "own-device", None)
            .map_err(|reason| anyhow::anyhow!("{reason}"))?;
        Ok(Self {
            _root: root,
            environment,
            ssh: Some(ssh),
            transport: Some(transport),
            started: Vec::new(),
            state,
            events,
        })
    }

    fn connect(&self) -> Result<hide_node_link::device::Established> {
        self.transport
            .as_ref()
            .context("the device's transport")?
            .establish(
                &HostConsent {
                    contract: HOST_CONSENT_CONTRACT,
                    helper_root: "~/.hide/host-helper".to_owned(),
                    cli_dir: Some("~/.local/bin".to_owned()),
                    granted_at_unix_ms: 1,
                    identity: None,
                },
                &[],
                hide_node_link::device::Attempt::unattended(
                    hide_node_link::device::Wake::Allowed,
                    Box::new(|_| {}),
                ),
            )
            .map_err(|error| anyhow::anyhow!("{error}"))
    }

    /// Every file the device's account holds whose name is Herdr's.
    fn herdr_files(&self) -> Result<Vec<PathBuf>> {
        let mut found = Vec::new();
        let mut folders = vec![self.environment.home.clone()];
        while let Some(folder) = folders.pop() {
            for entry in std::fs::read_dir(&folder)? {
                let entry = entry?;
                let kind = entry.file_type()?;
                if kind.is_dir() {
                    folders.push(entry.path());
                } else if entry.file_name().to_string_lossy().starts_with("herdr") {
                    found.push(entry.path());
                }
            }
        }
        Ok(found)
    }

    fn panes(link: &Arc<dyn NodeLink>) -> LinkPanes {
        LinkPanes::new(
            Arc::clone(link),
            Arc::new(|record| eprintln!("device pane record: {record}")),
        )
    }

    fn screen(panes: &LinkPanes, pane: &str) -> Result<String> {
        Ok(panes
            .read(pane, 40)
            .map_err(|error| anyhow::anyhow!("{error:?}"))?
            .0)
    }
}

#[test]
#[ignore = "real candidate node and pane holder over loopback SSH; set HIDE_TEST_HOLDER_BIN"]
fn a_device_of_the_own_engine_runs_no_herdr_and_its_pane_outlives_the_node() -> Result<()> {
    let mut device = Device::start()?;
    let first = device.connect()?;
    ensure!(
        first.node.terminals.is_ok(),
        "the device's terminals did not start on its holders: {:?}",
        first.node.terminals.as_ref().err()
    );
    let link = Arc::clone(&first.node.host);
    let panes = Device::panes(&link);
    let recovery = panes
        .recovered(Duration::from_secs(10))
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    ensure!(
        recovery.present.is_empty(),
        "a fresh device found panes: {recovery:?}"
    );
    let cwd = hide_platform::path::to_wire(&device.environment.home.join("project"))?;
    let start = |pane: &str, line: String| {
        // As the core's own engine starts a pane: a number, the pane's
        // reservation, its start, then the number retired.
        let start = HolderStart {
            pane: pane.to_owned(),
            program: Some(vec!["/bin/sh".to_owned(), "-c".to_owned(), line]),
            cwd: cwd.clone(),
            env: Vec::new(),
            rows: 24,
            cols: 120,
            notice: None,
        };
        let request = panes
            .request()
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        panes
            .reserve(&start, None, request)
            .and_then(|reserved| panes.start_reserved(&start, reserved, request))
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        panes
            .retire(request)
            .map_err(|error| anyhow::anyhow!("{error:?}"))?;
        panes
            .proof(pane)
            .map_err(|error| anyhow::anyhow!("{error:?}"))
    };
    let proof = start(PANE, "printf 'own_%s\\n' device; exec sleep 600".to_owned())?;
    device.started.push(proof);

    // A caller in a pane is proved by the pane's holder record: the node
    // checks the caller descends from the program the record names and
    // sends that identity up the link.
    let bridges = std::fs::read_dir(device.state.join("workspace-bridges"))?
        .map(|entry| Ok(entry?.path().join("bootstrap.sock")))
        .filter(|socket: &Result<PathBuf>| socket.as_ref().is_ok_and(|socket| socket.exists()))
        .collect::<Result<Vec<_>>>()?;
    let [socket] = &bridges[..] else {
        anyhow::bail!("the node's pane sockets: {bridges:?}");
    };
    let ask = format!(
        "python3 -c 'import json,socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); s.sendall((json.dumps({{\"pane_id\": sys.argv[2], \"nonce\": \"a\"*32, \"one_shot\": False}})+chr(10)).encode()); print(\"answer\", json.loads(s.makefile().readline()).get(\"ok\"))' {} {ASKER}; exec sleep 600",
        socket.display()
    );
    let asker = start(ASKER, ask)?;
    device.started.push(asker);
    wait_for("the asker's credential", || {
        Ok(Device::screen(&panes, ASKER)?
            .contains("answer True")
            .then_some(()))
    })?;
    let heard = device.events.heard.lock().unwrap().clone();
    ensure!(
        matches!(
            &heard[..],
            [NodeEvent::PaneProof { pane_id, identity, .. }]
                if pane_id == ASKER
                    && identity.terminal_id == ASKER
                    && u32::try_from(identity.shell_pid).ok() == Some(asker.program)
                    && identity.shell_started == asker.started
        ),
        "the proof did not name the asker's holder record {asker:?}: {heard:?}"
    );
    panes
        .terminate(ASKER)
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    device.started.retain(|proof| proof != &asker);
    wait_for("the pane's first output", || {
        Ok(Device::screen(&panes, PANE)?
            .contains("own_device")
            .then_some(()))
    })?;
    let herdr = device.herdr_files()?;
    ensure!(herdr.is_empty(), "the device got Herdr: {herdr:?}");

    // The node ends with its link, as when the device's network drops or
    // the core restarts; the holder keeps the pane's process.
    drop(panes);
    link.close("the node restarts");
    drop(link);
    drop(first);
    wait_for("the device's node exit", || {
        Ok((device.ssh.as_ref().unwrap().open() == 0).then_some(()))
    })?;
    ensure!(
        hide_platform::process::is_alive(proof.program),
        "the pane's program ended with its node"
    );

    let second = device.connect()?;
    let link = Arc::clone(&second.node.host);
    let panes = Device::panes(&link);
    let recovery = panes
        .recovered(Duration::from_secs(10))
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    ensure!(
        recovery.present == [PANE.to_owned()],
        "the new node did not find the pane: {recovery:?}"
    );
    let again = panes
        .proof(PANE)
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    ensure!(
        again == proof,
        "the pane's process changed: {proof:?} then {again:?}"
    );
    ensure!(Device::screen(&panes, PANE)?.contains("own_device"));

    panes
        .terminate(PANE)
        .map_err(|error| anyhow::anyhow!("{error:?}"))?;
    wait_for("the pane's program end", || {
        Ok((!hide_platform::process::is_alive(proof.program)).then_some(()))
    })?;
    device.started.clear();
    let herdr = device.herdr_files()?;
    ensure!(herdr.is_empty(), "the device got Herdr: {herdr:?}");
    drop(panes);
    link.close("test ended");
    Ok(())
}
