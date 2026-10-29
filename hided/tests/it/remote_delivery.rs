//! Candidate CLI + pinned Herdr lane. It crosses actual pane ancestry, SSH,
//! helper attestation over the node link and the durable mailbox.
#![cfg(unix)]

use crate::support::remote_delivery as fixture;

use anyhow::{Context, Result, ensure};
use fixture::{Fixture, quote, wait_for};
use herdr_core::delivery::ledger::State;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

fn context(stdout: &str) -> Result<String> {
    if stdout.is_empty() {
        return Ok(String::new());
    }
    let output: Value = serde_json::from_str(stdout)?;
    Ok(output
        .pointer("/hookSpecificOutput/additionalContext")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned())
}

/// The `value` of a `hide agent` answer, which does not use the envelope of
/// the mailbox commands.
fn agent_value(output: fixture::PaneOutput) -> Result<Value> {
    ensure!(
        output.status == 0,
        "agent command failed: {}",
        output.stderr
    );
    let answer: Value = serde_json::from_str(&output.stdout).context("agent command JSON")?;
    ensure!(answer["ok"] == true, "agent command refused: {answer}");
    answer.get("value").cloned().context("agent command value")
}

/// The prompt hook of a turn whose submitted prompt is `prompt`: only the
/// turn the line Hide's doorbell typed opened receives the letter bodies
/// (`Fixture::rung`), any other prompt receives a count.
fn hook(binary: &std::path::Path, session: &str, prompt: &str) -> String {
    format!(
        "printf '%s' {} | {} hook --runtime claude-code --event UserPromptSubmit",
        quote(serde_json::json!({"session_id":session,"prompt":prompt}).to_string()),
        quote(binary)
    )
}

