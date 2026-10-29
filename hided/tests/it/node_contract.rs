//! The node contract (PRD core-host-node b5): the core's own node
//! (`hide_node::Local`) and a device's node reached over SSH, `hided node
//! serve` behind the system's own `sshd` on loopback, answer the same calls
//! alike, and everything a device does rides its one connection (PRD
//! system-ssh-device-link B22). The server is a real `sshd`, which the
//! remote mailbox lane provides, so these tests are run there.
#![cfg(unix)]

use crate::support::sshd::Sshd;

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use hide_node_link::device::{
    DeviceConnector, DialedTransport, HOST_CONSENT_CONTRACT, HostConsent,
};
use hide_node_link::panes::{NodeEvent, ProofAnswer};
use hide_node_link::protocol::{Call, RootOpened, RootRef};
use hide_node_link::{NodeLink, call_as};
use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(20);
const ALIAS: &str = "contract-device";

/// The device's private account: its home and the environment every process
/// the SSH server starts for it gets, and nothing of this process's.
struct Account {
    home: PathBuf,
    values: Vec<(OsString, OsString)>,
}

impl Account {
    #[allow(clippy::disallowed_methods)] // env_clear: a session's environment (Unix only)
    fn command(&self, program: &OsStr) -> Command {
        let mut command = Command::new(program);
        command
            .env_clear()
            .envs(self.values.iter().map(|(name, value)| (name, value)))
            .current_dir(&self.home);
        command
    }
}

/// A device on the loopback SSH server with a committed checkout and a
/// change in it, and its node started over one connection.
struct Device {
    _root: tempfile::TempDir,
    home: PathBuf,
    project: PathBuf,
    ssh: Sshd,
    transport: Arc<dyn DialedTransport>,
    link: Arc<dyn NodeLink>,
    /// The device's state folder, where its node binds its pane socket.
    state: PathBuf,
    /// What the link's pane events told this process.
    events: Arc<Events>,
    /// The device's terminals, when its connector carries them.
    terminals: Option<Arc<dyn hide_node_link::terminal::TerminalNode>>,
    hello: hide_node_link::protocol::Hello,
    helper_path: String,
    local: PathBuf,
    current_packages: PathBuf,
}

/// The pane of the device's Herdr whose shell is this test process, so a
/// caller from here is one the device's node can prove.
const PROVED_PANE: &str = "w1:p2";
/// The credential the core side of these tests issues for a proof.
const TOKEN: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";

/// The pane events of the device's link, as hided would receive them. It
/// answers a proof as the core would: a credential for a lasting caller, and
/// `pane_changed` for a one-shot one.
#[derive(Default)]
struct Events {
    heard: std::sync::Mutex<Vec<NodeEvent>>,
    arrived: std::sync::Condvar,
    closed: std::sync::Mutex<Vec<String>>,
}

impl Events {
    /// The events heard so far, once `done` holds of them or the bound ends.
    fn heard_until(&self, done: impl Fn(&[NodeEvent]) -> bool) -> Vec<NodeEvent> {
        let heard = self.heard.lock().unwrap();
        let (heard, _) = self
            .arrived
            .wait_timeout_while(heard, TIMEOUT, |heard| !done(heard))
            .unwrap();
        heard.clone()
    }
}

impl hide_node::ssh::PaneEvents for Events {
    fn event(&self, _node: &str, link: &hide_node::ssh::RemoteHost, event: NodeEvent) {
        if let NodeEvent::PaneProof {
            request,
            one_shot,
            hook,
            ..
        } = &event
        {
            let answer = if let Some(hook) = hook
                && !hook.command.one_shot()
            {
                ProofAnswer::WorkspaceInfo {
                    token: TOKEN.to_owned(),
                    issued_new: true,
                    answer: json!({"ok":true,"request_id":hook.request_id,"result":{"context":{"device_id":"fixture"},"capabilities":["workspace.info"]}}),
                }
            } else if let Some(hook) = hook {
                ProofAnswer::Hook {
                    answer: json!({"ok":true,"request_id":hook.request_id,"result":{"context":null,"outcome":"empty"}}),
                }
            } else if *one_shot {
                ProofAnswer::Refused {
                    reason: "pane_changed".to_owned(),
                    next_action: "Reconnect the pane and retry".to_owned(),
                }
            } else {
                ProofAnswer::Issued {
                    token: TOKEN.to_owned(),
                    issued_new: true,
                }
            };
            let (link, request) = (link.clone(), *request);
            // The link's reader delivers this; it must never wait on the link.
            std::thread::spawn(move || {
                link.call(Call::PaneProofAnswer { request, answer }, TIMEOUT)
                    .unwrap();
            });
        }
        self.heard.lock().unwrap().push(event);
        self.arrived.notify_all();
    }

    fn closed(&self, node: &str, _link: &hide_node::ssh::RemoteHost) {
        self.closed.lock().unwrap().push(node.to_owned());
    }
}

fn run(command: &mut Command) {
    let output = command.output().unwrap();
    assert!(output.status.success(), "{command:?}: {output:?}");
}

impl Device {
    fn start() -> Self {
        Self::start_with_pane_events(None)
    }

    fn start_with_pane_events(pane_events: Option<Arc<dyn hide_node::ssh::PaneEvents>>) -> Self {
        Self::start_with(pane_events, None, None)
    }

