//! A node whose core runs on another machine (PRD core-host-node-remote-core):
//! candidate hided on both sides, the pinned Herdr on each machine, and a
//! real SSH connection between them that only the screen machine opens.
#![cfg(unix)]

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use hided::node_role::{LinkFailure, NodeIdentity, NodeRole, Phase};
use serde_json::Value;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use crate::support::remote_core::{CORE_NODE, Fixture, Herdr};
use crate::support::remote_delivery::wait_for;
use crate::support::screen::{
    Socket, chunks, first_snapshot, next_frame, plain, screen_socket, screen_socket_of, send,
    type_and_read,
};

const LINK_BOUND: Duration = Duration::from_secs(30);

/// Where an in-process node's own panes' output goes: a hub no screen reads.
fn screen() -> std::sync::Arc<dyn hide_node::terminal::OutputSink> {
    hided::terminal_hub::TerminalHub::new()
}

fn identity(fixture: &Fixture) -> Result<NodeIdentity> {
    Ok(NodeIdentity {
        node: herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned(),
        label: "screen-fixture".to_owned(),
        build: hided::build_id::of_file(&fixture.hided).map_err(anyhow::Error::msg)?,
        release: hided::build_order::Release::of_this_build(),
        herdr_socket: fixture.screen.socket.clone(),
        herdr_bin: std::env::var_os("HIDE_E2E_HERDR_BIN")
            .map(std::path::PathBuf::from)
            .context("HIDE_E2E_HERDR_BIN")?,
        worktree_facts: None,
    })
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_dials_its_core_and_the_core_reaches_its_herdr_through_the_link() -> Result<()> {
    Fixture::journey(|fixture| {
        let identity = identity(fixture)?;
        let node = identity.node.clone();
        let role = NodeRole::start(
            fixture.screen_home(),
            fixture.placement(),
            identity,
            screen(),
        )
        .map_err(anyhow::Error::msg)?;
        let phase = role.wait_for(LINK_BOUND, |phase| matches!(phase, Phase::Live(_)));
        ensure!(
            matches!(phase, Phase::Live(_)),
            "the node never linked: {phase:?}"
        );
        // The core took the node as a device of its own, keyed by the node
        // id, with no consent asked and its helper ready.
        let row = wait_for("the node's ready row on the core", || {
            Ok(fixture.device(&node)?.filter(|row| {
                row.pointer("/host/state")
                    .is_some_and(|state| state == "ready")
            }))
        })?;
        ensure!(
            row["kind"] == "remote",
            "the node's row is not remote: {row}"
        );
        // The core reaches the node's Herdr only through the link: a
        // checkout on the node registers and its session reads.
        let project = fixture.screen_home().join("project");
        fixture.create_workspace_on(&node, &project)?;
        wait_for("the node's checkout registered on the core", || {
            let snapshot = fixture.snapshot()?;
            Ok(snapshot
                .pointer("/status/remote")
                .and_then(Value::as_array)
                .and_then(|rows| rows.iter().find(|row| row["target_id"] == node.as_str()))
                .and_then(|remote| remote.pointer("/session/workspaces"))
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter().any(|row| {
                        row["registered"] == true
                            && row["path"] == project.to_string_lossy().as_ref()
                    })
                })
                .then_some(()))
        })?;
        fixture
            .core_log_until("node_link", "attach.linked", |rows| !rows.is_empty())
            .context("the core logged no linked node")?;
        // The node's end ends the link, and the core's row says so.
        drop(role);
        wait_for("the core's row after the node left", || {
            Ok(fixture
                .device(&node)?
                .filter(|row| {
                    row.pointer("/host/state")
                        .is_some_and(|state| state != "ready")
                })
                .map(|_| ()))
        })?;
        Ok(())
    })
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_of_another_build_or_the_cores_own_machine_is_refused() -> Result<()> {
    Fixture::journey(|fixture| {
        let mut other = identity(fixture)?;
        other.build = "0".repeat(64);
        let role = NodeRole::start(fixture.screen_home(), fixture.placement(), other, screen())
            .map_err(anyhow::Error::msg)?;
        let phase = role.wait_for(LINK_BOUND, |phase| matches!(phase, Phase::Waiting { .. }));
        ensure!(
            phase
                == Phase::Waiting {
                    reason: LinkFailure::Refused("other_build".to_owned())
                },
            "another build was not refused: {phase:?}"
        );
        drop(role);
        let mut own = identity(fixture)?;
        own.node = CORE_NODE.to_owned();
        let role = NodeRole::start(fixture.screen_home(), fixture.placement(), own, screen())
            .map_err(anyhow::Error::msg)?;
        let phase = role.wait_for(LINK_BOUND, |phase| matches!(phase, Phase::Waiting { .. }));
        ensure!(
            phase
                == Phase::Waiting {
                    reason: LinkFailure::Refused("own_node".to_owned())
                },
            "the core's own machine was not refused: {phase:?}"
        );
        Ok(())
    })
}

/// A core link that fails on what only the operator can fix starts no ssh
/// by itself: an alias whose server key is not known yet waits as a sign-in
/// the operator owes, a window that connected during that first dial adds
/// none, the server sees no other connection however long it waits, and the
/// operator's connect after they trusted the key links it.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_core_link_that_needs_the_operator_dials_again_only_when_they_connect() -> Result<()> {
    Fixture::journey(|fixture| {
        const UNKNOWN: &str = "core-unknown-key";
        let known_hosts = fixture
            .screen_home()
            .join(".ssh")
            .join("unknown_known_hosts");
        std::fs::write(&known_hosts, "")?;
        let held = held_dial(fixture.ssh.port)?;
        let config = fixture.screen_home().join(".ssh").join("config");
        let alias = fixture
            .ssh
            .client_config(UNKNOWN)
            .replace(
                &fixture.ssh.known_hosts.display().to_string(),
                &known_hosts.display().to_string(),
            )
            .replace(
                &format!("Port {}\n", fixture.ssh.port),
                &format!("Port {}\n", held.port),
            );
        std::fs::write(&config, std::fs::read_to_string(&config)? + &alias)?;
        let placement = hided::placement::Placement {
            alias: UNKNOWN.to_owned(),
            ..fixture.placement()
        };
        let before = fixture.ssh.connections();
        let role = NodeRole::start(
            fixture.screen_home(),
            placement,
            identity(fixture)?,
            screen(),
        )
        .map_err(anyhow::Error::msg)?;
        // A cold launch: the window connects while the first dial is in
        // flight, as `hide connect` does, and joins that dial.
        held.accepted
            .recv_timeout(LINK_BOUND)
            .context("the first dial never reached the server")?;
        ensure!(
            role.phase() == Phase::Connecting,
            "the first dial ended while it was held"
        );
        role.connect_again(None);
        held.open.send(())?;
        let owed = |phase: &Phase| {
            matches!(phase, Phase::Waiting { reason: LinkFailure::Operator(failure) }
                if failure.problem == hide_node_link::device::SshProblem::SignInNeeded)
        };
        let phase = role.wait_for(LINK_BOUND, |phase| matches!(phase, Phase::Waiting { .. }));
        ensure!(
            owed(&phase),
            "an unknown server key did not wait for the operator: {phase:?}"
        );
        ensure!(
            fixture.ssh.connections() == before + 1,
            "the first dial was not one connection: {}",
            fixture.ssh.log()
        );
        // Past the first two retries a backoff would have made (2 s, 4 s).
        let quiet = role.wait_for(Duration::from_secs(7), |phase| !owed(phase));
        ensure!(owed(&quiet), "the wait ended by itself: {quiet:?}");
        ensure!(
            fixture.ssh.connections() == before + 1,
            "ssh dialed again with nobody asking: {}",
            fixture.ssh.log()
        );
        std::fs::write(
            &known_hosts,
            std::fs::read_to_string(&fixture.ssh.known_hosts)?.replace(
                &format!("]:{} ", fixture.ssh.port),
                &format!("]:{} ", held.port),
            ),
        )?;
        role.connect_again(None);
        let phase = role.wait_for(LINK_BOUND, |phase| matches!(phase, Phase::Live(_)));
        ensure!(
            matches!(phase, Phase::Live(_)),
            "the operator's connect did not link: {phase:?}"
        );
        ensure!(
            fixture.ssh.connections() == before + 2,
            "the operator's connect was not one dial: {}",
            fixture.ssh.log()
        );
        Ok(())
    })
}

/// A loopback relay to the server on `to` that holds the first connection
/// it accepts until `open` hears, so a test acts while a dial is in flight;
/// every later connection goes straight through. Its threads end with the
/// test's process.
struct HeldDial {
    port: u16,
    accepted: std::sync::mpsc::Receiver<()>,
    open: std::sync::mpsc::SyncSender<()>,
}

fn held_dial(to: u16) -> Result<HeldDial> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let (accepted_tx, accepted) = std::sync::mpsc::channel();
    let (open, opened) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        for (index, client) in listener.incoming().enumerate() {
            let Ok(client) = client else { return };
            if index == 0 && (accepted_tx.send(()).is_err() || opened.recv().is_err()) {
                return;
            }
            let Ok(server) = std::net::TcpStream::connect(("127.0.0.1", to)) else {
                return;
            };
            for (from, into) in [
                (client.try_clone(), server.try_clone()),
                (server.try_clone(), client.try_clone()),
            ] {
                let (Ok(mut from), Ok(mut into)) = (from, into) else {
                    return;
                };
                std::thread::spawn(move || {
                    let _ = std::io::copy(&mut from, &mut into);
                    let _ = into.shutdown(std::net::Shutdown::Write);
                });
            }
        }
    });
    Ok(HeldDial {
        port,
        accepted,
        open,
    })
}

/// Brings the checkout at `path` on `device` forward from `socket`, as a
/// click on its sidebar row does, once the core's navigator lists it.
async fn focus_checkout(
    fixture: &Fixture,
    socket: &mut Socket,
    device: &str,
    path: &std::path::Path,
) -> Result<()> {
    send(socket, "focus_device", json!({"device_id": device})).await?;
    let (workspace, checkout) = tokio::task::block_in_place(|| listed_checkout(fixture, path))?;
    send(
        socket,
        "focus_checkout",
        json!({"workspace_id": workspace, "checkout_id": checkout, "focus_device": true}),
    )
    .await
}