/// A prompt the operator typed.
const OPERATOR_PROMPT: &str = "please run the tests";

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn connected_remote_panes_receive_once_and_disconnect_keeps_the_same_letter_pending() -> Result<()>
{
    Fixture::journey(|fixture| {
        let remote_hide = fixture.remote.environment.home.join("bin/hide");
        let remote_hooks = std::fs::canonicalize(&remote_hide)?
            .parent()
            .context("installed private CLI directory")?
            .join("hide-agent-hooks");
        ensure!(
            remote_hooks.is_file(),
            "private remote kit lacks its hook sibling"
        );
        // A named reference can disappear while the pane stays alive. The
        // actual remote CLI must still send a new kernel proof and get guidance.
        let missing = fixture.remote.environment.home.join("gone-reference.json");
        ensure!(
            !missing.exists(),
            "missing-reference fixture already exists"
        );
        let guidance = fixture
            .remote
            .run_in_pane(&format!(
                "HIDE_CAP_REF={} {} workspace info --hook",
                quote(&missing),
                quote(&remote_hide),
            ))?
            .json()?;
        ensure!(
            guidance["context"]["device_id"] == "remote"
                && guidance["capabilities"]
                    .as_array()
                    .is_some_and(|caps| caps.iter().any(|cap| cap == "workspace.info")),
            "fresh remote proof did not return remote guidance: {guidance}"
        );

        let send = format!(
            "{} request send remote-child --intent bridge-1 --body {}",
            quote(&fixture.hide),
            quote("DELIVERY_REMOTE_ONCE")
        );
        let first = fixture.local.run_in_pane(&send)?.json()?;
        let first_id = first["id"].as_str().context("sent letter ID")?.to_owned();
        let inbox = fixture
            .remote
            .run_in_pane(&format!("{} inbox", quote(&remote_hide)))?
            .json()?;
        ensure!(
            inbox.to_string().contains(&first_id),
            "remote inbox cannot see its letter"
        );
        ensure!(
            fixture
                .ledger()?
                .letters
                .iter()
                .any(|letter| letter.id == first_id && letter.state == State::Pending),
            "manual remote inbox changed delivery state"
        );
        let bell = fixture.rung(&first_id)?;
        let delivered =
            fixture
                .remote
                .run_in_pane(&hook(&remote_hooks, "fixture-remote-session", &bell))?;
        ensure!(
            delivered.status == 0 && delivered.elapsed < Duration::from_secs(2),
            "connected prompt hook did not complete within its budget"
        );
        let intake = context(&delivered.stdout)?;
        ensure!(
            intake.contains(&first_id) && intake.contains("DELIVERY_REMOTE_ONCE"),
            "remote hook did not receive the real letter"
        );
        wait_for("durable remote delivery confirmation", || {
            Ok(fixture
                .ledger()?
                .letters
                .iter()
                .any(|letter| letter.id == first_id && letter.state == State::Delivered)
                .then_some(()))
        })?;
        let again =
            fixture
                .remote
                .run_in_pane(&hook(&remote_hooks, "fixture-remote-session", &bell))?;
        ensure!(
            again.status == 0 && context(&again.stdout)?.is_empty(),
            "remote hook repeated a delivered letter"
        );

        let watch = fixture
            .local
            .run_in_pane(&format!(
                "{} watch start remote-child",
                quote(&fixture.hide)
            ))?
            .json()?;
        let watch_id = watch["id"].as_str().context("started watch ID")?.to_owned();
        let report = fixture
            .remote
            .run_in_pane(&format!(
                "{} request send local-parent --intent bridge-report --kind report --body {}",
                quote(&remote_hide),
                quote("DELIVERY_LOCAL_REPORT")
            ))?
            .json()?;
        let report_id = report["id"]
            .as_str()
            .context("report letter ID")?
            .to_owned();
        ensure!(
            fixture
                .ledger()?
                .watches
                .iter()
                .any(|watch| watch.id == watch_id),
            "sending a remote report stopped its watch before intake"
        );
        let bell = fixture.rung(&report_id)?;
        let parent =
            fixture
                .local
                .run_in_pane(&hook(&fixture.hooks, "fixture-local-session", &bell))?;
        ensure!(
            context(&parent.stdout)?.contains(&report_id),
            "local parent did not receive the report sent over the node link"
        );
        wait_for("report confirmation stops watch", || {
            Ok((!fixture
                .ledger()?
                .watches
                .iter()
                .any(|watch| watch.id == watch_id))
            .then_some(()))
        })?;

        let pending = fixture
            .local
            .run_in_pane(&format!(
                "{} request send remote-child --intent bridge-2 --body {}",
                quote(&fixture.hide),
                quote("DELIVERY_AFTER_RECONNECT")
            ))?
            .json()?;
        let pending_id = pending["id"]
            .as_str()
            .context("pending letter ID")?
            .to_owned();
        fixture.ssh.online(false)?;
        wait_for("private SSH device disconnected", || {
            let snapshot = fixture.snapshot()?;
            Ok(snapshot
                .pointer("/status/remote")
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row["target_id"] == "remote" && row["state"] != "connected")
                })
                .then_some(()))
        })?;
        let disconnected = fixture.remote.run_in_pane(&hook(
            &remote_hooks,
            "fixture-remote-session",
            OPERATOR_PROMPT,
        ))?;
        ensure!(
            disconnected.status == 0,
            "disconnected prompt hook failed the provider turn"
        );
        ensure!(
            disconnected.elapsed < Duration::from_secs(2),
            "disconnected prompt hook exceeded two seconds"
        );
        ensure!(
            context(&disconnected.stdout)?.is_empty(),
            "disconnected prompt hook emitted a letter"
        );
        ensure!(
            fixture
                .ledger()?
                .letters
                .iter()
                .any(|letter| letter.id == pending_id && letter.state == State::Pending),
            "disconnect acknowledged or lost the pending letter"
        );

        fixture.ssh.online(true)?;
        fixture.reconnect_device()?;
        fixture.wait_bridge(2)?;
        let bell = fixture.rung(&pending_id)?;
        let resumed =
            fixture
                .remote
                .run_in_pane(&hook(&remote_hooks, "fixture-remote-session", &bell))?;
        let intake = context(&resumed.stdout)?;
        ensure!(
            resumed.status == 0
                && intake.contains(&pending_id)
                && intake.contains("DELIVERY_AFTER_RECONNECT"),
            "reconnected remote hook did not receive the same letter"
        );
        wait_for("reconnected delivery confirmation", || {
            Ok(fixture
                .ledger()?
                .letters
                .iter()
                .any(|letter| letter.id == pending_id && letter.state == State::Delivered)
                .then_some(()))
        })?;
        let replay = fixture
            .local
            .run_in_pane(&format!(
                "{} request send remote-child --intent bridge-2 --body {}",
                quote(&fixture.hide),
                quote("DELIVERY_AFTER_RECONNECT")
            ))?
            .json()?;
        ensure!(
            replay["id"] == pending_id,
            "same intent created a new letter after reconnect"
        );
        ensure!(
            fixture
                .ledger()?
                .letters
                .iter()
                .filter(|letter| letter.id == pending_id)
                .count()
                == 1,
            "reconnect duplicated a durable letter"
        );
        let once =
            fixture
                .remote
                .run_in_pane(&hook(&remote_hooks, "fixture-remote-session", &bell))?;
        ensure!(
            once.status == 0 && context(&once.stdout)?.is_empty(),
            "reconnect caused duplicate prompt intake"
        );
        ensure!(
            !fixture
                .remote
                .environment
                .home
                .join(".hide/state/delivery-ledger.json")
                .exists(),
            "remote account gained a mailbox spool"
        );
        Ok(())
    })
}