    fn start_with(
        pane_events: Option<Arc<dyn hide_node::ssh::PaneEvents>>,
        terminals: Option<Arc<dyn hide_node::terminal::device::DeviceSink>>,
        retained_packages: Option<PathBuf>,
    ) -> Self {
        // Socket paths below this folder must fit a Unix socket address.
        let root = tempfile::Builder::new()
            .prefix("nc")
            .tempdir_in("/tmp")
            .unwrap();
        // The canonical path: the install refuses a link among a state
        // folder's parents, and `/tmp` is one on macOS.
        let base = fs::canonicalize(root.path()).unwrap();
        let home = base.join("h");
        let project = home.join("project");
        fs::create_dir_all(&project).unwrap();
        let herdr_socket = base.join("s.sock");
        let state = base.join("st");
        let account = Account {
            home: home.clone(),
            values: vec![
                ("HOME".into(), home.clone().into()),
                ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
                ("SHELL".into(), "/bin/sh".into()),
                ("HIDE_STATE_DIR".into(), state.clone().into()),
                // The device's node bridges its own Herdr, the one its
                // account's environment names (PRD system-ssh-device-link
                // B24).
                ("HERDR_SOCKET_PATH".into(), herdr_socket.clone().into()),
                ("HIDE_TEST_HIDED".into(), env!("CARGO_BIN_EXE_hided").into()),
            ],
        };
        let git = |args: &[&str]| {
            let mut command = account.command(OsStr::new("/usr/bin/git"));
            command.args(["-c", "init.defaultBranch=main", "-c", "user.name=Fixture"]);
            command.args(["-c", "user.email=fixture@example.invalid", "-C"]);
            command.arg(&project).args(args);
            run(&mut command);
        };
        fs::write(project.join("a.txt"), "one\n").unwrap();
        git(&["init", "-q"]);
        git(&["add", "a.txt"]);
        git(&["commit", "-q", "-m", "first"]);
        fs::write(project.join("a.txt"), "two\n").unwrap();
        fs::write(project.join("b.txt"), "new\n").unwrap();
        // The device's Herdr answers where its server listens; nothing is
        // served there but the socket a Herdr channel opens.
        let bin = home.join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        let status = json!({"running": true, "socket": herdr_socket, "version": "0.9.1"});
        executable(
            &bin.join("herdr"),
            &format!("#!/bin/sh\nprintf '%s\\n' '{status}'\n"),
        );
        // A Herdr that knows one pane, whose shell is this test process, and
        // hangs up on anything else: a channel to it opens, and asking it
        // about another pane fails rather than waits.
        let herdr = UnixListener::bind(&herdr_socket).unwrap();
        std::thread::spawn(move || {
            for connection in herdr.incoming().flatten() {
                std::thread::spawn(move || answer_herdr(connection));
            }
        });

        let ssh = Sshd::start(&base.join("k"), &account.values).unwrap();
        let local = base.join("l");
        fs::create_dir_all(local.join(".ssh")).unwrap();
        fs::write(local.join(".ssh/config"), ssh.client_config(ALIAS)).unwrap();
        // The build this daemon carries for the device: a launcher for the
        // `hided` under test, so the install moves a few bytes rather than a
        // debug binary, and the device runs this build's node.
        let packages = base.join("p");
        fs::create_dir(&packages).unwrap();
        executable(
            &packages.join("hided"),
            "#!/bin/sh\nexec \"$HIDE_TEST_HIDED\" \"$@\"\n",
        );
        let events = Arc::new(Events::default());
        let slot: hide_node::ssh::PaneEventsSlot = Arc::default();
        let _ = slot.set(pane_events.unwrap_or_else(|| events.clone()));
        let mut connector = hide_node::ssh::Connector::new(Some(
            retained_packages.unwrap_or_else(|| packages.clone()),
        ))
        .with_askpass_program(PathBuf::from(env!("CARGO_BIN_EXE_hide")))
        .with_pane_events(slot);
        if let Some(sink) = terminals {
            connector = connector.with_terminals(sink);
        }
        let transport = connector
            .transport(&local, "contract-node", ALIAS, None)
            .unwrap();
        let consent = HostConsent {
            contract: HOST_CONSENT_CONTRACT,
            helper_root: "~/helper".to_owned(),
            cli_dir: None,
            granted_at_unix_ms: 1,
            identity: None,
        };
        let established = match transport.establish(
            &consent,
            &[],
            hide_node_link::device::Attempt::unattended(
                hide_node_link::device::Wake::Allowed,
                Box::new(|_| {}),
            ),
        ) {
            Ok(established) => established,
            Err(error) => panic!("the device's node did not start: {error:?}"),
        };
        Self {
            _root: root,
            home,
            project,
            ssh,
            transport,
            link: established.node.host,
            state,
            events,
            terminals: established.node.terminals.ok(),
            hello: established.node.hello,
            helper_path: established.helper_path,
            local,
            current_packages: packages,
        }
    }
}

#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn authenticated_node_advertises_only_its_compiled_readers() {
    use hide_node_link::sessions::{ReaderFeature, ReaderFeatures};
    let device = Device::start();
    assert_eq!(
        device.hello.protocol,
        hide_node_link::protocol::PROTOCOL_VERSION
    );
    assert_eq!(
        device.link.reader_features(),
        Some(&ReaderFeatures::implemented())
    );
    assert!(
        device
            .link
            .reader_features()
            .unwrap()
            .supports("codex", ReaderFeature::Turns)
    );
    assert!(
        device
            .link
            .reader_features()
            .unwrap()
            .supports("pi", ReaderFeature::Conversation)
    );
    assert!(
        !device
            .link
            .reader_features()
            .unwrap()
            .supports("pi", ReaderFeature::Turns)
    );
    for agent in ["omp", "grok"] {
        for feature in [
            ReaderFeature::Conversation,
            ReaderFeature::Turns,
            ReaderFeature::UserTurnContent,
        ] {
            assert!(
                device
                    .link
                    .reader_features()
                    .unwrap()
                    .supports(agent, feature),
                "{agent}: {feature:?}"
            );
        }
    }
    // Cursor's accepted reader contract includes its conversation, while
    // its native records provide no title, Memory, link or structured turn.
    let features = device.link.reader_features().unwrap();
    for feature in [
        ReaderFeature::Identity,
        ReaderFeature::Labels,
        ReaderFeature::Conversation,
        ReaderFeature::Activity,
    ] {
        assert!(features.supports("cursor", feature), "cursor: {feature:?}");
    }
    for feature in [
        ReaderFeature::Titles,
        ReaderFeature::Memory,
        ReaderFeature::Links,
        ReaderFeature::Turns,
        ReaderFeature::UserTurnContent,
    ] {
        assert!(!features.supports("cursor", feature), "cursor: {feature:?}");
    }
}

// This test-only input names a retained release payload directory. It is
// deliberately mandatory for this opt-in historical-binary lane and never
// read by the product or used to fabricate a protocol number.
const RETAINED_PROTOCOL24_PACKAGES: &str = "HIDE_TEST_PROTOCOL24_PACKAGES";