/// The Project and checkout ids the core's catalog lists for `path`, once it
/// lists it.
fn listed_checkout(fixture: &Fixture, path: &std::path::Path) -> Result<(String, String)> {
    let path = path.to_string_lossy().into_owned();
    let mut seen = Value::Null;
    wait_for("the checkout in the core's navigator", || {
        let snapshot = fixture.snapshot()?;
        // This machine's checkouts are the navigator's; a device's are its
        // session's.
        let mut listed: Vec<Value> = snapshot["navigator"]["workspaces"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for remote in snapshot["status"]["remote"]
            .as_array()
            .into_iter()
            .flatten()
        {
            listed.extend(
                remote["session"]["workspaces"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        seen = json!(
            listed
                .iter()
                .map(|workspace| &workspace["path"])
                .collect::<Vec<_>>()
        );
        Ok(listed
            .iter()
            .find(|workspace| workspace["path"] == path.as_str())
            .and_then(|workspace| {
                Some((
                    workspace["id"].as_str()?.to_owned(),
                    workspace["checkouts"][0]["id"].as_str()?.to_owned(),
                ))
            }))
    })
    .with_context(|| format!("the core's navigator: {seen}"))
}

/// Waits up to `bound` for the node's link to reach `phase`, as its
/// `/health` reports it, and answers that report.
async fn node_link(port: u16, phase: &str, bound: Duration) -> Result<Value> {
    let deadline = Instant::now() + bound;
    loop {
        let health = node_health(port).await?;
        ensure!(
            health["role"] == "node",
            "the daemon is not in the node role: {health}"
        );
        if health["core_link"] == phase {
            return Ok(health);
        }
        ensure!(
            Instant::now() < deadline,
            "the node's link never reached {phase}: {health}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Reads `socket` until it closes within `bound`, and answers the close
/// frame's code and reason; a frame that is not a close is skipped.
async fn close_of(socket: &mut Socket, bound: Duration) -> Result<(u16, String)> {
    let deadline = Instant::now() + bound;
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .context("the screen socket never closed")?;
        let message = tokio::time::timeout(left, socket.next())
            .await
            .context("the screen socket never closed")?;
        match message {
            Some(Ok(Message::Close(Some(frame)))) => {
                return Ok((u16::from(frame.code), frame.reason.to_string()));
            }
            Some(Ok(Message::Close(None))) | None => bail!("the screen closed with no frame"),
            Some(Ok(_)) => {}
            Some(Err(error)) => bail!("the screen socket failed before its close: {error}"),
        }
    }
}

/// What `pane` draws on `socket` after a new view asks it whole, without
/// its control sequences, read until `until` is on it or `bound` passes:
/// the whole screen comes as one frame, which may take its time under load.
async fn pane_text(
    socket: &mut Socket,
    pane: &str,
    until: &str,
    bound: Duration,
) -> Result<String> {
    send(
        socket,
        "terminal_viewport",
        json!({"pane_id": pane, "cols": 100, "rows": 30, "new_view": true}),
    )
    .await?;
    let mut screen = String::new();
    let deadline = Instant::now() + bound;
    while let Some(frame) = next_frame(socket, deadline).await? {
        for (from, bytes) in chunks(&frame) {
            if from == pane {
                screen.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        if plain(&screen).contains(until) {
            break;
        }
    }
    Ok(plain(&screen))
}

async fn node_health(port: u16) -> Result<Value> {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await?;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream
        .write_all(
            format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await?;
    let text = String::from_utf8(response)?;
    let body = text.split_once("\r\n\r\n").context("health body")?.1;
    Ok(serde_json::from_str(body)?)
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_screen_on_the_node_draws_the_cores_state_and_its_own_pane() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            // The diagnostic log is written behind the daemon's own work.
            tokio::task::block_in_place(|| {
                wait_for("the node role's start logged beside its state", || {
                    Ok((!fixture.node_log("node_daemon", "started")?.is_empty()).then_some(()))
                })
            })?;
            let mut socket = screen_socket(port, &token).await?;
            // The screen's first state is the core's: the node is one of its
            // devices.
            let snapshot = first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            let devices = snapshot["payload"]["rest"]["navigator"]["devices"].clone();
            ensure!(
                devices
                    .as_array()
                    .is_some_and(|rows| rows.iter().any(|row| row["id"] == node.as_str())),
                "the core's snapshot did not reach the node's screen: {devices}"
            );
            // A checkout on the node, registered from the node's screen: its
            // pane's keys and output stay on the node.
            let project = fixture.screen_home().join("project");
            let herdr_pane = fixture.screen.workspace_at(&project)?;
            let pane = format!("remote:{node}:pane:{herdr_pane}");
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            type_and_read(&mut socket, &pane, "echo node-\"ok\"", "node-ok").await?;
            // A pane of the core's machine, typed from the node's screen: its
            // keys and output ride the node's terminals relay.
            let core_project = fixture.core_home().join("project");
            let core_pane = fixture.core.workspace_at(&core_project)?;
            send(
                &mut socket,
                "create_workspace",
                json!({"path": core_project, "label": "core", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, CORE_NODE, &core_project).await?;
            type_and_read(&mut socket, &core_pane, "echo core-\"ok\"", "core-ok").await?;
            // A window of the core's own draws the node's pane too, once it
            // opens: the node sends its output up only then.
            let (core_port, core_token) = fixture.core_screen();
            let mut window = screen_socket(core_port, &core_token).await?;
            let deadline = Instant::now() + Duration::from_secs(20);
            while next_frame(&mut window, deadline)
                .await?
                .context("no snapshot reached the core's window")?["type"]
                != "snapshot"
            {}
            // The operator looks at the node's checkout again, from the
            // node's screen, and a window of the core's own opens on it.
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            send(
                &mut window,
                "terminal_viewport",
                json!({"pane_id": pane, "cols": 100, "rows": 30, "new_view": true}),
            )
            .await?;
            let keys = base64::engine::general_purpose::STANDARD.encode("echo mirror-\"ok\"\r");
            send(&mut socket, "key", json!({"pane_id": pane, "bytes_base64": keys})).await?;
            let mut drawn = String::new();
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let frame = next_frame(&mut window, deadline).await?.with_context(|| {
                    format!("the node's pane never drew on the core's window: {:?}", plain(&drawn))
                })?;
                for (from, bytes) in chunks(&frame) {
                    if from == pane {
                        drawn.push_str(&String::from_utf8_lossy(&bytes));
                    }
                }
                if plain(&drawn).contains("mirror-ok") {
                    break;
                }
            }
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// What a held screen receives until `deadline`: where the node's link
/// stands, each phase it was told, and nothing else.
async fn held_for(socket: &mut Socket, deadline: Instant) -> Result<Vec<Value>> {
    let mut told = Vec::new();
    while let Some(frame) = next_frame(socket, deadline).await? {
        ensure!(
            frame["type"] == "core_link",
            "a held screen received {frame}"
        );
        told.push(frame["payload"]["phase"].clone());
    }
    Ok(told)
}

/// What a screen shows of the operator's layout: the focus, the tab and
/// the panes' places, as one snapshot frame carries them.
fn layout(snapshot: &Value) -> Value {
    let rest = &snapshot["payload"]["rest"];
    json!({
        "focused": rest["focused"],
        "tab": rest["tab"],
        "zoomed": rest["zoomed"],
        "pane_layouts": rest["pane_layouts"],
    })
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_that_loses_its_core_holds_its_screens_and_returns_as_it_was() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let project = fixture.screen_home().join("project");
        let herdr_pane = fixture.screen.workspace_at(&project)?;
        let pane = format!("remote:{node}:pane:{herdr_pane}");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            type_and_read(&mut socket, &pane, "echo before-\"ok\"", "before-ok").await?;
            // The layout as a screen opened now draws it.
            let mut look = screen_socket(port, &token).await?;
            let before = layout(&first_snapshot(&mut look, Duration::from_secs(20)).await?);
            drop(look);

            // The core's machine stops answering SSH: the open screen is
            // closed as one whose core is lost, and the node waits.
            tokio::task::block_in_place(|| fixture.ssh.online(false))?;
            let (code, reason) = close_of(&mut socket, Duration::from_secs(15)).await?;
            ensure!(
                (code, reason.as_str()) == (1012, "core_link_lost"),
                "the screen closed as {code} {reason}"
            );
            // The link's end, then a dial that finds no SSH server.
            let deadline = Instant::now() + LINK_BOUND;
            loop {
                let waiting = node_link(port, "waiting", LINK_BOUND).await?;
                let reason = waiting["core_link_reason"].as_str().unwrap_or_default();
                if reason.starts_with("unreachable") {
                    break;
                }
                ensure!(
                    Instant::now() < deadline,
                    "the node waits for another reason: {waiting}"
                );
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            // A screen opened meanwhile is held: nothing but where the link
            // stands reaches it, and the keys it sends are dropped rather
            // than kept for later.
            let mut held = screen_socket(port, &token).await?;
            let keys = base64::engine::general_purpose::STANDARD.encode("echo held-\"leak\"\r");
            send(&mut held, "key", json!({"pane_id": pane, "bytes_base64": keys})).await?;
            let told = held_for(&mut held, Instant::now() + Duration::from_secs(5)).await?;
            ensure!(
                told.first().is_some_and(|phase| phase == "waiting" || phase == "connecting"),
                "the held screen was told {told:?}"
            );
            // The outage lasts until the node's next try is further away than
            // the return is allowed to take: only the node noticing SSH
            // answer again brings it back in time.
            tokio::task::block_in_place(|| {
                wait_for("a retry wait past the return bound", || {
                    let ended = fixture.node_log("node_role", "link.ended")?;
                    Ok(ended
                        .iter()
                        .any(|entry| entry["retry_ms"].as_u64() >= Some(16_000))
                        .then_some(()))
                })
            })?;
            held_for(&mut held, Instant::now()).await?;

            // SSH answers again: the held screen attaches within ten seconds
            // and draws the layout it left.
            tokio::task::block_in_place(|| fixture.ssh.online(true))?;
            let back = Instant::now();
            let snapshot = first_snapshot(&mut held, Duration::from_secs(10))
                .await
                .context("the held screen did not attach within ten seconds")?;
            let returned = back.elapsed();
            let after = layout(&snapshot);
            ensure!(before == after, "the layout moved: {before} then {after}");
            type_and_read(&mut held, &pane, "echo after-\"ok\"", "after-ok").await?;
            let text = pane_text(&mut held, &pane, "after-ok", Duration::from_secs(20)).await?;
            ensure!(
                text.contains("after-ok") && !text.contains("held-"),
                "the pane after the outage reads {text:?}"
            );
            eprintln!("reattached {returned:?} after SSH answered again");
            Ok::<_, anyhow::Error>(())
        })?;
        // B19: the link's start, end and retries are recorded with the
        // machines they joined, and nothing typed is.
        fixture
            .core_log_until("node_link", "attach.linked", |linked| {
                linked.len() >= 2 && linked.iter().all(|row| row["node"] == node.as_str())
            })
            .context("the core's link records")?;
        let ended = fixture.node_log("node_role", "link.ended")?;
        ensure!(
            ended
                .iter()
                .all(|row| row["core"] == CORE_NODE && row["retry_ms"].is_u64()),
            "the node's link records: {ended:?}"
        );
        for typed in ["before-", "after-", "held-"] {
            ensure!(!fixture.logs_mention(typed)?, "{typed} reached a log");
        }
        Ok(())
    })
}

/// The node's placement record, as the node left it.
fn node_record(fixture: &Fixture) -> Result<Value> {
    let record = hided::placement::record_path(&fixture.node_state());
    Ok(serde_json::from_slice(&std::fs::read(record)?)?)
}

/// The answer `socket` gets to a `file_list` of `project` on `node`: a
/// listing or why there is none.
async fn listing_of(socket: &mut Socket, node: &str, project: &std::path::Path) -> Result<Value> {
    send(
        socket,
        "file_list",
        json!({"root": project, "path": project, "device_id": node}),
    )
    .await?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let frame = next_frame(socket, deadline)
            .await?
            .context("the file_list was never answered")?;
        if ["directory_list", "directory_unavailable", "path_refused"]
            .contains(&frame["type"].as_str().unwrap_or_default())
        {
            return Ok(frame);
        }
    }
}

/// B16: the operator ends the link from the node's window. At once the
/// core reaches nothing on the node: a key its own window types into the
/// node's pane never arrives and a listing there is refused, and its row
/// for the node is no longer ready. The node dials nothing until the
/// operator reconnects, a restart included, and its held screens are told
/// so; Reconnect links it again and the core reaches the node as before.
/// The node's pages are refused through the same end of the link that
/// `a_node_window_s_page_is_driven_from_its_machine_off_the_link_and_from_the_core_through_it`
/// drives with SSH gone.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_the_operator_disconnects_is_refused_by_its_core_until_they_reconnect() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let project = fixture.screen_home().join("project");
        let herdr_pane = fixture.screen.workspace_at(&project)?;
        let pane = format!("remote:{node}:pane:{herdr_pane}");
        let (core_port, core_token) = fixture.core_screen();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let mut window = runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            type_and_read(&mut socket, &pane, "echo before-\"ok\"", "before-ok").await?;
            // The core's own window reaches the node's checkout.
            let mut window = screen_socket(core_port, &core_token).await?;
            first_snapshot(&mut window, Duration::from_secs(20)).await?;
            focus_checkout(fixture, &mut window, &node, &project).await?;
            let listed = listing_of(&mut window, &node, &project).await?;
            ensure!(listed["type"] == "directory_list", "the core listed {listed}");

            // Disconnect, from the node's own window.
            send(&mut socket, "core_link", json!({"action": "disconnect"})).await?;
            let (code, reason) = close_of(&mut socket, Duration::from_secs(15)).await?;
            ensure!(
                (code, reason.as_str()) == (1012, "core_link_lost"),
                "the screen closed as {code} {reason}"
            );
            node_link(port, "disconnected", LINK_BOUND).await?;
            let keys = base64::engine::general_purpose::STANDARD.encode("echo core-\"leak\"\r");
            send(&mut window, "key", json!({"pane_id": pane, "bytes_base64": keys})).await?;
            let refused = listing_of(&mut window, &node, &project).await?;
            ensure!(
                refused["type"] != "directory_list",
                "the core listed the node's checkout after the disconnect: {refused}"
            );
            Ok::<_, anyhow::Error>(window)
        })?;
        ensure!(
            node_record(fixture)?["disconnected"] == true,
            "the record kept no disconnect"
        );
        wait_for("the node's row not ready on the core", || {
            Ok(fixture.device(&node)?.filter(|row| {
                row.pointer("/host/state")
                    .is_some_and(|state| state != "ready")
            }))
        })?;
        let linked = fixture.core_log("node_link", "attach.linked")?.len();

        // A restart keeps the disconnect: nothing dials, and a held screen
        // is told so and drops what it sends.
        fixture.signal_node(libc::SIGTERM)?;
        let (port, token) = fixture.restart_node()?;
        runtime.block_on(async {
            node_link(port, "disconnected", LINK_BOUND).await?;
            let mut held = screen_socket(port, &token).await?;
            let keys = base64::engine::general_purpose::STANDARD.encode("echo held-\"leak\"\r");
            send(
                &mut held,
                "key",
                json!({"pane_id": pane, "bytes_base64": keys}),
            )
            .await?;
            let told = held_for(&mut held, Instant::now() + Duration::from_secs(5)).await?;
            ensure!(
                told == [json!("disconnected")],
                "the held screen was told {told:?}"
            );
            ensure!(
                tokio::task::block_in_place(|| fixture.attach_processes())?.is_empty(),
                "the disconnected node runs an attach role"
            );
            ensure!(
                tokio::task::block_in_place(|| fixture.core_log("node_link", "attach.linked"))?
                    .len()
                    == linked,
                "the disconnected node linked to its core"
            );

            // Reconnect: the held screen attaches and draws the pane, which
            // nothing typed meanwhile reached.
            send(&mut held, "core_link", json!({"action": "reconnect"})).await?;
            first_snapshot(&mut held, Duration::from_secs(20))
                .await
                .context("the held screen did not attach after Reconnect")?;
            type_and_read(&mut held, &pane, "echo after-\"ok\"", "after-ok").await?;
            let text = pane_text(&mut held, &pane, "after-ok", Duration::from_secs(20)).await?;
            ensure!(
                !text.contains("held-") && !text.contains("core-"),
                "the pane after the disconnect reads {text:?}"
            );
            tokio::task::block_in_place(|| {
                wait_for("the node's ready row on the core again", || {
                    Ok(fixture.device(&node)?.filter(|row| {
                        row.pointer("/host/state")
                            .is_some_and(|state| state == "ready")
                    }))
                })
            })?;
            focus_checkout(fixture, &mut window, &node, &project).await?;
            let listed = listing_of(&mut window, &node, &project).await?;
            ensure!(
                listed["type"] == "directory_list",
                "the core listed {listed}"
            );
            Ok::<_, anyhow::Error>(())
        })?;
        ensure!(
            node_record(fixture)?.get("disconnected").is_none(),
            "the record kept the disconnect after Reconnect"
        );
        for typed in ["held-", "core-\"leak"] {
            ensure!(!fixture.logs_mention(typed)?, "{typed} reached a log");
        }
        Ok(())
    })
}

/// A node whose link went half open, so neither end saw it close (a
/// network change), dials again: the core greets the earlier link, ends it
/// when it does not answer, and takes the new one at once rather than
/// refuse it as already linked until the attach role's silence limit (B9,
/// D-09).
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_that_dials_again_replaces_its_link_that_no_longer_answers() -> Result<()> {
    use hided::attach::{Line, NodeHello, read_line, write_line};
    Fixture::journey(|fixture| {
        let (port, _token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(node_link(port, "live", LINK_BOUND))?;
        // The node reads its link live once the core accepts it; the core
        // holds it as the node's link once its Hello is answered.
        fixture.core_log_until("node_link", "attach.linked", |rows| !rows.is_empty())?;
        // The node stops answering and its link stays open.
        fixture.signal_running_node(libc::SIGSTOP)?;
        let dialed = Instant::now();
        let answer = (|| {
            // The record names the socket the core bound in its own folder.
            let socket = hide_node::pane_proof::recorded_socket_path(
                &hided::attach::attach_record(&fixture.core_state),
            )
            .map_err(anyhow::Error::msg)?;
            let stream = hide_platform::ipc::LocalStream::connect(&socket)?;
            stream.set_read_timeout(Some(Duration::from_secs(20)))?;
            let mut reader = std::io::BufReader::new(stream.duplicate());
            let mut writer = stream;
            let Line::Core(core) = read_line(&mut reader).map_err(anyhow::Error::msg)? else {
                bail!("the core did not greet first");
            };
            write_line(
                &mut writer,
                &Line::Node(NodeHello {
                    node: node.clone(),
                    label: "screen-fixture".to_owned(),
                    build: core.build,
                    herdr_socket: fixture.screen.socket.display().to_string(),
                    release: core.release,
                    move_intent: None,
                }),
            )
            .map_err(anyhow::Error::msg)?;
            read_line(&mut reader).map_err(anyhow::Error::msg)
        })();
        let took = dialed.elapsed();
        fixture.signal_running_node(libc::SIGCONT)?;
        let answer = answer?;
        ensure!(
            matches!(answer, Line::Accepted(_)),
            "the second link was answered {answer:?}"
        );
        ensure!(
            took < Duration::from_secs(10),
            "the second link waited {took:?}"
        );
        fixture
            .core_log_until("node_link", "attach.superseded", |rows| !rows.is_empty())
            .with_context(|| {
                format!("the core's link records: {:?}", fixture.core_link_records())
            })?;
        // The node finds its own link ended and returns.
        runtime.block_on(node_link(port, "live", LINK_BOUND))?;
        Ok(())
    })
}

/// The core's own rule, whatever a node decides before its hello (D-10):
/// only the same build links; an older node is told the core is newer, and
/// a newer or unordered one is refused as another build, since a newer node
/// updates the core before it dials and never links across builds.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn the_core_links_only_its_own_build_and_tells_an_older_node_it_is_newer() -> Result<()> {
    use hided::attach::{Line, NodeHello, read_line, write_line};
    use hided::build_order::Release;
    Fixture::journey(|fixture| {
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let socket = hide_node::pane_proof::recorded_socket_path(&hided::attach::attach_record(
            &fixture.core_state,
        ))
        .map_err(anyhow::Error::msg)?;
        let answer = |version: &str| -> Result<(Line, Release)> {
            let stream = hide_platform::ipc::LocalStream::connect(&socket)?;
            stream.set_read_timeout(Some(Duration::from_secs(20)))?;
            let mut reader = std::io::BufReader::new(stream.duplicate());
            let mut writer = stream;
            let Line::Core(core) = read_line(&mut reader).map_err(anyhow::Error::msg)? else {
                bail!("the core did not greet first");
            };
            write_line(
                &mut writer,
                &Line::Node(NodeHello {
                    node: node.clone(),
                    label: "screen-fixture".to_owned(),
                    build: "0".repeat(64),
                    herdr_socket: fixture.screen.socket.display().to_string(),
                    release: Release {
                        version: version.to_owned(),
                        order: core.release.order,
                        commit: None,
                    },
                    move_intent: None,
                }),
            )
            .map_err(anyhow::Error::msg)?;
            Ok((
                read_line(&mut reader).map_err(anyhow::Error::msg)?,
                core.release,
            ))
        };
        let refused = |line: &Line| match line {
            Line::Refused(refusal) => Some(refusal.reason.clone()),
            _ => None,
        };
        let (older, core) = answer("0.0.1")?;
        ensure!(
            refused(&older).as_deref() == Some("core_newer"),
            "an older node was answered {older:?} by core {core:?}"
        );
        for version in ["999.0.0", "nightly"] {
            let (other, _) = answer(version)?;
            ensure!(
                refused(&other).as_deref() == Some("other_build"),
                "a node at {version} was answered {other:?}"
            );
        }
        let older_logged = |row: &Value| {
            row["reason"] == "core_newer"
                && row["node_release"]["version"] == "0.0.1"
                && row["core_release"]["version"] == core.version.as_str()
        };
        fixture.core_log_until("node_link", "attach.refused", |rows| {
            rows.iter().any(older_logged)
        })?;
        Ok(())
    })
}

/// `hide connect --json` of the screen machine, run with `hide` at `cli`.
fn screen_connect(fixture: &Fixture, cli: &std::path::Path) -> Result<Value> {
    let output = fixture
        .screen_command(&cli.join("hide"))
        .args(["connect", "--json"])
        .output()?;
    serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "hide connect answered {:?} {:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_screen_whose_core_is_not_running_attaches_and_waits_for_it() -> Result<()> {
    Fixture::journey(|fixture| {
        let core_state = fixture.core_state.join("hided.json");
        let before = std::fs::read(&core_state)?;
        fixture.stop_core()?;
        let (port, _) = fixture.start_node()?;
        let cli = fixture.hided.parent().context("the CLI folder")?.to_owned();
        let answer = screen_connect(fixture, &cli)?;
        ensure!(answer["ok"] == true, "hide connect refused: {answer}");
        // B13: the screen's page learns which machine it runs on.
        let node = herdr_core::node::NodeId::of_this_machine().map_err(anyhow::Error::msg)?;
        ensure!(
            answer["url"]
                .as_str()
                .is_some_and(|url| url.ends_with(&format!("&node={}", node.as_str()))),
            "the screen's address names no machine: {answer}"
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let health = runtime.block_on(node_health(port))?;
        // The core was killed, so its record still names its socket, which
        // no core answers.
        ensure!(
            health["core_link"] == "waiting" && health["core_link_reason"] == "core_not_answering",
            "the node does not wait for its core: {health}"
        );
        // The attach role answered without starting a core in its place.
        ensure!(
            std::fs::read(&core_state)? == before,
            "a core started on the core's machine"
        );
        // The node keeps dialing as it does with no record: a core started
        // again on its machine is reached with nothing done on the screen.
        fixture.start_core()?;
        runtime.block_on(node_link(port, "live", LINK_BOUND))?;
        Ok(())
    })
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_screen_of_another_build_than_its_core_is_told_so() -> Result<()> {
    Fixture::journey(|fixture| {
        // The screen machine's app is another build: its hided differs from
        // the core's by one byte past the program's end.
        let cli = fixture.root.join("other-build");
        std::fs::create_dir_all(&cli)?;
        let source = fixture.hided.parent().context("the CLI folder")?;
        for name in ["hide", "hided"] {
            std::fs::copy(source.join(name), cli.join(name))?;
        }
        let mut bytes = std::fs::read(cli.join("hided"))?;
        bytes.push(0);
        std::fs::write(cli.join("hided"), bytes)?;
        fixture.start_node_with(&cli.join("hided"))?;
        let answer = screen_connect(fixture, &cli)?;
        ensure!(
            answer["ok"] == false && answer["reason"] == "other_build",
            "hide connect did not answer other_build: {answer}"
        );
        // Builds that only their hashes tell apart are not ordered: the
        // node refuses before it says hello and names both builds.
        let refused = fixture.node_log("node_role", "link.build_refused")?;
        ensure!(
            refused.iter().any(|row| {
                row["reason"] == "other_build" && row["node_build"] != row["core_build"]
            }),
            "the node logged no build mismatch: {refused:?}"
        );
        Ok(())
    })
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_pane_calls_its_core_through_the_link() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let project = fixture.screen_home().join("project");
        let herdr_pane = fixture.screen.workspace_at(&project)?;
        let pane = format!("remote:{node}:pane:{herdr_pane}");
        let hide = fixture.hided.with_file_name("hide");
        let call = format!(
            "HIDE_STATE_DIR={} {} workspace info; echo info-\"done\"",
            fixture.node_state().display(),
            hide.display()
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            // A pane's agent: vouched for by the node's kernel as that pane,
            // so it may act as the pane's agent.
            let answer = type_and_read(&mut socket, &pane, &call, "info-done").await?;
            let info = workspace_answer(&answer).with_context(|| format!("the pane read {answer:?}"))?;
            ensure!(
                info["ok"] == true
                    && info["result"]["context"]["device_id"] == node.as_str()
                    && capabilities(&info).contains(&"inbox".to_owned()),
                "the pane's call was not answered as the node's pane: {info}"
            );
            // A tool outside every pane, in the node's checkout: bound to the
            // checkout, so it acts on the Workspace but not as an agent.
            let outside = |cwd: &std::path::Path, pane: Option<&str>| -> Result<Value> {
                let mut command = fixture.screen_command(&hide);
                command.args(["workspace", "info"]).current_dir(cwd);
                if let Some(pane) = pane {
                    command.env("HERDR_PANE_ID", pane);
                }
                let output = command.output()?;
                workspace_answer(&String::from_utf8_lossy(&output.stdout)).with_context(|| {
                    format!("stderr: {}", String::from_utf8_lossy(&output.stderr))
                })
            };
            let info = tokio::task::block_in_place(|| outside(&project, None))?;
            ensure!(
                info["ok"] == true
                    && info["result"]["context"]["device_id"] == node.as_str()
                    && !capabilities(&info).contains(&"inbox".to_owned()),
                "the checkout caller was not answered as the checkout: {info}"
            );
            // Naming the pane from outside it, and outside every checkout,
            // proves nothing.
            let home = fixture.screen_home().to_owned();
            let info = tokio::task::block_in_place(|| outside(&home, Some(&herdr_pane)))?;
            ensure!(
                info["ok"] == false && info["reason"] == "checkout_not_registered",
                "a caller outside the pane and every checkout was answered: {info}"
            );
            // B19: the refusal is recorded with its node and reason; no pane
            // was proven, so it names none.
            tokio::task::block_in_place(|| {
                fixture.core_log_until("node_panes", "pane.refused", |refused| {
                    refused.iter().any(|row| {
                        row["node"] == node.as_str()
                            && row["reason"] == "checkout_not_registered"
                            && row["pane_id"].is_null()
                    })
                })
            })
            .context("the refusal records")?;
            // The link ends: the node's callers are refused at once, and
            // answered again once it is back.
            tokio::task::block_in_place(|| fixture.ssh.online(false))?;
            node_link(port, "waiting", LINK_BOUND).await?;
            let asked = Instant::now();
            let info = tokio::task::block_in_place(|| outside(&project, None))?;
            ensure!(
                info["ok"] == false && info["reason"] == "hide_unavailable",
                "a caller was answered while the link was down: {info}"
            );
            ensure!(
                asked.elapsed() < Duration::from_secs(2),
                "the refusal took {:?}",
                asked.elapsed()
            );
            tokio::task::block_in_place(|| fixture.ssh.online(true))?;
            node_link(port, "live", LINK_BOUND).await?;
            // The window reopens, as the web shell does after a close.
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            let mut last = Value::Null;
            let info = tokio::task::block_in_place(|| {
                wait_for("the checkout caller answered again", || {
                    let info = outside(&project, None)?;
                    last = info.clone();
                    Ok((info["ok"] == true).then_some(info))
                })
            })
            .with_context(|| format!("the last answer: {last}"))?;
            ensure!(
                info["result"]["context"]["device_id"] == node.as_str(),
                "{info}"
            );
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// The core's Memory store with one enabled Project, `checkout`'s on `node`,
/// holding `rule`.
fn seed_memory(
    fixture: &Fixture,
    node: &str,
    checkout: &std::path::Path,
    rule: &str,
) -> Result<()> {
    let project = hide_project::resolve(checkout, node)?;
    let mut store =
        hide_memory::MemoryStore::open(&hide_memory::database_path(&fixture.core_state))?;
    store.ensure_project(&project.id, &project.root, node)?;
    store.set_enabled(&project.id, true, true)?;
    store.apply_candidates(
        &hide_memory::AnalysisBatch {
            id: format!("{node}-batch"),
            project_id: project.id.clone(),
            provider: "codex".into(),
            analysis_provider: "codex".into(),
            session_id: "source-session".into(),
            content_hash: format!("{node}-hash"),
            // Made now, as an analysis makes it: a card made long ago and
            // never given is archived by the core's first aging pass.
            created_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64,
        },
        &[hide_memory::Candidate {
            text: rule.into(),
            title: String::new(),
            aliases: Vec::new(),
            kind: hide_memory::CandidateKind::Rule,
            confidence: 0.9,
            salience: 0.9,
            source_offsets: vec![1],
            human_offsets: vec![1],
            source_times: Default::default(),
            source_words: Default::default(),
            cited_paths: Vec::new(),
            relation: hide_memory::CandidateRelation::New,
        }],
    )?;
    Ok(())
}

/// `hide workspace memory` for a session start in `cwd`, as `command` runs
/// it, and how long the answer took.
fn ask_memory(command: std::process::Command, cwd: &std::path::Path) -> Result<(Value, Duration)> {
    ask_memory_for(command, cwd, "SessionStart", None)
}

/// `hide workspace memory` for `event` in `cwd`, with `prompt` on its
/// standard input as a hook sends it.
fn ask_memory_for(
    mut command: std::process::Command,
    cwd: &std::path::Path,
    event: &str,
    prompt: Option<&str>,
) -> Result<(Value, Duration)> {
    use std::io::Write;
    command
        .args(["workspace", "memory", "--event", event])
        .args(["--runtime", "codex", "--session", "memory-session"])
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let asked = Instant::now();
    let mut child = command.spawn()?;
    let mut stdin = child.stdin.take().context("the command's input")?;
    stdin.write_all(prompt.unwrap_or_default().as_bytes())?;
    drop(stdin);
    let output = child.wait_with_output()?;
    let took = asked.elapsed();
    let answer = workspace_answer(&String::from_utf8_lossy(&output.stdout))
        .with_context(|| format!("stderr: {}", String::from_utf8_lossy(&output.stderr)))?;
    Ok((answer, took))
}

/// PRD core-host-node-move B14: an agent on either machine gets Project
/// Memory from the one store the core owns, for the Project of its own
/// checkout, read by that checkout's node; with the link down the node's
/// agent is refused at once and its turn goes on without Memory.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn an_agent_on_either_machine_reads_the_one_memory_the_core_owns() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let node_project = fixture.screen_home().join("project");
        let core_project = fixture.core_home().join("core-project");
        // Each its own repository: a run folder sits inside this checkout,
        // whose Project a folder in it would otherwise read as.
        for folder in [&node_project, &core_project] {
            std::fs::create_dir_all(folder)?;
            let status = std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(folder)
                .status()?;
            ensure!(status.success(), "git init {}", folder.display());
        }
        fixture.screen.workspace_at(&node_project)?;
        let hide = fixture.hided.with_file_name("hide");
        seed_memory(
            fixture,
            &node,
            &node_project,
            "The node's checkout keeps its rule",
        )?;
        seed_memory(
            fixture,
            CORE_NODE,
            &core_project,
            "The core's checkout keeps its rule",
        )?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            for (device, path) in [(node.as_str(), &node_project), (CORE_NODE, &core_project)] {
                send(
                    &mut socket,
                    "create_workspace",
                    json!({"device_id": device, "path": path, "label": device, "initialize_git": false}),
                )
                .await?;
                focus_checkout(fixture, &mut socket, device, path).await?;
            }
            // A caller outside every pane bootstraps on its first call, which
            // the node answers once the core has bound the checkout; only
            // what follows is Memory's own time.
            let mut info = fixture.screen_command(&hide);
            info.args(["workspace", "info"]).current_dir(&node_project);
            tokio::task::block_in_place(|| info.output())?;
            let (answer, took) = tokio::task::block_in_place(|| {
                ask_memory(fixture.screen_command(&hide), &node_project)
            })?;
            ensure!(
                answer["ok"] == true
                    && answer["result"]["outcome"] == "provided"
                    && answer["result"]["context"]
                        .as_str()
                        .is_some_and(|context| context.contains("<hide-memory-index ")
                            && context.contains("m1 The node's checkout keeps its rule")
                            // The index's receipt records only that the
                            // lines were shown (PRD memory-self-improve B16).
                            && context.contains("<hide-memory-receipt event=\"SessionStart\"")
                            && !context.contains("The core's checkout")),
                "the node's agent did not get its Project's Memory: {answer}"
            );
            eprintln!("memory answer for the node's agent took {took:?}");
            let (answer, took) = tokio::task::block_in_place(|| {
                ask_memory(fixture.core_command(&hide), &core_project)
            })?;
            ensure!(
                answer["ok"] == true
                    && answer["result"]["context"]
                        .as_str()
                        .is_some_and(|context| context.contains("The core's checkout keeps its rule")
                            && !context.contains("The node's checkout")),
                "the core's agent did not get its Project's Memory: {answer}"
            );
            eprintln!("memory answer for the core's agent took {took:?}");
            // A later prompt's first bytes cross to the core with the ask.
            let prompt = "Where does the zebrafinch-7c41 rule apply?";
            let (answer, _) = tokio::task::block_in_place(|| {
                ask_memory_for(
                    fixture.screen_command(&hide),
                    &node_project,
                    "UserPromptSubmit",
                    Some(prompt),
                )
            })?;
            ensure!(answer["ok"] == true, "the node's prompt ask: {answer}");
            // PRD memory-stale-cards: a card an agent reported stale is held
            // out of a prompt, `show` says so through the real CLI, and
            // `release` gives it back; a caller with no agent session cannot
            // report one.
            let words = "Which rule does the node's checkout keep?";
            let gives_m1 = |answer: &Value| {
                answer["result"]["context"]
                    .as_str()
                    .is_some_and(|context| context.contains("The node's checkout keeps its rule"))
            };
            let (answer, _) = tokio::task::block_in_place(|| {
                ask_memory_for(fixture.screen_command(&hide), &node_project, "UserPromptSubmit", Some(words))
            })?;
            ensure!(gives_m1(&answer), "the prompt did not match m1: {answer}");
            ensure!(
                answer["result"]["context"]
                    .as_str()
                    .is_some_and(|context| context.starts_with("<hide-memory-instruction>")),
                "the prompt's cards follow the report instruction: {answer}"
            );
            let memory_cli = |args: &[&str]| {
                tokio::task::block_in_place(|| {
                    let mut command = fixture.screen_command(&hide);
                    command.arg("memory").args(args).current_dir(&node_project);
                    command.output()
                })
            };
            let refused = memory_cli(&["stale", "m1", "the operator changed it"])?;
            ensure!(
                !refused.status.success()
                    && String::from_utf8_lossy(&refused.stdout).contains("memory_stale_session_required"),
                "a caller with no session reported a card: {}",
                String::from_utf8_lossy(&refused.stdout)
            );
            {
                let project = hide_project::resolve(&node_project, &node)?;
                let mut store = hide_memory::MemoryStore::open(&hide_memory::database_path(&fixture.core_state))?;
                store.record_receipt(&project.id, "codex", "reporter", "event:1", "m1@1", 1)?;
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64;
                let outcome = store.report_stale(
                    &project.id,
                    1,
                    &hide_memory::StaleReporter { runtime: "codex", session_id: "reporter", report_offset: None },
                    "the operator changed it",
                    now,
                )?;
                ensure!(matches!(outcome, hide_memory::StaleOutcome::Held(_)), "{outcome:?}");
            }
            let (answer, _) = tokio::task::block_in_place(|| {
                ask_memory_for(fixture.screen_command(&hide), &node_project, "UserPromptSubmit", Some(words))
            })?;
            ensure!(answer["ok"] == true && !gives_m1(&answer), "a held card was given: {answer}");
            let shown = memory_cli(&["show", "m1"])?;
            let text = String::from_utf8_lossy(&shown.stdout);
            ensure!(
                shown.status.success()
                    && text.contains("Held: an agent reported it stale")
                    && text.contains("\"the operator changed it\""),
                "show did not say the card is held: {text}"
            );
            let released = memory_cli(&["release", "m1"])?;
            ensure!(
                released.status.success()
                    && String::from_utf8_lossy(&released.stdout).contains("Released m1"),
                "release: {}",
                String::from_utf8_lossy(&released.stdout)
            );
            let again = memory_cli(&["release", "m1"])?;
            ensure!(
                String::from_utf8_lossy(&again.stdout).contains("memory_card_not_held"),
                "a second release: {}",
                String::from_utf8_lossy(&again.stdout)
            );
            let (answer, _) = tokio::task::block_in_place(|| {
                ask_memory_for(fixture.screen_command(&hide), &node_project, "UserPromptSubmit", Some(words))
            })?;
            ensure!(gives_m1(&answer), "a released card was not given again: {answer}");
            // `hide memory` from the node's agent reads the same store, for
            // the node's Project.
            let shown = tokio::task::block_in_place(|| {
                let mut command = fixture.screen_command(&hide);
                command
                    .args(["memory", "show", "m1"])
                    .current_dir(&node_project);
                command.output()
            })?;
            let text = String::from_utf8_lossy(&shown.stdout);
            ensure!(
                shown.status.success()
                    && text.contains("The node's checkout keeps its rule")
                    && !text.contains("The core's checkout"),
                "the node's hide memory show: {text} {}",
                String::from_utf8_lossy(&shown.stderr)
            );
            // B19: neither the Memory read nor the prompt is recorded with its
            // text, in either machine's log or anywhere in the core's state.
            let mut records = vec![
                fixture.root.join("core-hided.log"),
                fixture.root.join("node-hided.log"),
            ];
            for folder in [fixture.core_state.clone(), fixture.node_state()] {
                records.extend(files_under(&folder)?);
            }
            let mut found_in_store = false;
            for record in &records {
                let Ok(bytes) = std::fs::read(record) else {
                    continue;
                };
                // The store holds the Memory itself, never the prompt; that it
                // is found there shows the search reads what was written.
                let store = record
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("project-memory.sqlite3"));
                for text in ["keeps its rule", "zebrafinch-7c41"] {
                    if store && text == "keeps its rule" {
                        found_in_store |= bytes.windows(text.len()).any(|window| window == text.as_bytes());
                        continue;
                    }
                    ensure!(
                        !bytes.windows(text.len()).any(|window| window == text.as_bytes()),
                        "{text:?} was written to {}",
                        record.display()
                    );
                }
            }
            ensure!(found_in_store, "the search found no seeded Memory in the store");

            // Sessions are each machine's own session files: the node's
            // Project lists what the node holds, read through its link, in the
            // right panel with its checkout in front.
            for (home, project, id) in [
                (fixture.screen_home(), &node_project, "node-session"),
                (fixture.core_home(), &core_project, "core-session"),
            ] {
                write_claude_session(home, project, id)?;
            }
            focus_checkout(fixture, &mut socket, &node, &node_project).await?;
            send(&mut socket, "sessions_refresh", json!({})).await?;
            let rows = tokio::task::block_in_place(|| {
                wait_for("the right panel's sessions of the node's checkout", || {
                    let sessions = fixture.snapshot()?["sessions"].clone();
                    Ok((sessions["checkout_path"] == node_project.to_string_lossy().as_ref()
                        && sessions["loading"] == false)
                        .then(|| session_ids(&sessions)))
                })
            })?;
            ensure!(rows == ["node-session"], "the right panel listed {rows:?}");

            tokio::task::block_in_place(|| fixture.ssh.online(false))?;
            node_link(port, "waiting", LINK_BOUND).await?;
            let (answer, took) = tokio::task::block_in_place(|| {
                ask_memory(fixture.screen_command(&hide), &node_project)
            })?;
            ensure!(
                answer["ok"] == false && answer["reason"] == "hide_unavailable",
                "the node's agent was answered while the link was down: {answer}"
            );
            ensure!(took < Duration::from_secs(2), "the refusal took {took:?}");
            tokio::task::block_in_place(|| fixture.ssh.online(true))?;
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// A Claude Code session `id` under `home` whose one request was made in
/// `project`.
fn write_claude_session(home: &std::path::Path, project: &std::path::Path, id: &str) -> Result<()> {
    let folder = home.join(".claude/projects/journey");
    std::fs::create_dir_all(&folder)?;
    let line = json!({
        "type": "user", "cwd": std::fs::canonicalize(project)?,
        "timestamp": "2026-10-11T01:00:00Z", "userType": "external", "promptId": "p-1",
        "message": {"role": "user", "content": format!("{id} request")},
    });
    std::fs::write(folder.join(format!("{id}.jsonl")), format!("{line}\n"))?;
    Ok(())
}

/// Every file under `folder`, at any depth.
fn files_under(folder: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                folders.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}

/// The ids of the session rows a Sessions section lists.
fn session_ids(sessions: &Value) -> Vec<String> {
    sessions["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["id"].as_str().map(str::to_owned))
        .collect()
}

/// The one JSON answer a `hide workspace` command printed among `text`.
fn workspace_answer(text: &str) -> Result<Value> {
    let start = text.find('{').context("no answer was printed")?;
    let mut answers = serde_json::Deserializer::from_str(&text[start..]).into_iter::<Value>();
    Ok(answers.next().context("no answer was printed")??)
}

fn capabilities(info: &Value) -> Vec<String> {
    info["result"]["capabilities"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|capability| capability.as_str().map(str::to_owned))
        .collect()
}

/// Sends `pane`'s grid from `socket` as a window's fit does.
async fn draw_at(socket: &mut Socket, pane: &str, cols: u16, rows: u16) -> Result<()> {
    for kind in ["terminal_viewport", "terminal_resize"] {
        send(
            socket,
            kind,
            json!({"pane_id": pane, "cols": cols, "rows": rows, "new_view": true}),
        )
        .await?;
    }
    Ok(())
}

/// Starts a loop in `pane` that writes the grid it runs at to `file`
/// every 100 ms as `rows cols count`, so the test reads the grid without
/// typing: a key typed to ask would itself be input, and would reach the
/// node's pane ahead of a resize that goes through the core. A file, not
/// the screen, because Herdr sends a screen as the cells that changed.
async fn watch_grid(socket: &mut Socket, pane: &str, file: &std::path::Path) -> Result<()> {
    let file = file.display();
    let line = format!(
        "n=0; while sleep 0.1; do n=$((n+1)); \
         echo \"$(stty size) $n\" > '{file}.tmp' && mv -f '{file}.tmp' '{file}'; done\r"
    );
    let keys = base64::engine::general_purpose::STANDARD.encode(line);
    send(
        socket,
        "key",
        json!({"pane_id": pane, "bytes_base64": keys}),
    )
    .await
}

/// One key from `socket` into `pane`: input, nothing the loop reads.
async fn type_space(socket: &mut Socket, pane: &str) -> Result<()> {
    let keys = base64::engine::general_purpose::STANDARD.encode(" ");
    send(
        socket,
        "key",
        json!({"pane_id": pane, "bytes_base64": keys}),
    )
    .await
}

/// The grid the loop wrote last, as `rows cols`, and its count.
fn written_grid(file: &std::path::Path) -> Option<(String, u64)> {
    let text = std::fs::read_to_string(file).ok()?;
    let mut parts = text.split_whitespace();
    let (rows, cols, count) = (parts.next()?, parts.next()?, parts.next()?);
    Some((format!("{rows} {cols}"), count.parse().ok()?))
}

/// Waits until the loop writes `expected`, within 20 s.
async fn grid_becomes(file: &std::path::Path, expected: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = None;
    while Instant::now() < deadline {
        last = written_grid(file);
        if last.as_ref().is_some_and(|(grid, _)| grid == expected) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("the pane ran at {last:?}, not {expected}")
}

/// Reads ten of the loop's samples (about a second) and requires each is
/// `expected`.
async fn grid_holds(file: &std::path::Path, expected: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut seen = std::collections::BTreeSet::new();
    while seen.len() < 10 {
        ensure!(
            Instant::now() < deadline,
            "the loop wrote {seen:?} samples only"
        );
        if let Some((grid, count)) = written_grid(file) {
            ensure!(grid == expected, "the pane ran at {grid}, not {expected}");
            seen.insert(count);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_pane_runs_at_the_grid_of_the_screen_that_last_typed_into_it() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let project = fixture.screen_home().join("project");
        let herdr_pane = fixture.screen.workspace_at(&project)?;
        let pane = format!("remote:{node}:pane:{herdr_pane}");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            // The screen machine's window types into the pane and draws it
            // wide: the pane takes its grid.
            type_and_read(&mut socket, &pane, "echo wide-\"ok\"", "wide-ok").await?;
            let grids = fixture.root.join("grid");
            watch_grid(&mut socket, &pane, &grids).await?;
            // The node tells the core of a pane's typing at most once a
            // second, so the second is counted from the wide window's last.
            let typed_wide = Instant::now();
            draw_at(&mut socket, &pane, 120, 33).await?;
            grid_becomes(&grids, "33 120").await.context("grid 1")?;
            // The core machine's window draws it narrow: the pane stays wide
            // while that window only looks.
            let (core_port, core_token) = fixture.core_screen();
            let mut window = screen_socket(core_port, &core_token).await?;
            first_snapshot(&mut window, Duration::from_secs(20)).await?;
            draw_at(&mut window, &pane, 70, 21).await?;
            grid_holds(&grids, "33 120").await.context("grid 2")?;
            // It types: the pane takes its grid.
            type_space(&mut window, &pane).await?;
            grid_becomes(&grids, "21 70").await.context("grid 3")?;
            // The wide window types again, once its last notice of typing is
            // a second old: the pane is wide again.
            tokio::time::sleep_until((typed_wide + Duration::from_millis(1100)).into()).await;
            type_space(&mut socket, &pane).await?;
            grid_becomes(&grids, "33 120").await.context("grid 4")?;
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// The next binary frame on `socket` within `bound`: its JSON header and
/// its bytes; text frames before it are skipped.
async fn next_bytes(socket: &mut Socket, bound: Duration) -> Result<(Value, Vec<u8>)> {
    let deadline = Instant::now() + bound;
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .context("no file bytes arrived")?;
        let message = tokio::time::timeout(left, socket.next())
            .await
            .context("no file bytes arrived")?;
        match message {
            Some(Ok(Message::Binary(frame))) => {
                let length = u32::from_be_bytes(frame[..4].try_into()?) as usize;
                let header = serde_json::from_slice(&frame[4..4 + length])?;
                return Ok((header, frame[4 + length..].to_vec()));
            }
            Some(Ok(Message::Text(text))) if text.contains("file_bytes") => {
                bail!("the read was refused: {text}")
            }
            Some(Ok(_)) => {}
            other => bail!("the screen socket ended: {other:?}"),
        }
    }
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_screen_reads_its_own_checkouts_files_without_the_core() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let project = fixture.screen_home().join("project");
        std::fs::write(project.join("notes.txt"), "read-on-this-machine")?;
        fixture.screen.workspace_at(&project)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            // The Explorer lists the checkout through the core, which opens
            // its root on this machine's node.
            send(
                &mut socket,
                "file_list",
                json!({"root": project, "path": project, "device_id": node}),
            )
            .await?;
            let deadline = Instant::now() + Duration::from_secs(20);
            loop {
                let frame = next_frame(&mut socket, deadline)
                    .await?
                    .context("the checkout was never listed")?;
                if frame["type"] == "directory_list" {
                    break;
                }
            }
            send(
                &mut socket,
                "file_bytes",
                json!({"request_id": "read-1", "path": project.join("notes.txt"), "device_id": node}),
            )
            .await?;
            let (header, bytes) = next_bytes(&mut socket, Duration::from_secs(20)).await?;
            ensure!(
                header["request_id"] == "read-1" && header["eof"] == true,
                "the read answered {header}"
            );
            ensure!(bytes == b"read-on-this-machine", "the file read {bytes:?}");
            // A path outside every checkout is the core's to refuse, as it
            // refuses any device's.
            send(
                &mut socket,
                "file_bytes",
                json!({"request_id": "read-2", "path": fixture.screen_home().join(".ssh/client"), "device_id": node}),
            )
            .await?;
            let refused = next_bytes(&mut socket, Duration::from_secs(20)).await;
            ensure!(refused.is_err(), "a path outside every checkout was read");
            socket.close(None).await?;
            Ok::<_, anyhow::Error>(())
        })?;
        let ended = wait_for("the screen's end on the node", || {
            let rows = fixture.node_log("node_daemon", "screen.ended")?;
            Ok((!rows.is_empty()).then_some(rows))
        })?;
        ensure!(
            ended.iter().any(|row| row["local_file_reads"] == 1),
            "the read went through the core: {ended:?}"
        );
        ensure!(
            !fixture.logs_mention("read-on-this-machine")?,
            "a file's content reached a log"
        );
        Ok(())
    })
}

/// Amendment 10: removing a node that dials in ends its link, and with it
/// its screens and every authority the link carried; the node registers
/// again on its next attach.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_removed_node_loses_its_link_and_registers_again_on_its_next_attach() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            tokio::task::block_in_place(|| {
                fixture.event("remove_device", json!({"device_id": node}))
            })?;
            let (code, reason) = close_of(&mut socket, Duration::from_secs(15)).await?;
            ensure!(
                (code, reason.as_str()) == (1012, "core_link_lost"),
                "the screen closed as {code} {reason}"
            );
            node_link(port, "live", LINK_BOUND)
                .await
                .context("the node did not attach again")?;
            tokio::task::block_in_place(|| {
                wait_for("the node's row again", || fixture.device(&node))
            })?;
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// B17: a screen that stops reading while its core sends it a large file
/// holds up neither another screen nor a pane of the core's machine, and is
/// closed as fallen behind, so its read fails rather than hang (R3).
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_screen_that_stops_reading_holds_up_no_other_screen() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let core_project = fixture.core_home().join("project");
        let core_pane = fixture.core.workspace_at(&core_project)?;
        let big = core_project.join("big.bin");
        std::fs::write(&big, vec![b'x'; 64 * 1024 * 1024])?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"path": core_project, "label": "core", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, CORE_NODE, &core_project).await?;
            type_and_read(&mut socket, &core_pane, "echo before-\"ok\"", "before-ok").await?;

            // A second screen asks the core for the file and reads no more.
            let mut stalled = screen_socket(port, &token).await?;
            first_snapshot(&mut stalled, Duration::from_secs(20)).await?;
            send(
                &mut stalled,
                "file_bytes",
                json!({"request_id": "big", "path": big}),
            )
            .await?;
            tokio::task::block_in_place(|| {
                wait_for("the stalled screen's backlog to be dropped", || {
                    let rows = fixture.node_log("node_daemon", "screen.fell_behind")?;
                    Ok((!rows.is_empty()).then_some(rows))
                })
            })?;
            let started = Instant::now();
            type_and_read(&mut socket, &core_pane, "echo during-\"ok\"", "during-ok").await?;
            ensure!(
                started.elapsed() < Duration::from_secs(10),
                "the core's pane answered the other screen after {:?}",
                started.elapsed()
            );

            // Reading again, the stalled screen finds itself closed, so the
            // read it waits for fails rather than hang, and a screen opened
            // again is drawn whole.
            let (code, reason) = close_of(&mut stalled, Duration::from_secs(30)).await?;
            ensure!(
                (code, reason.as_str()) == (1013, "screen_fell_behind"),
                "the stalled screen closed as {code} {reason}"
            );
            let mut again = screen_socket(port, &token).await?;
            first_snapshot(&mut again, Duration::from_secs(20)).await?;
            Ok::<_, anyhow::Error>(())
        })
    })
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_that_ends_leaves_no_attach_role_or_ssh_connection() -> Result<()> {
    Fixture::journey(|fixture| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        for (signal, name) in [(libc::SIGTERM, "SIGTERM"), (libc::SIGKILL, "SIGKILL")] {
            let (port, _) = fixture.start_node()?;
            runtime.block_on(node_link(port, "live", LINK_BOUND))?;
            ensure!(
                fixture.attach_processes()?.len() == 1 && fixture.ssh.open() == 1,
                "a live node has one attach role and one connection: {:?}, {}",
                fixture.attach_processes()?,
                fixture.ssh.open()
            );
            fixture.signal_node(signal)?;
            wait_for(&format!("nothing left after {name}"), || {
                Ok(
                    (fixture.attach_processes()?.is_empty() && fixture.ssh.open() == 0)
                        .then_some(()),
                )
            })
            .with_context(|| {
                format!(
                    "left after {name}: {:?}, {} connections",
                    fixture.attach_processes(),
                    fixture.ssh.open()
                )
            })?;
        }
        // A node that stops answering but keeps its connection open: the
        // attach role ends on its own once the node fell silent.
        let (port, _) = fixture.start_node()?;
        runtime.block_on(node_link(port, "live", LINK_BOUND))?;
        fixture.signal_running_node(libc::SIGSTOP)?;
        let stopped = Instant::now();
        let ended = loop {
            if fixture.attach_processes()?.is_empty() {
                break stopped.elapsed();
            }
            ensure!(
                stopped.elapsed() < Duration::from_secs(45),
                "the attach role outlived a silent node: {:?}",
                fixture.attach_processes()?
            );
            runtime.block_on(async { tokio::time::sleep(Duration::from_millis(250)).await });
        };
        fixture.signal_running_node(libc::SIGCONT)?;
        runtime.block_on(node_link(port, "live", LINK_BOUND))?;
        eprintln!("the attach role ended {ended:?} after its node fell silent");
        Ok(())
    })
}