/// A request the recipient took in and never answered stops being awaited when
/// the recipient's registration ends, and the sender, on the other machine,
/// reads that and the reason from `hide request show`; the log records it
/// without the body.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn ending_the_recipients_registration_ends_the_senders_answer_wait_and_says_why() -> Result<()> {
    Fixture::journey(|fixture| {
        let remote_hide = fixture.remote.environment.home.join("bin/hide");
        let hide = quote(&fixture.hide);
        let recipient = agent_value(fixture.local.run_in_pane(&format!(
                "{hide} agent register --host-scope \"$HERDR_SOCKET_PATH\" \
                 --session fixture-local-session --instance {pane} --name local-parent --pane {pane}",
                pane = fixture.local.pane
            ))?)?["id"]
            .as_str()
            .context("the recipient's registration ID")?
            .to_owned();
        let sent = fixture
            .remote
            .run_in_pane(&format!(
                "{} request send local-parent --intent wait-ends-1 --body {}",
                quote(&remote_hide),
                quote("DELIVERY_WAIT_ENDS")
            ))?
            .json()?;
        let id = sent["id"].as_str().context("sent letter ID")?.to_owned();
        ensure!(
            sent["waiting_answer"] == true,
            "a new request awaits its answer"
        );
        let bell = fixture.rung(&id)?;
        let delivered =
            fixture
                .local
                .run_in_pane(&hook(&fixture.hooks, "fixture-local-session", &bell))?;
        ensure!(
            context(&delivered.stdout)?.contains(&id),
            "the recipient did not take the request in: status {} stdout {:?} stderr {:?}",
            delivered.status,
            delivered.stdout,
            delivered.stderr
        );
        wait_for("durable delivery confirmation", || {
            Ok(fixture
                .ledger()?
                .letters
                .iter()
                .any(|letter| letter.id == id && letter.state == State::Delivered)
                .then_some(()))
        })?;
        let show = format!("{} request show {id}", quote(&remote_hide));
        let waiting = fixture.remote.run_in_pane(&show)?.json()?;
        ensure!(
            waiting["waiting_answer"] == true && waiting["answer_wait_ended"].is_null(),
            "an awaited request already reports an end: {waiting}"
        );

        agent_value(
            fixture
                .local
                .run_in_pane(&format!("{hide} agent end {recipient}"))?,
        )?;

        let shown = fixture.remote.run_in_pane(&show)?.json()?;
        ensure!(
            shown["waiting_answer"] == false,
            "the wait did not end: {shown}"
        );
        ensure!(
            shown["answer_wait_ended"] == "party_ended",
            "the sender cannot read why: {shown}"
        );
        ensure!(shown["state"] == "delivered", "the state moved: {shown}");
        ensure!(
            shown["finished_at_unix_ms"].is_u64(),
            "the finished letter has no finish time: {shown}"
        );
        let log = fixture.state.join("Logs/core.jsonl");
        wait_for("the answer_wait.ended log line", || {
            if !log.exists() {
                return Ok(None);
            }
            let text = String::from_utf8(std::fs::read(&log)?)?;
            let line = text
                .lines()
                .find(|line| line.contains("answer_wait.ended") && line.contains(id.as_str()));
            Ok(line.map(|line| {
                assert!(
                    line.contains("party_ended") && !line.contains("DELIVERY_WAIT_ENDS"),
                    "the log line carries the reason and never the body: {line}"
                );
            }))
        })?;
        Ok(())
    })
}