#[test]
#[ignore = "requires an actual retained protocol-24 payload in HIDE_TEST_PROTOCOL24_PACKAGES"]
fn a_retained_protocol24_node_keeps_legacy_readers_then_upgrades_normally() {
    use hide_node_link::sessions::ReaderFeature;
    use hide_session::Agent;
    use hide_session::label_transcript::{LabelTranscript, LabelTranscriptRequest};

    let retained = PathBuf::from(
        std::env::var_os(RETAINED_PROTOCOL24_PACKAGES)
            .expect("supply the actual accepted protocol-24 payload directory"),
    );
    let bytes = fs::read(retained.join("hided")).unwrap();
    assert!(
        !bytes.starts_with(b"#!"),
        "retained helper must be the real binary"
    );
    let mut device = Device::start_with(
        None,
        Some(Arc::new(HeardTerminals::default())),
        Some(retained),
    );
    assert_eq!(device.hello.protocol, 24);
    assert!(device.hello.reader_features.is_none());
    // The audited 24 node keeps its legacy features only: it has no terminal
    // service, so it is never asked to start one.
    assert!(device.terminals.is_none());
    let old_helper = device.helper_path.clone();

    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../hide-session/tests/fixtures/adapters");
    fn copy_fixture(source: &Path, target: &Path) {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let destination = target.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_fixture(&entry.path(), &destination);
            } else {
                fs::copy(entry.path(), destination).unwrap();
            }
        }
    }
    copy_fixture(
        &fixtures.join("claude-2.1.288"),
        &device.home.join(".claude"),
    );
    copy_fixture(&fixtures.join("codex-0.160.0"), &device.home.join(".codex"));
    let opencode = device.home.join(".local/share/opencode");
    fs::create_dir_all(&opencode).unwrap();
    run(Command::new("sqlite3")
        .arg(opencode.join("opencode.db"))
        .stdin(fs::File::open(fixtures.join("opencode-1.18.30/opencode.sql")).unwrap()));
    for (agent, id) in [
        (Agent::Claude, "a1b2c3d4-0000-4000-8000-000000000001"),
        (Agent::Codex, "0199a000-0000-7000-8000-000000000002"),
        (Agent::OpenCode, "ses_0a1b2c3d4e5f60718293a4b5c6"),
    ] {
        let transcript: LabelTranscript = call_as(
            device.link.as_ref(),
            Call::LabelTranscript {
                request: LabelTranscriptRequest {
                    agent,
                    reference_kind: "id".to_owned(),
                    reference_value: id.to_owned(),
                    cwd: Some("/work/app".to_owned()),
                    checkpoint: None,
                    subagents: Default::default(),
                    turns: None,
                    tools: None,
                },
            },
            TIMEOUT,
        )
        .unwrap();
        assert!(
            transcript
                .events
                .iter()
                .any(|event| event.text.contains("요청 보기를 만들어줘")),
            "{} legacy conversation",
            agent.as_str()
        );
        assert!(transcript.title.is_some());
        if agent == Agent::OpenCode {
            assert!(
                !device
                    .link
                    .reader_features()
                    .unwrap()
                    .supports("opencode", ReaderFeature::Conversation)
            );
            assert!(transcript.turns.is_none());
        }
    }
    let listing =
        hide_node_link::readers::link_files(device.link.as_ref(), 0, None, TIMEOUT).unwrap();
    assert_eq!(listing.refused, 0);
    for agent in [Agent::Claude, Agent::Codex, Agent::OpenCode] {
        let candidate = listing
            .rows
            .iter()
            .find(|row| row.agent == agent)
            .expect("legacy native session was listed");
        let batch = hide_node_link::readers::links(
            device.link.as_ref(),
            vec![hide_session::links::ReadRequest {
                agent,
                path: candidate.path.clone(),
                checkpoint: None,
            }],
            TIMEOUT,
        )
        .unwrap();
        assert!(batch.refused.is_empty());
        assert!(batch.answers[0].facts.session_id.is_some());
        if agent.has_session_file() {
            let stat: hide_node_link::sessions::SessionStat = hide_node_link::link::call_as_reader(
                device.link.as_ref(),
                agent,
                ReaderFeature::Memory,
                Call::SessionStat {
                    scope: None,
                    path: candidate.path.clone(),
                },
                TIMEOUT,
            )
            .unwrap();
            assert!(stat.size > 0);
            let text: String = hide_node_link::link::call_as_reader(
                device.link.as_ref(),
                agent,
                ReaderFeature::Conversation,
                Call::SessionText {
                    scope: None,
                    path: candidate.path.clone(),
                },
                TIMEOUT,
            )
            .unwrap();
            assert!(text.contains("요청 보기를 만들어줘"));
        }
    }
    for provider in ["pi", "omp", "grok", "cursor"] {
        assert!(
            !device
                .link
                .reader_features()
                .unwrap()
                .supports(provider, ReaderFeature::Identity)
        );
    }
    let refused: Result<LabelTranscript, _> = call_as(
        device.link.as_ref(),
        Call::LabelTranscript {
            request: LabelTranscriptRequest {
                agent: Agent::Omp,
                reference_kind: "id".into(),
                reference_value: "native-omp".into(),
                cwd: Some("/work/app".into()),
                checkpoint: None,
                subagents: Default::default(),
                turns: None,
                tools: None,
            },
        },
        TIMEOUT,
    );
    assert!(
        refused.is_err(),
        "an actual protocol24 helper cannot read omp"
    );
    let project: RootOpened = call_as(
        device.link.as_ref(),
        Call::RootOpen {
            root: device.project.to_str().unwrap().to_owned(),
        },
        TIMEOUT,
    )
    .unwrap();
    let expected: RootOpened = call_as(
        &hide_node::Local::new(Some(device.home.clone())),
        Call::RootOpen {
            root: device.project.to_str().unwrap().to_owned(),
        },
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(project.identity, expected.identity);
    device.link.close("ordinary build upgrade");
    let transport = hide_node::ssh::Connector::new(Some(device.current_packages.clone()))
        .with_askpass_program(PathBuf::from(env!("CARGO_BIN_EXE_hide")))
        .transport(&device.local, "contract-node", ALIAS, None)
        .unwrap();
    let consent = HostConsent {
        contract: HOST_CONSENT_CONTRACT,
        helper_root: "~/helper".to_owned(),
        cli_dir: None,
        granted_at_unix_ms: 1,
        identity: None,
    };
    let established = transport
        .establish(
            &consent,
            &[],
            hide_node_link::device::Attempt::unattended(
                hide_node_link::device::Wake::Allowed,
                Box::new(|_| {}),
            ),
        )
        .unwrap();
    assert_eq!(
        established.node.hello.protocol,
        hide_node_link::protocol::PROTOCOL_VERSION
    );
    assert!(established.installed);
    assert_ne!(established.helper_path, old_helper);
    assert_eq!(
        established.node.host.reader_features(),
        Some(&hide_node_link::sessions::ReaderFeatures::implemented())
    );
    assert!(
        established
            .node
            .host
            .reader_features()
            .unwrap()
            .supports("omp", ReaderFeature::UserTurnContent)
    );
    device.link = established.node.host;
    device.link.close("retained-payload fixture ended");
}

/// Records only completed production callbacks, so a gate can hold their
/// work without relying on when the executor or node happens to run it.
struct ProductionEvents {
    panes: hided::node_panes::Events,
    counts: std::sync::Mutex<CallbackCounts>,
    changed: std::sync::Condvar,
}

#[derive(Default)]
struct CallbackCounts {
    proofs: usize,
    closed: usize,
}

impl ProductionEvents {
    fn wait(&self, expected: usize, count: impl Fn(&CallbackCounts) -> usize) {
        let value = self.counts.lock().unwrap();
        let (value, _) = self
            .changed
            .wait_timeout_while(value, TIMEOUT, |value| count(value) < expected)
            .unwrap();
        assert_eq!(count(&value), expected);
    }
}

impl hide_node::ssh::PaneEvents for ProductionEvents {
    fn event(&self, node: &str, link: &hide_node::ssh::RemoteHost, event: NodeEvent) {
        let proof = matches!(&event, NodeEvent::PaneProof { .. });
        self.panes.event(node, link, event);
        if proof {
            self.counts.lock().unwrap().proofs += 1;
            self.changed.notify_all();
        }
    }

    fn closed(&self, node: &str, link: &hide_node::ssh::RemoteHost) {
        self.panes.closed(node, link);
        self.counts.lock().unwrap().closed += 1;
        self.changed.notify_all();
    }
}

/// A failed assertion must release the executor before its runtime drops.
struct ReleaseOnDrop(Option<std::sync::mpsc::Sender<()>>);

impl ReleaseOnDrop {
    fn release(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.release();
    }
}