/// Serves `body` to every request on a loopback port until the test ends.
fn loopback_page(body: String) -> Result<u16> {
    use std::io::{Read as _, Write as _};
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    Ok(port)
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .proxy(None)
        .build()
        .into()
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_page_an_agent_of_the_core_opens_shows_on_the_node_and_reaches_the_cores_loopback() -> Result<()>
{
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        // Its own repository, so the checkout is this folder and not the
        // repository the run folder sits in.
        let site = fixture.core_home().join("site");
        fixture.core.git_init(&site)?;
        fixture.create_workspace_on(CORE_NODE, &site)?;
        let nonce = format!("core-page-{}", std::process::id());
        let page = loopback_page(nonce.clone())?;
        let url = format!("http://localhost:{page}/");
        let hide = fixture.hided.with_file_name("hide");
        let open = |fixture: &Fixture| -> Result<Value> {
            let mut command = fixture.core_command(&hide);
            command
                .args(["browser", "open", url.as_str()])
                .current_dir(&site);
            let output = command.output()?;
            workspace_answer(&String::from_utf8_lossy(&output.stdout))
                .with_context(|| format!("stderr: {}", String::from_utf8_lossy(&output.stderr)))
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            // No window anywhere: the agent is told there is none to show
            // the page in. The core registers the new checkout from Herdr's
            // events on its own time, and until it has, an open answers
            // checkout_not_registered (#912).
            let answer = tokio::task::block_in_place(|| {
                wait_for("the core registered the site's checkout", || {
                    let answer = open(fixture)?;
                    Ok((answer["reason"] != "checkout_not_registered").then_some(answer))
                })
            })?;
            ensure!(
                answer["ok"] == false && answer["reason"] == "renderer_unavailable",
                "a page opened with no window: {answer}"
            );
            // The node's desktop window is the one the page shows in.
            let mut desktop = screen_socket_of(port, &token, "desktop").await?;
            first_snapshot(&mut desktop, Duration::from_secs(20)).await?;
            let answer = tokio::task::block_in_place(|| {
                wait_for("the page opened in the node's window", || {
                    let answer = open(fixture)?;
                    Ok((answer["ok"] == true).then_some(answer))
                })
            })?;
            let opened = &answer["result"];
            ensure!(
                opened["context"]["device_id"] == CORE_NODE
                    && opened["context"]["checkout_path"] == site.to_str().context("site")?,
                "the page opened elsewhere: {answer}"
            );
            let route = json!({
                "device_id": CORE_NODE,
                "checkout_path": site,
                "id": opened["view_id"],
                "load": opened["load"],
                "owner_pid": std::process::id(),
            })
            .to_string();
            let endpoint = format!("http://127.0.0.1:{port}/browser-route");
            let agent = http_agent();
            // The node's desktop host resolves it on its own machine's daemon.
            let resolved: Value = tokio::task::block_in_place(|| -> Result<Value> {
                let unauthorized = agent
                    .post(&endpoint)
                    .header("Content-Type", "application/json")
                    .send(route.as_bytes())?;
                ensure!(
                    unauthorized.status() == 401,
                    "a route was resolved without the node's token"
                );
                let mut answer = agent
                    .post(&endpoint)
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "application/json")
                    .send(route.as_bytes())?;
                let status = answer.status();
                let body = answer.body_mut().read_to_string()?;
                ensure!(status == 200, "the route answered {status}: {body}");
                Ok(serde_json::from_str(&body)?)
            })?;
            let routed = resolved["url"].as_str().context("routed url")?.to_owned();
            ensure!(
                resolved["source_url"] == url.as_str()
                    && resolved["load"] == opened["load"]
                    && routed != url
                    && !routed.contains(&format!(":{page}/")),
                "the page was not routed through the link: {resolved}"
            );
            // The routed address reaches the core machine's loopback page
            // through the link's SSH connection.
            let body = tokio::task::block_in_place(|| -> Result<String> {
                Ok(agent.get(&routed).call()?.body_mut().read_to_string()?)
            })?;
            ensure!(body == nonce, "the routed page answered {body:?}");
            // The window lets the page go: the route closes with it.
            tokio::task::block_in_place(|| -> Result<()> {
                let released = agent
                    .delete(&endpoint)
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "application/json")
                    .force_send_body()
                    .send(route.as_bytes())?;
                ensure!(
                    released.status() == 204,
                    "release answered {}",
                    released.status()
                );
                wait_for("the routed address closed", || {
                    Ok(agent.get(&routed).call().is_err().then_some(()))
                })
            })?;
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// The pane object named `id` anywhere in a core snapshot.
fn pane_row<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    match value {
        Value::Object(fields) => {
            if fields.get("id").and_then(Value::as_str) == Some(id)
                && fields.contains_key("servers")
            {
                return Some(value);
            }
            fields.values().find_map(|field| pane_row(field, id))
        }
        Value::Array(items) => items.iter().find_map(|item| pane_row(item, id)),
        _ => None,
    }
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_server_started_in_a_node_pane_is_that_panes_on_the_core() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let project = fixture.screen_home().join("project");
        let herdr_pane = fixture.screen.workspace_at(&project)?;
        let pane = format!("remote:{node}:pane:{herdr_pane}");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            // A listener started in the node's pane, in its checkout.
            let screen = type_and_read(
                &mut socket,
                &pane,
                "python3 -c 'import socket,time;s=socket.socket();s.bind((\"127.0.0.1\",0));s.listen();print(\"LISTEN\"+\"ING\"+str(s.getsockname()[1])+\"x\",flush=True);time.sleep(600)'",
                "LISTENING",
            )
            .await?;
            let listening: u16 = {
                let after = screen
                    .split("LISTENING")
                    .nth(1)
                    .context("the listener's port")?;
                let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().with_context(|| format!("{screen:?}"))?
            };
            // The core reads the node's listeners through its link and
            // gives the pane its server, as it does for its own panes.
            let mut last = Value::Null;
            tokio::task::block_in_place(|| {
                wait_for("the node pane's server on the core", || {
                    let snapshot = fixture.snapshot()?;
                    let row = pane_row(&snapshot, &pane).cloned().unwrap_or(Value::Null);
                    last = row.clone();
                    Ok(row["servers"]
                        .as_array()
                        .is_some_and(|servers| {
                            servers.iter().any(|server| server["port"] == listening)
                        })
                        .then_some(()))
                })
            })
            .with_context(|| format!("the pane's row: {last}"))?;
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// Uploads `bytes` from a screen as the web shell does, as one dropped file
/// for `pane`, after typing `cat ` there, then presses Enter: answers what
/// the pane drew once `expect` appeared.
async fn drop_into_cat(
    socket: &mut Socket,
    pane: &str,
    bytes: &[u8],
    expect: &str,
) -> Result<String> {
    type_and_read(socket, pane, "echo drop-\"ready\"", "drop-ready").await?;
    let key = |text: &str| json!({"pane_id": pane, "bytes_base64": base64::engine::general_purpose::STANDARD.encode(text)});
    send(socket, "key", key("cat ")).await?;
    let batch = batch_id();
    let stage = format!("{batch}-0");
    send(
        socket,
        "attachment_stage",
        json!({"request_id": stage, "name": "dropped.txt", "size": bytes.len(), "clipboard": false}),
    )
    .await?;
    let header = json!({"request_id": stage, "offset": 0, "eof": true}).to_string();
    let mut frame = (header.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(header.as_bytes());
    frame.extend_from_slice(bytes);
    socket.send(Message::Binary(frame.into())).await?;
    send(
        socket,
        "attachment_commit",
        json!({"request_id": batch, "pane_id": pane, "bracketed_paste": true, "clipboard": false, "stages": [stage]}),
    )
    .await?;
    let mut screen = String::new();
    let mut entered = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    while let Some(frame) = next_frame(socket, deadline).await? {
        ensure!(
            frame["type"] != "attachment_refused",
            "the drop was refused: {frame}"
        );
        for (from, bytes) in chunks(&frame) {
            if from == pane {
                screen.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        if !entered && plain(&screen).contains("dropped.txt") {
            send(socket, "key", key("\r")).await?;
            entered = true;
        }
        if plain(&screen).contains(expect) {
            return Ok(plain(&screen));
        }
    }
    bail!(
        "{expect} never came back on {pane}; the screen read: {:?}",
        plain(&screen)
    )
}

/// A fresh batch id in the UUID shape the daemon requires.
fn batch_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let hex = format!("{nanos:032x}");
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// The files a daemon's state folder stages uploads in.
fn staged_files(state: &std::path::Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(state.join("attachments"))
        .map(|entries| entries.flatten().map(|entry| entry.path()).collect())
        .unwrap_or_default()
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_file_dropped_on_the_node_stays_there_for_its_own_pane_and_crosses_once_for_the_cores()
-> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let project = fixture.screen_home().join("project");
        let herdr_pane = fixture.screen.workspace_at(&project)?;
        let pane = format!("remote:{node}:pane:{herdr_pane}");
        let core_project = fixture.core_home().join("project");
        let core_pane = fixture.core.workspace_at(&core_project)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            // Into the node's own pane: the file is pasted from where the
            // node staged it, and the core never holds a copy.
            let here = format!("node-drop-{}", std::process::id());
            drop_into_cat(&mut socket, &pane, here.as_bytes(), &here).await?;
            ensure!(
                staged_files(&fixture.node_state())
                    .iter()
                    .any(|path| std::fs::read(path).is_ok_and(|bytes| bytes == here.as_bytes())),
                "the node did not stage the file it pasted"
            );
            ensure!(
                staged_files(&fixture.core_state).is_empty(),
                "the core holds a copy of a file for the node's own pane: {:?}",
                staged_files(&fixture.core_state)
            );
            // Into a pane of the core's machine: the file reaches the core
            // once, and the node keeps no copy.
            send(
                &mut socket,
                "create_workspace",
                json!({"path": core_project, "label": "core", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, CORE_NODE, &core_project).await?;
            let there = format!("core-drop-{}", std::process::id());
            drop_into_cat(&mut socket, &core_pane, there.as_bytes(), &there).await?;
            ensure!(
                staged_files(&fixture.core_state)
                    .iter()
                    .any(|path| std::fs::read(path).is_ok_and(|bytes| bytes == there.as_bytes())),
                "the core did not stage the file for its own pane"
            );
            ensure!(
                !staged_files(&fixture.node_state())
                    .iter()
                    .any(|path| std::fs::read(path).is_ok_and(|bytes| bytes == there.as_bytes())),
                "the node kept a copy of a file it sent on"
            );
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// The core's row for `device`'s project at `path`, in that device's session.
fn device_project(snapshot: &Value, device: &str, path: &std::path::Path) -> Option<Value> {
    snapshot
        .pointer("/status/remote")
        .and_then(Value::as_array)?
        .iter()
        .filter(|status| status["target_id"] == device)
        .filter_map(|status| {
            status
                .pointer("/session/workspaces")
                .and_then(Value::as_array)
        })
        .flatten()
        .find(|row| row["path"] == path.to_str().unwrap_or_default())
        .cloned()
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_project_is_measured_on_the_node_as_a_local_one_is() -> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        // A repository of the node's, with a file whose size the
        // measurement has to count.
        let project = fixture.screen_home().join("project");
        fixture.screen.git_init(&project)?;
        std::fs::write(project.join("weight.bin"), vec![1_u8; 512 * 1024])?;
        fixture.screen.workspace_at(&project)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            let row = tokio::task::block_in_place(|| {
                wait_for("the node's Git project on the core", || {
                    Ok(device_project(&fixture.snapshot()?, &node, &project)
                        .filter(|row| row["is_git"] == true))
                })
            })?;
            // The node's screen shows the Projects tab, which asks for its
            // projects' sizes.
            send(
                &mut socket,
                "disk_watch",
                json!({"workspace_ids": [row["id"]]}),
            )
            .await?;
            let mut last = Value::Null;
            tokio::task::block_in_place(|| {
                wait_for("the node project's size", || {
                    let row = device_project(&fixture.snapshot()?, &node, &project)
                        .unwrap_or(Value::Null);
                    last = row["disk"].clone();
                    Ok(row["disk"]["total_bytes"]
                        .as_u64()
                        .filter(|bytes| *bytes >= 512 * 1024)
                        .map(|_| ()))
                })
            })
            .with_context(|| format!("the project's disk: {last}"))?;
            Ok::<_, anyhow::Error>(())
        })
    })
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_checkouts_pull_request_is_read_with_the_cores_login_by_repository_name() -> Result<()> {
    Fixture::journey(|fixture| {
        // The node's own account runs git, so the operator's configuration
        // (a signing key behind their agent) never reaches the fixture.
        let git = |folder: &std::path::Path, arguments: &[&str]| -> Result<String> {
            let output = fixture
                .screen
                .environment
                .command("/usr/bin/git")
                .args([
                    "-c",
                    "user.name=fixture",
                    "-c",
                    "user.email=fixture@example.invalid",
                ])
                .args(arguments)
                .current_dir(folder)
                .output()?;
            ensure!(output.status.success(), "git {arguments:?}: {output:?}");
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        };
        // A checkout of the node's on a branch with a pull request, whose
        // origin names its GitHub repository.
        let project = fixture.screen_home().join("pr-project");
        std::fs::create_dir_all(&project)?;
        git(&project, &["init", "-q", "-b", "feature"])?;
        git(&project, &["commit", "-q", "--allow-empty", "-m", "work"])?;
        git(
            &project,
            &["remote", "add", "origin", "git@github.com:acme/app.git"],
        )?;
        let head = git(&project, &["rev-parse", "HEAD"])?;
        // The core machine's `gh` answers only for that repository, named, and
        // for the signed-in account's login, which a PR read asks first.
        let gh = fixture.core_home().join("bin/gh");
        std::fs::write(
            &gh,
            format!(
                r#"#!/bin/sh
[ "$1 $2" = "auth status" ] && exit 0
[ "$1 $2" = "api user" ] && {{ echo viewer; exit 0; }}
[ "$GH_REPO" = "acme/app" ] || {{ echo "no repository named" >&2; exit 1; }}
case "$1 $2" in
  "pr list") case "$4" in
      all) echo '[{{"title":"Node PR","number":7,"headRefName":"feature","headRefOid":"{head}","isCrossRepository":false,"baseRefName":"main","state":"OPEN","reviewDecision":"","isDraft":false,"url":"https://github.com/acme/app/pull/7","mergedAt":null,"updatedAt":"2026-10-01T10:00:00Z","createdAt":"2026-10-01T10:00:00Z","closedAt":null,"closingIssuesReferences":[]}}]' ;;
      *) echo '[{{"number":7,"statusCheckRollup":[]}}]' ;;
    esac ;;
  "repo view") echo '{{"nameWithOwner":"acme/app","id":"R_1"}}' ;;
  "api graphql") echo '{{"data":{{"repository":{{"pullRequests":{{"nodes":[]}}}}}}}}' ;;
  *) echo '[]' ;;
esac
"#
            ),
        )?;
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755))?;
        }
        fixture.screen.workspace_at(&project)?;
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            let mut last = Value::Null;
            tokio::task::block_in_place(|| {
                wait_for("the node checkout's pull request", || {
                    let row = device_project(&fixture.snapshot()?, &node, &project)
                        .unwrap_or(Value::Null);
                    last = row["checkouts"].clone();
                    Ok(row["checkouts"]
                        .as_array()
                        .is_some_and(|rows| rows.iter().any(|row| row["pull_request"]["number"] == 7))
                        .then_some(()))
                })
            })
            .with_context(|| format!("the project's checkouts: {last}"))?;
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// A provider that says which arguments it started with and then waits, as
/// an idle agent does.
const SLEEP_PROVIDER: &str = r#"
#include <stdio.h>
#include <string.h>
#include <termios.h>
#include <unistd.h>
int main(int argc, char **argv) {
  if (argc > 1 && strcmp(argv[1], "--version") == 0) { puts("fixture provider"); return 0; }
  if (argc > 1 && strcmp(argv[1], "auth") == 0) { puts("{\"loggedIn\":false}"); return 0; }
  struct termios t;
  if (tcgetattr(0, &t) == 0) {
    t.c_lflag &= ~(ICANON | ECHO | IEXTEN);
    t.c_cc[VMIN] = 1; t.c_cc[VTIME] = 0;
    tcsetattr(0, TCSANOW, &t);
  }
  printf("SLEEP_FIXTURE");
  for (int i = 1; i < argc; i++) printf(" %s", argv[i]);
  puts(""); fflush(stdout);
  char byte;
  while (read(0, &byte, 1) == 1) {}
  return 0;
}
"#;