/// The pane of the named agent in a Herdr answer, wherever the answer nests it.
fn agent_pane(value: &Value, name: &str) -> Option<(String, bool)> {
    match value {
        Value::Object(map) => {
            if map.get("name").and_then(Value::as_str) == Some(name)
                && let Some(pane) = map.get("pane_id").and_then(Value::as_str)
            {
                let reported = map
                    .get("agent_session")
                    .is_some_and(|session| !session.is_null());
                return Some((pane.to_owned(), reported));
            }
            map.values().find_map(|value| agent_pane(value, name))
        }
        Value::Array(rows) => rows.iter().find_map(|value| agent_pane(value, name)),
        _ => None,
    }
}

/// The agent row of a pane in a snapshot, wherever the snapshot nests it.
fn row_of_pane<'a>(value: &'a Value, pane: &str) -> Option<&'a Value> {
    match value {
        Value::Object(map) => {
            if map.get("pane_id").and_then(Value::as_str) == Some(pane)
                && map.contains_key("lineage_parent_pane_id")
            {
                return Some(value);
            }
            map.values().find_map(|value| row_of_pane(value, pane))
        }
        Value::Array(rows) => rows.iter().find_map(|value| row_of_pane(value, pane)),
        _ => None,
    }
}

/// Every string stored under `key`, wherever the answer nests it.
fn strings_under(value: &Value, key: &str, found: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (name, value) in map {
                match value.as_str() {
                    Some(text) if name == key => found.push(text.to_owned()),
                    _ => strings_under(value, key, found),
                }
            }
        }
        Value::Array(rows) => rows.iter().for_each(|row| strings_under(row, key, found)),
        _ => {}
    }
}