/// The production overflow callback answers the actual node caller with
/// bridge_busy, keeps its reader usable, and reaches normal close cleanup.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_saturated_core_answers_busy_over_ssh_and_keeps_its_reader_usable() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (started, ready) = std::sync::mpsc::channel();
    let (release, held) = std::sync::mpsc::channel();
    let worker = runtime.spawn_blocking(move || {
        started.send(()).unwrap();
        let _ = held.recv();
    });
    let mut gate = ReleaseOnDrop(Some(release));
    ready.recv_timeout(TIMEOUT).unwrap();
    let events = Arc::new(ProductionEvents {
        panes: hided::node_panes::Events(Arc::new(hided::node_panes::NodePanes::new(
            runtime.handle().clone(),
        ))),
        counts: std::sync::Mutex::default(),
        changed: std::sync::Condvar::new(),
    });
    let device = Device::start_with_pane_events(Some(events.clone()));
    let consent = HostConsent {
        contract: HOST_CONSENT_CONTRACT,
        helper_root: "~/helper".to_owned(),
        cli_dir: None,
        granted_at_unix_ms: 1,
        identity: None,
    };
    let mut links = vec![Arc::clone(&device.link)];
    for _ in 0..2 {
        links.push(
            device
                .transport
                .establish(
                    &consent,
                    &[],
                    hide_node_link::device::Attempt::unattended(
                        hide_node_link::device::Wake::Allowed,
                        Box::new(|_| {}),
                    ),
                )
                .unwrap()
                .node
                .host,
        );
    }
    let herdr_socket = device.state.parent().unwrap().join("s.sock");
    let sockets = links
        .iter()
        .map(|link| {
            call_as::<hide_node_link::panes::PanesStarted>(
                link.as_ref(),
                Call::PanesStart {
                    herdr_socket: herdr_socket.to_str().unwrap().to_owned(),
                },
                TIMEOUT,
            )
            .unwrap()
            .socket
        })
        .collect::<Vec<_>>();
    let request = |socket: &str, nonce: usize| {
        let mut stream = UnixStream::connect(socket).unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        writeln!(
            stream,
            "{}",
            json!({"pane_id": PROVED_PANE, "nonce": format!("{nonce:032x}")})
        )
        .unwrap();
        BufReader::new(stream)
    };
    let mut waiting = Vec::new();
    for socket in &sockets[..2] {
        for nonce in 0..16 {
            waiting.push(request(socket, nonce));
        }
    }
    events.wait(32, |counts| counts.proofs);
    let mut overflow = request(&sockets[2], 32);
    events.wait(33, |counts| counts.proofs);
    gate.release();
    runtime.block_on(worker).unwrap();
    let read = |reader: &mut BufReader<UnixStream>| {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };
    assert_eq!(
        read(&mut overflow),
        json!({"ok": false, "reason": "bridge_busy", "next_action": "Reconnect the pane and retry"})
    );
    for reader in &mut waiting {
        assert_eq!(
            read(reader),
            json!({"ok": false, "reason": "hide_unavailable", "next_action": "Reconnect the pane and retry"})
        );
    }
    assert!(links[2].call(Call::Hello, TIMEOUT).is_ok());
    assert_eq!(
        read(&mut request(&sockets[2], 33)),
        json!({"ok": false, "reason": "hide_unavailable", "next_action": "Reconnect the pane and retry"})
    );
    // Each attempt is one connection of its own, and nothing else dialed.
    assert_eq!(device.ssh.accepted(), 3);
    for link in &links {
        link.close("contract");
    }
    events.wait(3, |counts| counts.closed);
    for socket in &sockets {
        gone_within(Path::new(socket), TIMEOUT);
        assert!(!Path::new(socket).exists());
    }
}

/// One request to the fixture Herdr, answered only for [`PROVED_PANE`].
fn answer_herdr(connection: std::os::unix::net::UnixStream) {
    use std::io::{BufRead, BufReader, Write};
    let mut line = String::new();
    if BufReader::new(&connection).read_line(&mut line).is_err() {
        return;
    }
    let Ok(request) = serde_json::from_str::<Value>(&line) else {
        return;
    };
    let pane = if request["method"] == "pane.current" {
        &request["params"]["caller_pane_id"]
    } else {
        &request["params"]["pane_id"]
    };
    if pane != PROVED_PANE {
        return;
    }
    let result = match request["method"].as_str() {
        Some("pane.current") => json!({"pane": {"pane_id": request["params"]["caller_pane_id"]}}),
        Some("pane.process_info") => json!({"process_info": {"shell_pid": std::process::id()}}),
        Some("pane.get") => json!({"pane": {"terminal_id": "t-proved"}}),
        _ => return,
    };
    let _ = writeln!(
        &connection,
        "{}",
        json!({"id": request["id"], "result": result})
    );
}

fn executable(path: &Path, contents: &str) {
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// The answers a node gives for one checkout: its files, a file's bytes, its
/// Git changes and worktrees, and the account's hook state.
fn contract(link: &dyn NodeLink, project: &Path) -> Vec<Value> {
    let path = project.to_str().unwrap().to_owned();
    let opened: RootOpened = call_as(link, Call::RootOpen { root: path.clone() }, TIMEOUT).unwrap();
    let root = RootRef {
        path: path.clone(),
        identity: opened.identity,
    };
    let calls = [
        Call::List {
            root: root.clone(),
            path: String::new(),
        },
        Call::Bytes {
            root: root.clone(),
            path: "a.txt".to_owned(),
            offset: 0,
            length: 64,
        },
        Call::Changes {
            root,
            scope: String::new(),
            selected: Some("a.txt".to_owned()),
            committed: false,
            base: None,
            diffs: Vec::new(),
        },
        Call::Worktrees {
            path,
            bases: BTreeMap::new(),
            base_override: None,
            remote_default: None,
            reuse_unchanged: false,
        },
        Call::HookDiagnosis,
    ];
    calls
        .into_iter()
        .map(|call| {
            let name = format!("{call:?}");
            let mut answer = call_as::<Value>(link, call, TIMEOUT)
                .unwrap_or_else(|error| panic!("{name}: {error:?}"));
            unbound(&mut answer);
            answer
        })
        .collect()
}

/// Drops what an answer reads from outside the node: when it was measured,
/// and which agent CLIs the answering process's own `PATH` finds.
fn unbound(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.remove("measured_at_unix_ms");
            object.remove("memory_compatibility");
            object.values_mut().for_each(unbound);
        }
        Value::Array(items) => items.iter_mut().for_each(unbound),
        _ => {}
    }
}

#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn the_core_s_own_node_and_a_device_link_answer_alike() {
    let device = Device::start();
    let local = hide_node::Local::new(Some(device.home.clone()));
    let here = contract(&local, &device.project);
    let there = contract(&*device.link, &device.project);
    assert_eq!(here, there);
    // What the checkout holds, as written above, not as either node said.
    let listed = here[0].to_string();
    assert!(
        listed.contains("\"a.txt\"") && listed.contains("\"b.txt\""),
        "{listed}"
    );
    let bytes = here[1].to_string();
    assert!(
        bytes.contains("dHdvCg==") || bytes.contains("two\\n"),
        "{bytes}"
    );
    let changes = here[2].to_string();
    assert!(
        changes.contains("a.txt") && changes.contains("-one") && changes.contains("+two"),
        "{changes}"
    );
}