/// The command lines running in the foreground of a pane of `herdr`.
fn foreground(
    herdr: &crate::support::remote_core::Herdr,
    pane: &str,
) -> Result<(Value, Vec<String>)> {
    let info = herdr.run(&["pane", "process-info", "--pane", pane])?;
    let info = info["result"]["process_info"].clone();
    let commands = info["foreground_processes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|process| {
            let argv = process["argv"].as_array()?;
            Some(
                argv.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        })
        .collect();
    Ok((info, commands))
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn an_agent_in_a_node_pane_sleeps_and_wakes_there_as_the_cores_own_does() -> Result<()> {
    const SESSION: &str = "11111111-2222-3333-4444-555555555555";
    Fixture::journey(|fixture| {
        let source = fixture.root.join("sleep-provider.c");
        std::fs::write(&source, SLEEP_PROVIDER)?;
        let mut compiler = fixture.screen.environment.command("/usr/bin/cc");
        compiler
            .arg("-O1")
            .arg(&source)
            .arg("-o")
            .arg(fixture.screen_home().join("bin/claude"));
        ensure!(
            compiler.status()?.success(),
            "the fixture provider compiles"
        );
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let project = fixture.screen_home().join("project");
        let herdr_pane = fixture.screen.workspace_at(&project)?;
        let pane = format!("remote:{node}:pane:{herdr_pane}");
        wait_for("the provider started in the node's pane", || {
            Ok(fixture
                .screen
                .run(&[
                    "agent",
                    "start",
                    "sleeper",
                    "--kind",
                    "claude",
                    "--pane",
                    &herdr_pane,
                ])
                .ok())
        })?;
        fixture.screen.write(&[
            "pane",
            "report-agent-session",
            &herdr_pane,
            "--source",
            "herdr:claude",
            "--agent",
            "claude",
            "--agent-session-id",
            SESSION,
            "--seq",
            "1",
        ])?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let mut socket = screen_socket(port, &token).await?;
            first_snapshot(&mut socket, Duration::from_secs(20)).await?;
            send(
                &mut socket,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "node", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut socket, &node, &project).await?;
            let row = |snapshot: &Value| pane_row(snapshot, &pane).cloned().unwrap_or(Value::Null);
            let mut last = Value::Null;
            // The node's pane offers Sleep agent as the core's own panes do.
            tokio::task::block_in_place(|| {
                wait_for("Sleep agent offered on the node's pane", || {
                    last = row(&fixture.snapshot()?);
                    Ok((last["sleep_action"]["available"] == true).then_some(()))
                })
            })
            .with_context(|| format!("the pane's row: {last}"))?;
            send(&mut socket, "agent_sleep", json!({"pane_id": pane})).await?;
            // The agent ended on the node's machine, its shell holds the
            // pane, and the core draws the pane asleep.
            tokio::task::block_in_place(|| {
                wait_for("the node's pane asleep", || {
                    last = row(&fixture.snapshot()?);
                    Ok((last["sleep"]["state"] == "sleeping").then_some(()))
                })
            })
            .with_context(|| format!("the pane's row: {last}"))?;
            let (info, _) = foreground(&fixture.screen, &herdr_pane)?;
            ensure!(
                info["foreground_process_group_id"] == info["shell_pid"],
                "the pane's shell holds the terminal once its agent sleeps: {info}"
            );
            // The node's Herdr bringing another tab forward and then the
            // sleeper's again is a visit, which wakes the same conversation
            // in the same pane.
            let workspace = fixture.screen.run(&["pane", "get", &herdr_pane])?;
            let workspace_id = workspace["result"]["pane"]["workspace_id"]
                .as_str()
                .context("the node pane's workspace")?
                .to_owned();
            let tab_id = workspace["result"]["pane"]["tab_id"]
                .as_str()
                .context("the node pane's tab")?
                .to_owned();
            let other = fixture.screen.run(&[
                "tab",
                "create",
                "--workspace",
                &workspace_id,
                "--cwd",
                &project.to_string_lossy(),
                "--label",
                "other",
                "--focus",
            ])?;
            let other_pane = format!(
                "remote:{node}:pane:{}",
                other["result"]["root_pane"]["pane_id"]
                    .as_str()
                    .context("the other tab's pane")?
            );
            tokio::task::block_in_place(|| {
                wait_for("the other tab in front on the core", || {
                    Ok(pane_row(&fixture.snapshot()?, &other_pane).map(|_| ()))
                })
            })?;
            ensure!(
                row(&fixture.snapshot()?)["sleep"]["state"] == "sleeping",
                "leaving the tab wakes nothing"
            );
            fixture.screen.run(&["tab", "focus", &tab_id])?;
            let mut commands = Vec::new();
            tokio::task::block_in_place(|| {
                wait_for("the conversation resumed in the node's pane", || {
                    commands = foreground(&fixture.screen, &herdr_pane)?.1;
                    Ok(commands
                        .iter()
                        .any(|command| command.contains(&format!("--resume {SESSION}")))
                        .then_some(()))
                })
            })
            .with_context(|| format!("the pane runs {commands:?}"))?;
            tokio::task::block_in_place(|| {
                wait_for("the node's pane awake on the core", || {
                    last = row(&fixture.snapshot()?);
                    Ok(last["sleep"].is_null().then_some(()))
                })
            })
            .with_context(|| format!("the pane's row: {last}"))?;
            Ok::<_, anyhow::Error>(())
        })
    })
}