#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_spawn_with_machine_starts_the_agent_on_the_device_under_its_caller() -> Result<()> {
    Fixture::journey(|fixture| {
        let project = fixture.commit_remote_project()?;
        let remote_hide = fixture.remote.environment.home.join("bin/hide");
        let spawn = format!(
            "{} agent spawn --parent here --machine remote --name remote-worker --intent spawn-remote-1 --kind claude --repo {} --branch remote-worker",
            quote(&fixture.hide),
            quote(&project)
        );
        // The fixture provider reports no native session of its own, which a
        // real provider's hook does as soon as it starts. Report it for the
        // pane Hide starts so the spawn can finish.
        let finished = AtomicBool::new(false);
        let (local, remote) = (&mut fixture.local, &fixture.remote);
        let first = std::thread::scope(|scope| {
            let reporter = scope.spawn(|| -> Result<()> {
                let pane = wait_for("the spawned agent's pane on the device", || {
                    if finished.load(Ordering::Relaxed) {
                        return Ok(Some(None));
                    }
                    let agents = remote.run(&["agent", "list"])?;
                    Ok(agent_pane(&agents, "remote-worker").map(|(pane, _)| Some(pane)))
                })?;
                if let Some(pane) = pane {
                    remote.report_session(&pane, "fixture-spawned-session")?;
                }
                Ok(())
            });
            let output = local.run_in_pane(&spawn);
            finished.store(true, Ordering::Relaxed);
            let reported = reporter
                .join()
                .map_err(|_| anyhow::anyhow!("reporter panicked"))?;
            let output = output?;
            reported?;
            Ok::<_, anyhow::Error>(output)
        })?;
        ensure!(
            first.status == 0,
            "remote spawn refused: {} {}",
            first.stdout,
            first.stderr
        );

        let ledger = fixture.ledger()?;
        let spawn_record = ledger
            .spawns
            .iter()
            .find(|record| record.intent == "spawn-remote-1")
            .context("spawn receipt")?;
        ensure!(
            spawn_record.machine.as_deref() == Some("remote"),
            "receipt does not name the device"
        );
        let child_id = spawn_record
            .child
            .clone()
            .context("completed spawn child")?;
        let child = ledger
            .agents
            .iter()
            .find(|record| record.id == child_id)
            .context("child registration")?;
        let parent = ledger
            .agents
            .iter()
            .find(|record| record.id == spawn_record.parent)
            .context("caller registration")?;
        ensure!(
            child.machine == "remote" && child.parent.as_deref() == Some(parent.id.as_str()),
            "child is not the device's agent under the caller: {child:?}"
        );
        let device_agents = fixture.remote.run(&["agent", "list"])?;
        ensure!(
            agent_pane(&device_agents, "remote-worker").is_some_and(|(pane, _)| pane == child.pane),
            "the child's pane is not the device Herdr's agent pane"
        );
        ensure!(
            agent_pane(&fixture.local.run(&["agent", "list"])?, "remote-worker").is_none(),
            "the agent was also started on the caller's own Herdr"
        );
        // B8/B11: the real device server's workspace and tab leave their
        // recovery markers only after placement has reached the receipt.
        wait_for("receipt-proven names on the device", || {
            let state = fixture.remote.run(&["api", "snapshot"])?;
            let state = &state["result"]["snapshot"];
            let pane = state["panes"]
                .as_array()
                .context("device panes")?
                .iter()
                .find(|pane| pane["pane_id"] == child.pane)
                .context("child pane")?;
            let workspace = state["workspaces"]
                .as_array()
                .context("device workspaces")?
                .iter()
                .find(|workspace| workspace["workspace_id"] == pane["workspace_id"])
                .context("child workspace")?;
            let tab = state["tabs"]
                .as_array()
                .context("device tabs")?
                .iter()
                .find(|tab| tab["tab_id"] == pane["tab_id"])
                .context("child tab")?;
            let saved = fixture.ledger()?;
            let record = saved
                .spawns
                .iter()
                .find(|record| record.id == spawn_record.id)
                .context("naming receipt")?;
            let names = serde_json::to_value(record)?;
            Ok((workspace["label"] == "remote-worker"
                && tab["label"] == "remote-worker"
                && names["names"]["workspace"] == "applied"
                && names["names"]["tab"] == "applied")
                .then_some(()))
        })?;
        ensure!(
            ledger
                .watches
                .iter()
                .any(|watch| watch.target.device_id == "remote"
                    && watch.target.name == "remote-worker"
                    && Some(&watch.parent.name) == parent.name.as_ref()),
            "the caller has no watch on the device's agent"
        );

        // The device's Herdr holds the lineage: the child is its own pane there
        // and its token names this machine as the parent's.
        let snapshot = fixture.remote.run(&["api", "snapshot"])?;
        let mut machines = Vec::new();
        strings_under(&snapshot, "parent_machine", &mut machines);
        ensure!(
            machines.contains(&parent.native_machine),
            "device lineage token does not name the caller's machine: {machines:?} vs {}",
            parent.native_machine
        );

        // The same command returns the same agent and makes nothing twice.
        let again = fixture.local.run_in_pane(&spawn)?;
        ensure!(
            again.status == 0,
            "repeat spawn refused: {} {}",
            again.stdout,
            again.stderr
        );
        let after = fixture.ledger()?;
        ensure!(
            after.spawns.len() == ledger.spawns.len()
                && after.agents.len() == ledger.agents.len()
                && after.watches.len() == ledger.watches.len(),
            "the same intent made something twice"
        );

        // Another machine under the same intent is a different spawn.
        let moved = fixture
            .local
            .run_in_pane(&spawn.replace("--machine remote ", ""))?;
        ensure!(
            moved.status != 0 && moved.stdout.contains("intent_conflict"),
            "the same intent on another machine was not refused: {} {}",
            moved.stdout,
            moved.stderr
        );

        // This machine's screen places the device's agent under the lead.
        let (pane, _) = agent_pane(&fixture.remote.run(&["agent", "list"])?, "remote-worker")
            .context("spawned agent pane")?;
        let screen_pane = format!("remote:remote:pane:{pane}");
        let lead_pane = fixture.local.pane.clone();
        wait_for(
            "the device agent under its lead on the caller's screen",
            || {
                let snapshot = fixture.snapshot()?;
                Ok(row_of_pane(&snapshot, &screen_pane)
                    .filter(|row| {
                        row["delegated"] == true && row["lineage_parent_pane_id"] == lead_pane
                    })
                    .map(|_| ()))
            },
        )?;

        // The lead's letter reaches the device agent through its own hook.
        let remote_hooks = std::fs::canonicalize(&remote_hide)?
            .parent()
            .context("installed private CLI directory")?
            .join("hide-agent-hooks");
        let letter = fixture
            .local
            .run_in_pane(&format!(
                "{} request send remote-worker --intent lead-to-child --body {}",
                quote(&fixture.hide),
                quote("DELIVERY_TO_SPAWNED")
            ))?
            .json()?;
        let letter_id = letter["id"].as_str().context("lead letter ID")?.to_owned();
        let (pane, _) = agent_pane(&fixture.remote.run(&["agent", "list"])?, "remote-worker")
            .context("spawned agent pane")?;
        let bell = fixture.rung(&letter_id)?;
        let delivered = fixture.remote.run_in(
            &pane,
            &hook(&remote_hooks, "fixture-spawned-session", &bell),
        )?;
        let intake = context(&delivered.stdout)?;
        ensure!(
            intake.contains(&letter_id) && intake.contains("DELIVERY_TO_SPAWNED"),
            "the spawned agent did not receive its lead's letter: {intake}"
        );

        // The child's report reaches its caller over the device's bridge.
        let (pane, _) = agent_pane(&fixture.remote.run(&["agent", "list"])?, "remote-worker")
            .context("spawned agent pane")?;
        let report = fixture
            .remote
            .run_in(
                &pane,
                &format!(
                    "{} request send local-parent --intent spawn-report --kind report --body {}",
                    quote(&remote_hide),
                    quote("DELIVERY_SPAWNED_REPORT")
                ),
            )?
            .json()?;
        let report_id = report["id"]
            .as_str()
            .context("report letter ID")?
            .to_owned();
        let bell = fixture.rung(&report_id)?;
        let received =
            fixture
                .local
                .run_in_pane(&hook(&fixture.hooks, "fixture-local-session", &bell))?;
        ensure!(
            context(&received.stdout)?.contains(&report_id),
            "the caller did not receive the spawned agent's report"
        );
        Ok(())
    })
}