#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_published_guardrail_policy_is_what_the_devices_hooks_read() {
    use hide_agent_hooks::guardrails::{Policy, ProjectPolicy, Rules, read_policy};
    let device = Device::start();
    let policy = Policy::new(vec![ProjectPolicy {
        root: device.project.to_str().unwrap().to_owned(),
        base: Some("main".to_owned()),
        rules: Rules {
            files: true,
            ..Rules::default()
        },
    }]);
    let answer: Value = call_as(
        &*device.link,
        Call::GuardrailPolicy {
            policy: policy.clone(),
            sequence: 10,
        },
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(answer, Value::Null);
    assert_eq!(read_policy(&device.home), Ok(policy.clone()));
    // A later publication replaces it whole.
    let none = Policy::new(Vec::new());
    let _: Value = call_as(
        &*device.link,
        Call::GuardrailPolicy {
            policy: none.clone(),
            sequence: 11,
        },
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(read_policy(&device.home), Ok(none.clone()));
    // A write that outlived its caller's wait does not land over a newer one,
    // and says which sequence the node holds.
    let late: Value = call_as(
        &*device.link,
        Call::GuardrailPolicy {
            policy,
            sequence: 10,
        },
        TIMEOUT,
    )
    .unwrap();
    assert_eq!(late, serde_json::json!({ "superseded_by": 11 }));
    assert_eq!(read_policy(&device.home), Ok(none));
}

#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_device_s_channels_share_its_one_connection() {
    let device = Device::start();
    // The start script, the install and the link made one connection.
    assert_eq!(device.ssh.accepted(), 1);
    // The device's Herdr is a stream of it, and an attachment rides it too.
    device.transport.herdr_api_connector().connect().unwrap();
    let request = "8a6e0e4c-3f0b-4d2c-9a51-0c6f2b9d7e10";
    let staged = stage(&device, request, 0, 1, "attached.txt").unwrap();
    assert!(
        Path::new(&staged).starts_with(device.state.join("terminal-attachments")),
        "staged at {staged}"
    );
    assert_eq!(fs::read(&staged).unwrap(), b"attached\n");
    let _: Value = call_as(
        &*device.link,
        Call::RemoveAttachments {
            request: request.to_owned(),
        },
        TIMEOUT,
    )
    .unwrap();
    assert!(!Path::new(&staged).exists());
    assert_eq!(device.ssh.accepted(), 1);
    // The link is the connection: closing it ends the device's Herdr too,
    // and nothing dials again on its own.
    device.link.close("contract");
    assert!(device.transport.herdr_api_connector().connect().is_err());
    assert_eq!(device.ssh.accepted(), 1);
}

/// B10: a request the link ends part way leaves nothing behind.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_link_ending_mid_request_removes_what_it_staged() {
    let device = Device::start();
    let request = "0d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
    let first = stage(&device, request, 0, 2, "first.txt").unwrap();
    assert!(Path::new(&first).exists());
    // A retried file is the same answer, and an index past its count is refused.
    assert_eq!(stage(&device, request, 0, 2, "first.txt").unwrap(), first);
    assert!(stage(&device, request, 2, 2, "second.txt").is_err());
    device.link.close("contract");
    gone_within(Path::new(&first), TIMEOUT);
    assert!(!Path::new(&first).exists(), "the ended link left {first}");
}

/// B10, the journey: a link that ends while the device writes a request's
/// first file leaves nothing behind. Which comes first, the end or the
/// file, is ordered step by step in `hide-host`'s staging tests.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_link_ending_during_a_write_leaves_no_file() {
    let device = Device::start();
    let request = "5b0c9d8e-7f6a-4b5c-8d4e-3f2a1b0c9d8e";
    let staged =
        hide_kit::layout::terminal_attachments(&device.state).join(format!("hide-{request}-0.bin"));
    // The largest file takes the longest to write, which the link's end
    // has to land within.
    let size = hide_node_link::attachments::MAX_FILE_BYTES as usize;
    let link = Arc::clone(&device.link);
    let writer = std::thread::spawn(move || {
        call_as::<String>(
            &*link,
            Call::StageAttachment {
                request: request.to_owned(),
                index: 0,
                count: 2,
                file: hide_node_link::feedback::File::new("big.bin", &vec![1; size]),
            },
            TIMEOUT,
        )
    });
    begun_within(&staged, &|| writer.is_finished(), TIMEOUT);
    assert!(
        staged.exists(),
        "the device began no file: {:?}",
        writer.join()
    );
    device.link.close("contract");
    let _ = writer.join();
    gone_within(&staged, TIMEOUT);
    assert!(!staged.exists(), "the ended link left {}", staged.display());
}

/// B12: a device's node answers the Hide AI calls on its own link, with a
/// backend of its own built for the core's instance and released by it,
/// where a device used to refuse them. No provider is run.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_device_answers_hide_ai_for_its_own_agents() {
    let device = Device::start();
    let backend = hide_node_link::ai::BackendSpec {
        instance: 7,
        provider: hide_node_link::ai::ProviderId::CODEX,
        model: String::new(),
    };
    let measured: hide_node_link::ai::ProcessMeasurement = call_as(
        &*device.link,
        Call::AiMeasurement {
            backend: backend.clone(),
        },
        TIMEOUT,
    )
    .expect("the device answers a backend's measurement");
    assert_eq!(
        measured,
        hide_node_link::ai::ProcessMeasurement::Unavailable
    );
    let _: Value = call_as(&*device.link, Call::AiRelease { instance: 7 }, TIMEOUT)
        .expect("the device releases the backend");
    // GitHub stays the core's (D-18): the device still refuses it.
    let gh = call_as::<Value>(
        &*device.link,
        Call::Gh {
            args: vec!["auth".to_owned(), "status".to_owned()],
            cwd: None,
            repository: None,
        },
        TIMEOUT,
    );
    assert!(gh.is_err(), "a device answered gh: {gh:?}");
}

/// B9: a device's localhost page rides its node link, whichever link that
/// is: a connection to the page's forward reaches the device's loopback
/// server, dropping the forward ends that server's connection, and a
/// forward whose link ended reaches nothing.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_device_page_rides_its_node_link_and_ends_with_it() {
    use hided::page_forward::PageForward;
    use std::io::Write;
    let device = Device::start();
    let server = PageServer::start();
    let link = || {
        device
            .link
            .as_any()
            .and_then(|link| link.downcast_ref::<hide_node::ssh::RemoteHost>())
            .expect("a device link carries streams")
            .clone()
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let forward = runtime
        .block_on(PageForward::start(link(), server.address, None, false))
        .unwrap();
    let mut page = std::net::TcpStream::connect(forward.local_addr()).unwrap();
    page.set_read_timeout(Some(TIMEOUT)).unwrap();
    page.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();
    assert!(read_page(&mut page).ends_with("hello"));
    drop(forward);
    server
        .ended
        .recv_timeout(TIMEOUT)
        .expect("the node kept the page's connection after its forward ended");

    let forward = runtime
        .block_on(PageForward::start(link(), server.address, None, false))
        .unwrap();
    device.link.close("contract");
    assert!(!forward.live());
    let mut page = std::net::TcpStream::connect(forward.local_addr()).unwrap();
    page.set_read_timeout(Some(TIMEOUT)).unwrap();
    let _ = page.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
    assert_eq!(
        read_page(&mut page),
        "",
        "an ended link still reached the page"
    );
}

/// A device's loopback server for one page: it answers each request
/// `hello` and tells `ended` when a connection to it closes.
struct PageServer {
    address: std::net::SocketAddr,
    ended: std::sync::mpsc::Receiver<()>,
}