/// What the scripted desktop gateway saw.
#[derive(Default)]
struct GatewaySeen {
    /// CDP text bytes it read and wrote.
    bytes: usize,
    /// The CDP methods it was asked, in order.
    methods: Vec<String>,
    /// Revocations its daemon asked for.
    revoked: usize,
}

type Seen = std::sync::Arc<std::sync::Mutex<GatewaySeen>>;

/// A desktop window's CDP gateway on the node's machine, as `browserCdp.ts`
/// serves one: `/connect` hands out a capability on its own loopback port,
/// `/revoke` drops them all, and a capability's socket answers the CDP a
/// page command sends with a page that has one button. Answers its address
/// and private token.
async fn desktop_gateway(seen: Seen) -> Result<(String, String)> {
    use axum::extract::ws::{Message as Frame, WebSocketUpgrade};
    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::{get, post};

    #[derive(Clone)]
    struct Gateway {
        endpoint: String,
        token: String,
        seen: Seen,
        capabilities: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
    }
    fn answer(request: &Value) -> Value {
        let expression = request["params"]["expression"].as_str().unwrap_or("");
        // The page script's operation, as `call` names it after the script.
        let op = |name: &str| expression.contains(&format!(")(\"{name}\","));
        let value = |value: Value| json!({"result": {"value": value}});
        let result = match request["method"].as_str().unwrap_or("") {
            "Target.getTargets" => json!({"targetInfos": [
                {"type": "page", "targetId": "page-1", "url": "http://page.test/"}]}),
            "Target.attachToTarget" => json!({"sessionId": "top"}),
            "Runtime.evaluate" if expression == "0" => {
                json!({"result": {"type": "number", "value": 0}})
            }
            "Runtime.evaluate" if op("probe") => value(json!({"width": 800, "height": 600,
                "dpr": 1, "vv": {"left": 0, "top": 0, "scale": 1}})),
            "Runtime.evaluate" if op("rect") => value(json!({"centerX": 10, "centerY": 10})),
            "Runtime.evaluate" if op("activeOpaqueFrame") => value(json!({"opaque": false})),
            "Runtime.evaluate" if op("baseline") => {
                value(json!({"previous": "# T\n# http://page.test/\n"}))
            }
            "Runtime.evaluate" if op("waitText") => value(json!({"found": true})),
            "Runtime.evaluate" => value(json!("# T\n# http://page.test/\n\n@1 button \"Go\"\n")),
            _ => json!({}),
        };
        let mut reply = json!({"id": request["id"], "result": result});
        if let Some(session) = request["sessionId"].as_str() {
            reply["sessionId"] = json!(session);
        }
        reply
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = format!("http://{}", listener.local_addr()?);
    let token = format!(
        "{:032x}{:032x}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    );
    let gateway = Gateway {
        endpoint: endpoint.clone(),
        token: token.clone(),
        seen,
        capabilities: Default::default(),
    };
    let authorized = |gateway: &Gateway, headers: &HeaderMap| {
        headers
            .get("authorization")
            .and_then(|value| value.to_str().ok())
            == Some(format!("Bearer {}", gateway.token).as_str())
    };
    let app = axum::Router::new()
        .route(
            "/connect",
            post(move |State(gateway): State<Gateway>, headers: HeaderMap| async move {
                if !authorized(&gateway, &headers) {
                    return StatusCode::UNAUTHORIZED.into_response();
                }
                let path = format!("/cdp/{:040x}", rand_id());
                gateway.capabilities.lock().unwrap().push(path.clone());
                axum::Json(json!({
                    "cdp_http_url": format!("{}{path}", gateway.endpoint),
                    "browser_ws_url": format!("{}{path}/devtools/browser", gateway.endpoint.replace("http:", "ws:")),
                }))
                .into_response()
            }),
        )
        .route(
            "/revoke",
            post(move |State(gateway): State<Gateway>, headers: HeaderMap| async move {
                if !authorized(&gateway, &headers) {
                    return StatusCode::UNAUTHORIZED;
                }
                gateway.capabilities.lock().unwrap().clear();
                gateway.seen.lock().unwrap().revoked += 1;
                StatusCode::OK
            }),
        )
        .route(
            "/cdp/{capability}/devtools/browser",
            get(
                |State(gateway): State<Gateway>,
                 Path(capability): Path<String>,
                 upgrade: WebSocketUpgrade| async move {
                    let path = format!("/cdp/{capability}");
                    if !gateway.capabilities.lock().unwrap().contains(&path) {
                        return StatusCode::FORBIDDEN.into_response();
                    }
                    upgrade.on_upgrade(move |mut socket| async move {
                        while let Some(Ok(Frame::Text(text))) = socket.recv().await {
                            let Ok(request) = serde_json::from_str::<Value>(text.as_str()) else {
                                return;
                            };
                            let reply = answer(&request).to_string();
                            {
                                let mut seen = gateway.seen.lock().unwrap();
                                seen.bytes += text.len() + reply.len();
                                seen.methods
                                    .push(request["method"].as_str().unwrap_or("").to_owned());
                            }
                            if socket.send(Frame::Text(reply.into())).await.is_err() {
                                return;
                            }
                        }
                    })
                },
            ),
        )
        .with_state(gateway);
    tokio::spawn(async move { axum::serve(listener, app).await });
    Ok((endpoint, token))
}

fn rand_id() -> u128 {
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes).expect("random bytes");
    u128::from_le_bytes(bytes)
}

/// The bytes the relays of `way` carried, as `rows` logged them.
fn relayed(rows: &[Value], way: &str) -> u64 {
    rows.iter()
        .filter(|row| row["way"] == way)
        .filter_map(|row| row["bytes"].as_u64())
        .sum()
}

/// Runs `hide <args>` in `herdr`'s `pane` with the state folder of the
/// pane's machine, and answers what it printed: the command is the
/// pane's own, proven as that pane.
fn hide_in_pane(
    (herdr, pane): (&Herdr, &str),
    (state, hide): (&std::path::Path, &std::path::Path),
    args: &str,
    out: &std::path::Path,
) -> Result<String> {
    let done = out.with_extension("done");
    let _ = std::fs::remove_file(&done);
    let line = format!(
        "HIDE_STATE_DIR='{}' '{}' {args} > '{}' 2>&1; touch '{}'",
        state.display(),
        hide.display(),
        out.display(),
        done.display()
    );
    tokio::task::block_in_place(|| {
        herdr.write(&["pane", "run", pane, &line])?;
        wait_for(&format!("`hide {args}` finished in {pane}"), || {
            Ok(done.exists().then_some(()))
        })?;
        Ok(std::fs::read_to_string(out)?)
    })
}

/// The core's browser relay streams through a node's link, as it opened
/// them: each one is CDP of a caller crossing the link.
fn relay_streams(fixture: &Fixture) -> Result<usize> {
    Ok(fixture
        .core_log("remote_host", "link_stream.opened")?
        .iter()
        .filter(|row| row["end"] == "browser_relay")
        .count())
}

/// B4, B13, B15: a page shown only in the node's desktop window is driven
/// from a pane on each machine. The node's pane is handed that machine's
/// own capability and relay, so none of its CDP crosses the link: the core
/// opens no browser relay stream, and every byte the gateway carried went
/// through the node's own relay. The core's pane is relayed through the
/// link to the same window. A page action the window asks for runs on the
/// core as that window. Revocation follows the link: when it ends, the
/// gateway is asked to drop what it handed out (the node pane's capability
/// stops working) and the node's pane is turned away; once the link and
/// the window are back the core's pane drives the page again.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_node_window_s_page_is_driven_from_its_machine_off_the_link_and_from_the_core_through_it()
-> Result<()> {
    Fixture::journey(|fixture| {
        let (port, token) = fixture.start_node()?;
        let node = herdr_core::node::NodeId::of_this_machine()
            .map_err(anyhow::Error::msg)?
            .as_str()
            .to_owned();
        let project = fixture.screen_home().join("project");
        let site = fixture.core_home().join("site");
        std::fs::create_dir_all(&project)?;
        fixture.core.git_init(&site)?;
        let page = loopback_page("page".to_owned())?;
        let url = format!("http://localhost:{page}/");
        let hide = fixture.hided.with_file_name("hide");
        let node_side = (fixture.node_state(), hide.clone());
        let core_side = (fixture.core_state.clone(), hide.clone());
        let out = fixture.root.join("pane-out");
        let seen = Seen::default();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async {
            node_link(port, "live", LINK_BOUND).await?;
            let node_pane = fixture.screen.workspace_at(&project)?;
            let core_pane = fixture.core.workspace_at(&site)?;
            let (endpoint, gateway_token) = desktop_gateway(std::sync::Arc::clone(&seen)).await?;
            let mut desktop = screen_socket_of(port, &token, "desktop").await?;
            first_snapshot(&mut desktop, Duration::from_secs(20)).await?;
            let drained = tokio::spawn(async move {
                while let Some(Ok(_)) = desktop.next().await {}
            });
            // The screen the panes are typed into and read from.
            let mut screen = screen_socket(port, &token).await?;
            first_snapshot(&mut screen, Duration::from_secs(20)).await?;
            send(
                &mut screen,
                "create_workspace",
                json!({"device_id": node, "path": project, "label": "page", "initialize_git": false}),
            )
            .await?;
            focus_checkout(fixture, &mut screen, &node, &project).await?;
            send(
                &mut screen,
                "create_workspace",
                json!({"path": site, "label": "site", "initialize_git": false}),
            )
            .await?;
            // The window registers its gateway with its own machine's daemon,
            // as it does with a core's; the registration stays there.
            let registration = json!({
                "owner_pid": std::process::id(),
                "endpoint": endpoint,
                "token": gateway_token,
            })
            .to_string();
            tokio::task::block_in_place(|| -> Result<()> {
                let answer = http_agent()
                    .post(format!("http://127.0.0.1:{port}/browser-control"))
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "application/json")
                    .send(registration.as_bytes())?;
                ensure!(answer.status() == 204, "registration answered {}", answer.status());
                Ok(())
            })?;
            // A page opened from the node's pane is connected to its own
            // window: the capability is on that machine's loopback.
            let node_in = (node_side.0.as_path(), node_side.1.as_path());
            let opened = workspace_answer(&hide_in_pane(
                (&fixture.screen, &node_pane),
                node_in,
                &format!("browser open {url}"),
                &out,
            )?)?;
            ensure!(opened["ok"] == true, "the node pane's page did not open: {opened}");
            // The core may not have heard of the window when the page
            // opened; the pane connects to the display once it has.
            let deadline = Instant::now() + LINK_BOUND;
            let mut connection = opened["result"].clone();
            while !connection["cdp_http_url"].is_string() {
                ensure!(
                    Instant::now() < deadline
                        && connection["browser_control"]["reason"] == "browser_control_unavailable",
                    "the node pane's page was never connected: {connection}"
                );
                tokio::time::sleep(Duration::from_millis(500)).await;
                let display = opened["result"]["view_id"].as_str().context("display")?;
                let answer = workspace_answer(&hide_in_pane(
                    (&fixture.screen, &node_pane),
                    node_in,
                    &format!("browser connect --display {display}"),
                    &out,
                )?)?;
                connection = if answer["ok"] == true {
                    answer["result"].clone()
                } else {
                    json!({"browser_control": answer})
                };
            }
            ensure!(
                connection["cdp_http_url"]
                    .as_str()
                    .is_some_and(|capability| capability.starts_with(&format!("{endpoint}/cdp/"))),
                "the node pane was not handed its own window's capability: {connection}"
            );
            let node_display = opened["result"]["view_id"].as_str().context("display")?.to_owned();
            let area = opened["result"]["area_id"].as_str().context("area")?.to_owned();
            let capability = connection["browser_ws_url"]
                .as_str()
                .context("capability")?
                .to_owned();
            let snapshot = hide_in_pane(
                (&fixture.screen, &node_pane),
                node_in,
                &format!("browser snapshot {node_display}"),
                &out,
            )?;
            ensure!(snapshot.contains("@1 button \"Go\""), "the node pane's snapshot: {snapshot}");
            let clicked = hide_in_pane(
                (&fixture.screen, &node_pane),
                node_in,
                &format!("browser click {node_display} @1 --no-verify"),
                &out,
            )?;
            ensure!(workspace_answer(&clicked)?["ok"] == true, "the node pane's click: {clicked}");
            ensure!(
                seen.lock().unwrap().methods.iter().any(|method| method == "Input.dispatchMouseEvent"),
                "the click never reached the window"
            );
            // The count: the core opened no browser relay stream through the
            // link (it logs one as it opens it, before its caller hears
            // back), and every CDP byte the gateway carried went through the
            // node's own relay.
            ensure!(
                relay_streams(fixture)? == 0,
                "the node pane's CDP crossed the link"
            );
            // Each relay logs as it ends, which may be after the command
            // printed.
            let mut last = (0, 0);
            tokio::task::block_in_place(|| {
                wait_for("the node's relays to have carried what the gateway did", || {
                    let rows = fixture.node_log("browser_relay", "relay.ended")?;
                    last = (relayed(&rows, "node"), seen.lock().unwrap().bytes as u64);
                    Ok((last.1 > 0 && last.0 == last.1).then_some(()))
                })
            })
            .with_context(|| format!("the node relayed {} bytes, the gateway carried {}", last.0, last.1))?;
            ensure!(
                fixture.core_log("browser_relay", "relay.ended")?.is_empty(),
                "the core relayed the node pane's CDP"
            );
            // A page action the window asks for runs on the core as the
            // window: its own machine's daemon passes it on. A second page
            // takes the area first, so the selection is the action's doing;
            // a process that is no window here is refused.
            let second = workspace_answer(&hide_in_pane(
                (&fixture.screen, &node_pane),
                node_in,
                &format!("browser open {url}second"),
                &out,
            )?)?;
            ensure!(second["ok"] == true, "the second page: {second}");
            let page_action = |owner_pid: u32| -> Result<(u16, Value)> {
                let action = json!({
                    "owner_pid": owner_pid,
                    "device_id": node,
                    "checkout_path": project,
                    "area_id": area,
                    "action": "select",
                    "display_id": node_display,
                    "request_id": format!(
                        "{}-{:032x}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)?
                            .as_millis(),
                        rand_id()
                    ),
                })
                .to_string();
                let mut answer = http_agent()
                    .post(format!("http://127.0.0.1:{port}/browser-control/action"))
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Content-Type", "application/json")
                    .send(action.as_bytes())?;
                let status = answer.status().as_u16();
                let body: Value = serde_json::from_str(&answer.body_mut().read_to_string()?)?;
                Ok((status, body))
            };
            let (status, refusal) = tokio::task::block_in_place(|| page_action(1))?;
            ensure!(
                status == 409 && refusal["reason"] == "browser_control_unavailable",
                "a process that is no window ran a page action: {status} {refusal}"
            );
            let selected = || -> Result<Value> {
                let views = workspace_answer(&hide_in_pane(
                    (&fixture.screen, &node_pane),
                    node_in,
                    "view list",
                    &out,
                )?)?;
                // Delivery is offered only to a caller proven as a pane.
                ensure!(
                    views["result"]["capabilities"]
                        .as_array()
                        .is_some_and(|offered| offered.iter().any(|name| name == "inbox")),
                    "the node's commands did not run as its pane: {views}"
                );
                Ok(views["result"]["views"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|view| view["area_id"] == area.as_str() && view["selected"] == true)
                    .map(|view| view["view_id"].clone())
                    .unwrap_or_default())
            };
            ensure!(selected()? == second["result"]["view_id"], "the second page did not take the area");
            let (status, answer) = tokio::task::block_in_place(|| page_action(std::process::id()))?;
            ensure!(
                status == 200 && answer["ok"] == true && answer["result"]["view_id"] == node_display.as_str(),
                "the window's page action answered {status}: {answer}"
            );
            ensure!(selected()? == node_display.as_str(), "the window's selection did not take effect");
            // A page of the core's checkout, driven from the core's pane: the
            // only window is the node's, so the core relays through the link
            // to it, and hands out no capability URL of another machine.
            focus_checkout(fixture, &mut screen, CORE_NODE, &site).await?;
            let core_in = (core_side.0.as_path(), core_side.1.as_path());
            let opened = workspace_answer(
                &hide_in_pane((&fixture.core, &core_pane), core_in, &format!("browser open {url}"), &out)?,
            )?;
            ensure!(
                opened["ok"] == true
                    && opened["result"]["browser_control"]["reason"] == "browser_control_elsewhere"
                    && opened["result"]["browser_control"]["next_action"]
                        .as_str()
                        .is_some_and(|next| next.contains(&format!("hide browser snapshot {}", opened["result"]["view_id"].as_str().unwrap_or("?")))),
                "the core pane's page: {opened}"
            );
            let core_display = opened["result"]["view_id"].as_str().context("display")?.to_owned();
            let methods_before = seen.lock().unwrap().methods.len();
            let snapshot = hide_in_pane(
                (&fixture.core, &core_pane),
                core_in,
                &format!("browser snapshot {core_display}"),
                &out,
            )?;
            ensure!(snapshot.contains("@1 button \"Go\""), "the core pane's snapshot: {snapshot}");
            let clicked = hide_in_pane(
                (&fixture.core, &core_pane),
                core_in,
                &format!("browser click {core_display} @1 --no-verify"),
                &out,
            )?;
            ensure!(workspace_answer(&clicked)?["ok"] == true, "the core pane's click: {clicked}");
            ensure!(
                seen.lock().unwrap().methods[methods_before..]
                    .iter()
                    .any(|method| method == "Input.dispatchMouseEvent"),
                "the core pane's click never reached the window"
            );
            ensure!(
                relay_streams(fixture)? >= 2,
                "the core pane's commands opened no browser relay stream through the link"
            );
            let core_rows = tokio::task::block_in_place(|| {
                fixture.core_log_until("browser_relay", "relay.ended", |rows| rows.len() >= 2)
            })?;
            ensure!(
                relayed(&core_rows, "link") > 0,
                "the core pane was not relayed through the link: {core_rows:?}"
            );
            // The link ends: the window is asked to revoke what it handed
            // out, so the capability the node pane holds stops working, and
            // the node pane is turned away until the link is back.
            // The capability works while the link lives.
            let (mut held, _) = tokio_tungstenite::connect_async(capability.as_str()).await?;
            held.close(None).await?;
            tokio::task::block_in_place(|| fixture.ssh.online(false))?;
            node_link(port, "waiting", LINK_BOUND).await?;
            tokio::task::block_in_place(|| {
                wait_for("the window revoked its capabilities", || {
                    Ok((seen.lock().unwrap().revoked > 0).then_some(()))
                })
            })?;
            ensure!(
                tokio_tungstenite::connect_async(capability.as_str()).await.is_err(),
                "the node pane's capability still works after the link ended"
            );
            let refused = workspace_answer(&hide_in_pane(
                (&fixture.screen, &node_pane),
                node_in,
                &format!("browser snapshot {node_display}"),
                &out,
            )?)?;
            ensure!(
                refused["ok"] == false && refused["reason"] == "hide_unavailable",
                "the node pane was answered without a link: {refused}"
            );
            tokio::task::block_in_place(|| fixture.ssh.online(true))?;
            node_link(port, "live", LINK_BOUND).await?;
            // The window reattaches, as it does after its screen closed with
            // the link; its gateway's registration stayed on its machine and
            // reaches the new link's core.
            drained.abort();
            drop(screen);
            let mut desktop = screen_socket_of(port, &token, "desktop").await?;
            first_snapshot(&mut desktop, Duration::from_secs(20)).await?;
            let mut screen = screen_socket(port, &token).await?;
            first_snapshot(&mut screen, Duration::from_secs(20)).await?;
            focus_checkout(fixture, &mut screen, CORE_NODE, &site).await?;
            let mut last = String::new();
            let deadline = Instant::now() + LINK_BOUND;
            while !last.contains("@1 button \"Go\"") {
                ensure!(
                    Instant::now() < deadline,
                    "the core pane's page was not driven again: {last}"
                );
                last = hide_in_pane(
                    (&fixture.core, &core_pane),
                    core_in,
                    &format!("browser snapshot {core_display}"),
                    &out,
                )?;
            }
            Ok::<_, anyhow::Error>(())
        })
    })
}