/// `hide <path>` from a plain shell on the device, an ssh login with no
/// Hide pane: its node proves it only as a process of its own account, the
/// core registers the device's folder as the device's helper judges it, and
/// the same command again only brings the project forward. A folder outside
/// the device's home is refused, since only Add a project asks about one.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_plain_shell_on_the_device_opens_its_folder_as_a_project() -> Result<()> {
    Fixture::journey(|fixture| {
        let remote_hide = fixture.remote.environment.home.join("bin/hide");
        let folder = fixture.remote.environment.home.join("plain-open");
        std::fs::create_dir_all(folder.join("src"))?;
        let open = |path: &std::path::Path| {
            let mut command = fixture.remote.environment.command(&remote_hide);
            command.arg(path);
            fixture::capture(command)
        };
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();

        let added = open(&folder)?;
        ensure!(
            added.status.success() && text(&added.stdout).starts_with("Added plain-open ("),
            "the device's folder was not added: {} {}",
            text(&added.stdout),
            text(&added.stderr)
        );
        let wire = folder.to_string_lossy().into_owned();
        wait_for("the device's folder registered on the core", || {
            let snapshot = fixture.snapshot()?;
            Ok(snapshot
                .pointer("/status/remote")
                .and_then(Value::as_array)
                .and_then(|rows| rows.iter().find(|row| row["target_id"] == "remote"))
                .and_then(|remote| remote.pointer("/session/workspaces"))
                .and_then(Value::as_array)
                .is_some_and(|rows| {
                    rows.iter()
                        .any(|row| row["registered"] == true && row["path"] == wire.as_str())
                })
                .then_some(()))
        })?;

        let again = open(&folder.join("src"))?;
        ensure!(
            again.status.success() && text(&again.stdout).starts_with("Brought plain-open ("),
            "a folder inside the project was not only brought forward: {} {}",
            text(&again.stdout),
            text(&again.stderr)
        );

        let outside = tempfile::tempdir()?;
        let refused = open(outside.path())?;
        ensure!(
            !refused.status.success()
                && text(&refused.stderr).contains("needs_confirmation")
                && text(&refused.stderr).contains("Add a project"),
            "a folder outside the device's home was not refused: {} {}",
            text(&refused.stdout),
            text(&refused.stderr)
        );
        Ok(())
    })
}