impl PageServer {
    fn start() -> Self {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (closed, ended) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut connection) = connection else {
                    break;
                };
                let closed = closed.clone();
                std::thread::spawn(move || {
                    let mut request = Vec::new();
                    let mut chunk = [0_u8; 1024];
                    while let Ok(read) = connection.read(&mut chunk) {
                        if read == 0 {
                            break;
                        }
                        request.extend_from_slice(&chunk[..read]);
                        if request.windows(4).any(|end| end == b"\r\n\r\n") {
                            request.clear();
                            let answer = b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello";
                            if connection.write_all(answer).is_err() {
                                break;
                            }
                        }
                    }
                    let _ = closed.send(());
                });
            }
        });
        Self { address, ended }
    }
}

/// What a page's connection answered, up to its `hello` or its end.
fn read_page(page: &mut std::net::TcpStream) -> String {
    use std::io::Read;
    let mut answer = Vec::new();
    let mut chunk = [0_u8; 1024];
    while !answer.ends_with(b"hello") {
        match page.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => answer.extend_from_slice(&chunk[..read]),
        }
    }
    String::from_utf8_lossy(&answer).into_owned()
}

/// Stages one file of `count` through the device's node link.
fn stage(
    device: &Device,
    request: &str,
    index: usize,
    count: usize,
    name: &str,
) -> Result<String, hide_node_link::LinkError> {
    call_as(
        &*device.link,
        Call::StageAttachment {
            request: request.to_owned(),
            index,
            count,
            file: hide_node_link::feedback::File::new(name, b"attached\n"),
        },
        TIMEOUT,
    )
}

/// What a device's terminals sent up its link: each pane's reports.
#[derive(Default)]
struct HeardTerminals {
    reports: std::sync::Mutex<Vec<hide_node_link::terminal::TerminalReport>>,
    arrived: std::sync::Condvar,
}

impl hide_node::terminal::device::DeviceSink for HeardTerminals {
    fn output(&self, _device: &str, _pane: &str, _bytes: &[u8], _full: bool) {}

    fn report(&self, _device: &str, report: hide_node_link::terminal::TerminalReport) {
        self.reports.lock().unwrap().push(report);
        self.arrived.notify_all();
    }
}

/// B16: a device's panes' terminals ride its one connection. Attaching
/// several of its panes opens no connection of their own, and each pane's
/// word comes back up the same link.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_device_s_terminals_ride_its_one_connection() {
    use hide_node_link::terminal::{GridSize, TerminalControl, TerminalReport};
    let heard = Arc::new(HeardTerminals::default());
    let device = Device::start_with(None, Some(heard.clone()), None);
    let terminals = device
        .terminals
        .clone()
        .expect("the device's node runs a terminal service");
    let panes = (1..=5)
        .map(|index| format!("w1:p{index}"))
        .collect::<Vec<_>>();
    for pane in &panes {
        terminals.control(TerminalControl::Attach {
            pane: pane.clone(),
            size: Some(GridSize { rows: 24, cols: 80 }),
            manual: false,
        });
    }
    let reports = heard.reports.lock().unwrap();
    let (reports, _) = heard
        .arrived
        .wait_timeout_while(reports, TIMEOUT, |reports| {
            !panes.iter().all(|pane| {
                reports.iter().any(|report| {
                    matches!(report, TerminalReport::State { pane: reported, .. } if reported == pane)
                })
            })
        })
        .unwrap();
    for pane in &panes {
        assert!(
            reports.iter().any(|report| {
                matches!(report, TerminalReport::State { pane: reported, .. } if reported == pane)
            }),
            "{pane} reported nothing: {reports:?}"
        );
    }
    drop(reports);
    assert_eq!(device.ssh.accepted(), 1);
}

/// A device removed while a call is still out drops its own hold on the
/// connection; the link keeps the connection it runs on, so the call (the
/// kit coming off) still gets its answer.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_link_outlives_its_device_s_hold_on_the_connection() {
    let Device {
        _root,
        link,
        transport,
        ssh,
        ..
    } = Device::start();
    drop(transport);
    let answer = link.call(Call::Hello, Duration::from_secs(10));
    assert!(answer.is_ok(), "{answer:?}");
    assert_eq!(ssh.accepted(), 1);
}

/// A reporting call crosses the link both ways: the device's Git watch
/// reports up as it runs, a report answered with false stops it on the
/// device, and the call ends with the watch's own answer.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_device_s_reporting_call_reports_and_stops_when_told() {
    use hide_node_link::worktrees::GitWatchReport;
    let device = Device::start();
    let common = device.project.join("watched");
    fs::create_dir_all(common.join("refs/heads")).unwrap();
    let mut reports = Vec::new();
    let mut wrote = false;
    let answer = device.link.call_with_progress(
        Call::GitWatch {
            common_dirs: vec![common.to_string_lossy().into_owned()],
        },
        Duration::from_secs(30),
        &mut |report| {
            let report: GitWatchReport = serde_json::from_value(report).unwrap();
            if !wrote {
                fs::write(common.join("refs/heads/main"), "0000\n").unwrap();
                wrote = true;
            }
            let changed = matches!(report, GitWatchReport::Changed { .. });
            reports.push(report);
            !changed
        },
    );
    assert!(answer.is_ok(), "{answer:?}");
    assert!(
        matches!(reports.first(), Some(GitWatchReport::Watching { .. })),
        "{reports:?}"
    );
    assert!(
        matches!(reports.last(), Some(GitWatchReport::Changed { .. })),
        "{reports:?}"
    );
    // The link still answers after the stopped call.
    assert!(
        device
            .link
            .call(Call::Hello, Duration::from_secs(10))
            .is_ok()
    );
}

/// A draining link (consent withdrawn) stops the reporting call it would
/// otherwise wait out for its whole bound, and closes once it has.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_draining_link_stops_its_reporting_call_and_closes() {
    let device = Device::start();
    let socket = pane_sockets(&device.state).pop().unwrap();
    let common = device.project.join("watched");
    fs::create_dir_all(common.join("refs/heads")).unwrap();
    let (watching, watched) = std::sync::mpsc::channel();
    let link = Arc::clone(&device.link);
    let watch = std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let answer = link.call_with_progress(
            Call::GitWatch {
                common_dirs: vec![common.to_string_lossy().into_owned()],
            },
            Duration::from_secs(120),
            &mut |_| {
                let _ = watching.send(());
                true
            },
        );
        (answer.is_ok(), started.elapsed())
    });
    watched.recv_timeout(TIMEOUT).unwrap();
    device.link.close_when_idle("consent withdrawn");
    let (_, took) = watch.join().unwrap();
    assert!(took < Duration::from_secs(30), "the watch ran {took:?}");
    gone_within(&socket, TIMEOUT);
    assert!(device.link.closed_reason().is_some());
    assert!(!socket.exists(), "the drained link kept its pane socket");
}

/// Waits until the node began writing `path`, or until `answered` says its
/// call is over without it.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn begun_within(path: &Path, answered: &dyn Fn() -> bool, bound: Duration) {
    let deadline = std::time::Instant::now() + bound;
    while !path.exists() && !answered() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Waits until the node removed `path`, which it does once its input ends.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn gone_within(path: &Path, bound: Duration) {
    let deadline = std::time::Instant::now() + bound;
    while path.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The device's pane sockets: one `bridge-*` folder per daemon's link, each
/// holding the socket a device pane's `hide` asks.
fn pane_sockets(state: &Path) -> Vec<PathBuf> {
    match fs::read_dir(state.join("workspace-bridges")) {
        Ok(entries) => entries
            .map(|entry| entry.unwrap().path().join("bootstrap.sock"))
            .filter(|socket| socket.exists())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// B18 and letter-720: the node's pane socket is the account's alone, a
/// caller the node cannot prove is refused with today's reason, and the
/// socket lives exactly as long as the link it answers for.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_device_s_pane_socket_lives_with_its_link() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let device = Device::start();
    let sockets = pane_sockets(&device.state);
    assert_eq!(sockets.len(), 1, "{sockets:?}");
    let socket = &sockets[0];
    let mode = |path: &Path| fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(socket.parent().unwrap()), 0o700);
    assert_eq!(mode(&device.state.join("workspace-bridges")), 0o700);
    assert_eq!(mode(socket) & 0o077, 0, "{:o}", mode(socket));
    // This test process descends from no pane of the device's Herdr.
    let mut stream = UnixStream::connect(socket).unwrap();
    writeln!(
        stream,
        "{}",
        json!({"pane_id": "w1:p1", "nonce": "0".repeat(32)})
    )
    .unwrap();
    let mut answer = String::new();
    BufReader::new(&stream).read_line(&mut answer).unwrap();
    let answer: Value = serde_json::from_str(&answer).unwrap();
    assert_eq!(
        answer,
        json!({"ok": false, "reason": "pane_unavailable", "next_action": "Reconnect the pane and retry"})
    );
    device.link.close("contract");
    gone_within(socket, TIMEOUT);
    assert!(
        !socket.exists(),
        "the node kept its socket after its link closed"
    );
    assert_eq!(*device.events.closed.lock().unwrap(), ["contract-node"]);
}

/// One credential request on the device's pane socket and the line it answers.
fn ask(socket: &Path, pane_id: &str, nonce: char, one_shot: bool) -> Value {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let mut stream = UnixStream::connect(socket).unwrap();
    let request =
        json!({"pane_id": pane_id, "nonce": nonce.to_string().repeat(32), "one_shot": one_shot});
    writeln!(stream, "{request}").unwrap();
    let mut answer = String::new();
    BufReader::new(&stream).read_line(&mut answer).unwrap();
    serde_json::from_str(&answer).unwrap()
}

/// B18, B19 and B30 over a real link: a caller the device's node proves
/// reaches the core with the pane's identity and gets the credential the
/// core issued; a refusal the core answers reaches the caller byte for
/// byte; a refusal the node makes itself, at its caller cap too, is
/// reported up the link for the core's record; and closing the link takes
/// the credential's reference with it.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_device_pane_s_proof_crosses_the_link_and_its_refusals_are_reported() {
    use std::os::unix::net::UnixStream;
    let device = Device::start();
    let sockets = pane_sockets(&device.state);
    assert_eq!(sockets.len(), 1, "{sockets:?}");
    let socket = &sockets[0];

    let issued = ask(socket, PROVED_PANE, 'a', false);
    assert_eq!(issued["ok"], true, "{issued}");
    let reference = PathBuf::from(issued["reference"].as_str().unwrap());
    let written: Value = serde_json::from_slice(&fs::read(&reference).unwrap()).unwrap();
    assert_eq!(written["token"], TOKEN);
    assert_eq!(written["socket"], socket.to_str().unwrap());
    let proofs = device.events.heard.lock().unwrap().clone();
    let shell = i32::try_from(std::process::id()).unwrap();
    assert!(
        matches!(
            &proofs[..],
            [NodeEvent::PaneProof { pane_id, identity, .. }]
                if pane_id == PROVED_PANE
                    && identity.terminal_id == "t-proved"
                    && identity.shell_pid == shell
        ),
        "{proofs:?}"
    );

    // Refused by the core: the caller reads its reason, and the node
    // reports nothing, since the core records what it refused.
    let changed = ask(socket, PROVED_PANE, 'b', true);
    assert_eq!(
        changed,
        json!({"ok": false, "reason": "pane_changed", "next_action": "Reconnect the pane and retry"})
    );
    // Refused by the node: a pane its Herdr does not know.
    let unknown = ask(socket, "w1:p1", 'c', false);
    assert_eq!(
        unknown,
        json!({"ok": false, "reason": "pane_unavailable", "next_action": "Reconnect the pane and retry"})
    );
    // At the cap: sixteen callers that never ask hold every place.
    let waiting = (0..16)
        .map(|_| UnixStream::connect(socket).unwrap())
        .collect::<Vec<_>>();
    let busy = ask(socket, PROVED_PANE, 'd', false);
    assert_eq!(
        busy,
        json!({"ok": false, "reason": "bridge_busy", "next_action": "Reconnect the pane and retry"})
    );
    drop(waiting);
    let refusals = |heard: &[NodeEvent]| {
        heard
            .iter()
            .filter_map(|event| match event {
                NodeEvent::Refused { pane_id, reason } => Some((pane_id.clone(), reason.clone())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let heard = device
        .events
        .heard_until(|heard| refusals(heard).len() >= 2);
    assert_eq!(
        refusals(&heard)[..2],
        [
            (Some("w1:p1".to_owned()), "pane_unavailable".to_owned()),
            (None, "bridge_busy".to_owned()),
        ]
    );

    device.link.close("contract");
    gone_within(&reference, TIMEOUT);
    assert!(!reference.exists(), "a reference outlived its link");
}

/// Pane events that issue a credential for a one-shot proof too, as the core
/// does for a pane it resolves, and hear everything else as [`Events`].
#[derive(Default)]
struct IssuingEvents(Events);

/// The one-shot credential [`IssuingEvents`] issues.
const ONE_SHOT_TOKEN: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

impl hide_node::ssh::PaneEvents for IssuingEvents {
    fn event(&self, node: &str, link: &hide_node::ssh::RemoteHost, event: NodeEvent) {
        let NodeEvent::PaneProof {
            request,
            one_shot: true,
            ..
        } = &event
        else {
            return self.0.event(node, link, event);
        };
        let (link, request) = (link.clone(), *request);
        std::thread::spawn(move || {
            let answer = ProofAnswer::Issued {
                token: ONE_SHOT_TOKEN.to_owned(),
                issued_new: true,
            };
            link.call(Call::PaneProofAnswer { request, answer }, TIMEOUT)
                .unwrap();
        });
        self.0.heard.lock().unwrap().push(event);
        self.0.arrived.notify_all();
    }

    fn closed(&self, node: &str, link: &hide_node::ssh::RemoteHost) {
        self.0.closed(node, link);
    }
}

/// A stream opens naming its caller when the caller holds a one-shot
/// credential the device's node issued for a pane, with that pane's
/// identity as the node reads it then, so the core need not ask the node
/// again. A lasting credential names its token in the local prelude, but
/// the node still proves the peer against the pane it issued that token for.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_stream_opened_by_a_one_shot_holder_names_its_pane() {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    let events = Arc::new(IssuingEvents::default());
    let device = Device::start_with_pane_events(Some(events.clone()));
    let sockets = pane_sockets(&device.state);
    assert_eq!(sockets.len(), 1, "{sockets:?}");
    let socket = &sockets[0];
    let open = |count: usize, token: Option<&str>| {
        let mut stream = UnixStream::connect(socket).unwrap();
        writeln!(stream, "{}", json!({"stream": true, "token": token})).unwrap();
        let heard = events.0.heard_until(|heard| {
            heard
                .iter()
                .filter(|event| matches!(event, NodeEvent::StreamOpen { .. }))
                .count()
                >= count
        });
        let opened = heard
            .into_iter()
            .filter_map(|event| match event {
                NodeEvent::StreamOpen { caller, .. } => Some(caller),
                _ => None,
            })
            .nth(count - 1)
            .expect("the stream opened");
        drop(stream);
        opened
    };

    assert_eq!(ask(socket, PROVED_PANE, 'a', false)["ok"], true);
    assert_eq!(open(1, None), None);
    assert_eq!(open(2, Some("unknown")), None);
    let persistent = open(3, Some(TOKEN)).expect("a persistent reference proves its peer too");
    assert_eq!(persistent.pane_id, PROVED_PANE);
    assert_eq!(persistent.identity.shell_pid, std::process::id() as i32);

    let issued = ask(socket, PROVED_PANE, 'b', true);
    assert_eq!(issued["ok"], true, "{issued}");
    let proved = events
        .0
        .heard
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            NodeEvent::PaneProof {
                identity,
                one_shot: true,
                ..
            } => Some(identity.clone()),
            _ => None,
        })
        .expect("the one-shot proof");
    assert_eq!(
        open(4, None),
        Some(hide_node_link::panes::StreamCaller {
            pane_id: PROVED_PANE.to_owned(),
            identity: proved,
        })
    );
}

/// The socket proves a hook's caller before forwarding the typed command,
/// answers that command once, and leaves no reusable reference behind.
#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn a_hook_proof_answers_once_and_never_hands_out_a_reference() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let device = Device::start();
    let sockets = pane_sockets(&device.state);
    let socket = &sockets[0];
    let request = json!({"pane_id":PROVED_PANE,"nonce":"ab".repeat(16),"one_shot":true,"hook":{
        "request_id":"hook-1","token":null,"command":{"kind":"memory","event":"SessionStart","runtime":"claude","session":"session"}
    }});
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    writeln!(stream, "{request}").unwrap();
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let answer: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(answer["request_id"], "hook-1");
    assert_eq!(answer["ok"], true);
    assert!(answer.get("reference").is_none());
    assert!(answer.get("token").is_none());
    line.clear();
    assert_eq!(
        reader.read_line(&mut line).unwrap(),
        0,
        "one socket answers only one hook"
    );
    assert!(
        !socket
            .parent()
            .unwrap()
            .join(format!("{}.json", "ab".repeat(16)))
            .exists()
    );
    let proofs = device.events.heard_until(|events| {
        events
            .iter()
            .any(|event| matches!(event, NodeEvent::PaneProof { hook: Some(_), .. }))
    });
    assert!(proofs.iter().any(|event| matches!(event, NodeEvent::PaneProof { pane_id, identity, one_shot: true, hook: Some(_), .. } if pane_id == PROVED_PANE && identity.terminal_id == "t-proved")));
    for invalid in [
        json!({"kind":"workspace_action","command":{"type":"close_pane"}}),
        json!({"kind":"letter_confirm","ids":["letter-1"]}),
    ] {
        let mut request = request.clone();
        request["hook"]["command"] = invalid;
        let mut stream = UnixStream::connect(socket).unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        writeln!(stream, "{request}").unwrap();
        let mut answer = String::new();
        BufReader::new(stream).read_line(&mut answer).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&answer).unwrap()["reason"],
            "invalid_request"
        );
    }
    let mut other_pane = request.clone();
    other_pane["pane_id"] = json!("w1:p999");
    let mut stream = UnixStream::connect(socket).unwrap();
    stream.set_read_timeout(Some(TIMEOUT)).unwrap();
    writeln!(stream, "{other_pane}").unwrap();
    let mut answer = String::new();
    BufReader::new(stream).read_line(&mut answer).unwrap();
    assert_eq!(serde_json::from_str::<Value>(&answer).unwrap()["ok"], false);
    assert!(!device.events.heard.lock().unwrap().iter().any(|event| matches!(event, NodeEvent::PaneProof { pane_id, hook: Some(_), .. } if pane_id == "w1:p999")));
}

#[test]
#[ignore = "the system's sshd; the remote mailbox lane runs it"]
fn workspace_guidance_gets_one_reusable_reference_and_rejects_writes() {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let device = Device::start();
    let sockets = pane_sockets(&device.state);
    let ask = |request: &Value| {
        let mut stream = UnixStream::connect(&sockets[0]).unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        writeln!(stream, "{request}").unwrap();
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let answer: Value = serde_json::from_str(&line).unwrap();
        line.clear();
        assert_eq!(reader.read_line(&mut line).unwrap(), 0);
        answer
    };
    let request = json!({"pane_id":PROVED_PANE,"nonce":"cd".repeat(16),"one_shot":false,"hook":{
        "request_id":"guidance","token":null,"command":{"kind":"workspace_info"}
    }});
    let answer = ask(&request);
    assert_eq!(answer["ok"], true);
    assert_eq!(answer["result"]["capabilities"], json!(["workspace.info"]));
    assert!(answer.get("token").is_none());
    let reference = std::path::PathBuf::from(answer["reference"].as_str().unwrap());
    assert!(reference.is_file());
    assert!(reference.with_extension("claimed").is_file());
    let mut repeated = request.clone();
    repeated["nonce"] = json!("de".repeat(16));
    assert_eq!(ask(&repeated)["reference"], answer["reference"]);
    for invalid in [
        json!({"kind":"workspace_info","command":{"type":"close_pane"}}),
        json!({"kind":"workspace_action","command":{"type":"close_pane"}}),
        json!({"kind":"letter_confirm","ids":["letter-1"]}),
    ] {
        let mut refused = request.clone();
        refused["hook"]["command"] = invalid;
        assert_eq!(ask(&refused)["reason"], "invalid_request");
    }
    let mut wrong_lifetime = request.clone();
    wrong_lifetime["one_shot"] = json!(true);
    assert_eq!(ask(&wrong_lifetime)["reason"], "invalid_request");
    let mut other_pane = request;
    other_pane["pane_id"] = json!("w1:p999");
    assert_eq!(ask(&other_pane)["ok"], false);
    assert!(
        reference.is_file(),
        "a refusal cannot revoke the previous reference"
    );
    let accepted: bool = call_as(
        device.link.as_ref(),
        Call::PaneProofAnswer {
            request: u64::MAX,
            answer: ProofAnswer::WorkspaceInfo {
                token: TOKEN.to_owned(),
                issued_new: false,
                answer: json!({"ok":true}),
            },
        },
        TIMEOUT,
    )
    .unwrap();
    assert!(
        !accepted,
        "a late answer cannot claim an undelivered reference"
    );
}
