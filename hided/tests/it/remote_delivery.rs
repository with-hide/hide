//! Candidate CLI lane on the core's own engine. It crosses actual pane
//! ancestry, SSH, helper attestation over the node link and the durable
//! mailbox.
#![cfg(unix)]

use crate::support::remote_delivery as fixture;

use anyhow::{Context, Result, ensure};
use fixture::{Fixture, quote, wait_for};
use hide_core::delivery::ledger::State;
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

/// The Stop hook that ends a Claude Code turn. The core's turn for the pane
/// follows the hook (`docs/status-model.md`), so a turn a prompt hook opened
/// works until this runs, whatever the screen reads.
fn stop(binary: &std::path::Path, session: &str) -> String {
    format!(
        "printf '%s' {} | {} hook --runtime claude-code --event Stop",
        quote(
            serde_json::json!({
                "session_id": session,
                "hook_event_name": "Stop",
                "stop_hook_active": false,
            })
            .to_string()
        ),
        quote(binary)
    )
}

/// A prompt the operator typed.
const OPERATOR_PROMPT: &str = "please run the tests";

#[test]
#[ignore = "external lane requires this worktree's CLI binaries (scripts/verify-cargo.sh cli)"]
fn connected_remote_panes_receive_once_and_disconnect_keeps_the_same_letter_pending() -> Result<()>
{
    Fixture::journey(|fixture| {
        let remote_hide = fixture.remote.hide.clone();
        let remote_hooks = fixture.remote.hooks.clone();
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
        // The turn the bell opened ends.
        ensure!(
            fixture
                .remote
                .run_in_pane(&stop(&remote_hooks, "fixture-remote-session"))?
                .status
                == 0,
            "remote stop hook failed"
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
        // The turn the operator's prompt opened ends; the letter rings at rest.
        ensure!(
            fixture
                .remote
                .run_in_pane(&stop(&remote_hooks, "fixture-remote-session"))?
                .status
                == 0,
            "remote stop hook failed"
        );
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

/// with-hide/hide-private#654: a request for an agent on a device is belled
/// with no `--force` once its composer reads empty, though input reached its
/// pane after its report. The core held such a letter for its 60 minutes: a
/// key hide routed to the pane with no submission after it holds a letter on
/// a possible draft, only a composer read of a known agent version releases
/// it, and the core knew no version of an agent on a device. The device's
/// helper now says which Claude Code is installed there
/// (`Call::HookDiagnosis`), the core's composer checks hold that version as
/// passed (the fixture's seed), and the screen the stand-in draws reads empty.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries (scripts/verify-cargo.sh cli)"]
fn a_request_for_a_device_agent_that_reported_is_belled_without_force_once_its_composer_reads_empty()
-> Result<()> {
    Fixture::journey_installed(|fixture| {
        let remote_hide = fixture.remote.hide.clone();
        let remote_hooks = fixture.remote.hooks.clone();

        // The device's agent reports to the agent that started it.
        let report = fixture
            .remote
            .run_in_pane(&format!(
                "{} request send local-parent --intent bridge-report --kind report --body {}",
                quote(&remote_hide),
                quote("DEVICE_REPORT")
            ))?
            .json()?;
        ensure!(
            report["id"].as_str().is_some(),
            "the device agent's report was not recorded: {report}"
        );
        // Then input reaches its pane that no submission follows, as an
        // operator's key does: a Backspace the stand-in ignores, so its
        // composer stays empty on the screen.
        let key = fixture.local.run_in_pane(&format!(
            "{} pane send {} --key backspace",
            quote(&fixture.hide),
            quote(&fixture.remote.pane)
        ))?;
        ensure!(key.status == 0, "the key was not sent: {}", key.stderr);

        let request = fixture
            .local
            .run_in_pane(&format!(
                "{} request send remote-child --intent bridge-after-report --body {}",
                quote(&fixture.hide),
                quote("DELIVERY_AFTER_REPORT")
            ))?
            .json()?;
        let id = request["id"]
            .as_str()
            .context("the request's letter ID")?
            .to_owned();
        // The letter waits out the quiet period and the core's read of the
        // device's agent, then its composer; before this change it waited
        // for its deadline instead.
        let bell = fixture.rung_within(&id, Duration::from_secs(150))?;
        let delivered =
            fixture
                .remote
                .run_in_pane(&hook(&remote_hooks, "fixture-remote-session", &bell))?;
        let intake = context(&delivered.stdout)?;
        ensure!(
            delivered.status == 0
                && intake.contains(&id)
                && intake.contains("DELIVERY_AFTER_REPORT"),
            "the device agent's prompt hook did not receive the request: {intake}"
        );
        wait_for("durable delivery of the request", || {
            Ok(fixture
                .ledger()?
                .letters
                .iter()
                .any(|letter| letter.id == id && letter.state == State::Delivered && !letter.forced)
                .then_some(()))
        })
    })
}

/// A request the recipient took in and never answered stops being awaited when
/// the recipient's registration ends, and the sender, on the other machine,
/// reads that and the reason from `hide request show`; the log records it
/// without the body.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries (scripts/verify-cargo.sh cli)"]
fn ending_the_recipients_registration_ends_the_senders_answer_wait_and_says_why() -> Result<()> {
    Fixture::journey(|fixture| {
        let remote_hide = fixture.remote.hide.clone();
        let hide = quote(&fixture.hide);
        let recipient = fixture.local.registration.clone();
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

        fixture
            .local
            .run_in_pane(&format!("{hide} agent end {recipient}"))?
            .value()?;

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

#[test]
#[ignore = "external lane requires this worktree's CLI binaries (scripts/verify-cargo.sh cli)"]
fn a_spawn_with_machine_starts_the_agent_on_the_device_under_its_caller() -> Result<()> {
    Fixture::journey(|fixture| {
        let project = fixture.commit_remote_project()?;
        fixture.agent_ready_on_device("claude-code", ".claude")?;
        let spawn = format!(
            "{} agent spawn --parent here --machine remote --name remote-worker --intent spawn-remote-1 --kind claude --repo {} --branch remote-worker",
            quote(&fixture.hide),
            quote(&project)
        );
        // The fixture provider reports no native session of its own, which a
        // real provider's hook does as soon as it starts. Report it from the
        // pane Hide starts on the device so the spawn can finish.
        let before = fixture::ready_panes(&fixture.remote.environment.home)?;
        let finished = AtomicBool::new(false);
        let (local, remote) = (&mut fixture.local, &mut fixture.remote);
        let (first, spawned) = std::thread::scope(|scope| {
            let reporter = scope.spawn(|| -> Result<Option<String>> {
                let pane = wait_for("the spawned agent's pane on the device", || {
                    if finished.load(Ordering::Relaxed) {
                        return Ok(Some(None));
                    }
                    let ready = fixture::ready_panes(&remote.environment.home)?;
                    Ok(ready
                        .into_iter()
                        .find(|pane| !before.contains(pane))
                        .map(Some))
                })?;
                if let Some(pane) = &pane {
                    remote.report_session(&remote.core_pane(pane), "fixture-spawned-session")?;
                }
                Ok(pane)
            });
            let output = local.run_in_pane(&spawn);
            finished.store(true, Ordering::Relaxed);
            let reported = reporter
                .join()
                .map_err(|_| anyhow::anyhow!("reporter panicked"))?;
            let output = output?;
            Ok::<_, anyhow::Error>((output, reported?))
        })?;
        ensure!(
            first.status == 0,
            "remote spawn refused: {} {}",
            first.stdout,
            first.stderr
        );
        let spawned = spawned.context("the spawned agent's pane on the device")?;
        let screen_pane = fixture.remote.core_pane(&spawned);

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
        ensure!(
            child.pane == spawned || child.pane == screen_pane,
            "the child's pane is not the device pane its agent runs in: {} vs {spawned}",
            child.pane
        );
        ensure!(
            fixture::ready_panes(&fixture.local.environment.home)? == [fixture.local.pane.clone()],
            "the agent was also started on the caller's own machine"
        );
        // B8/B11: the spawn's workspace and tab names reach the receipt only
        // after placement has.
        wait_for("receipt-proven names on the device", || {
            let saved = fixture.ledger()?;
            let record = saved
                .spawns
                .iter()
                .find(|record| record.id == spawn_record.id)
                .context("naming receipt")?;
            let names = serde_json::to_value(record)?;
            Ok(
                (names["names"]["workspace"] == "applied" && names["names"]["tab"] == "applied")
                    .then_some(()),
            )
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
        let remote_hooks = fixture.remote.hooks.clone();
        let letter = fixture
            .local
            .run_in_pane(&format!(
                "{} request send remote-worker --intent lead-to-child --body {}",
                quote(&fixture.hide),
                quote("DELIVERY_TO_SPAWNED")
            ))?
            .json()?;
        let letter_id = letter["id"].as_str().context("lead letter ID")?.to_owned();
        let bell = fixture.rung(&letter_id)?;
        let delivered = fixture.remote.run_in(
            &screen_pane,
            &hook(&remote_hooks, "fixture-spawned-session", &bell),
        )?;
        let intake = context(&delivered.stdout)?;
        ensure!(
            intake.contains(&letter_id) && intake.contains("DELIVERY_TO_SPAWNED"),
            "the spawned agent did not receive its lead's letter: {intake}"
        );

        // The child's report reaches its caller over the device's bridge.
        let remote_hide = fixture.remote.hide.clone();
        let report = fixture
            .remote
            .run_in(
                &screen_pane,
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
#[ignore = "external lane requires this worktree's CLI binaries (scripts/verify-cargo.sh cli)"]
fn a_plain_shell_on_the_device_opens_its_folder_as_a_project() -> Result<()> {
    Fixture::journey(|fixture| {
        let remote_hide = fixture.remote.hide.clone();
        let folder = fixture.remote.environment.home.join("plain-open");
        std::fs::create_dir_all(folder.join("src"))?;
        // The run directory lives inside this checkout, whose repository
        // holds the account's home; the folder is a repository of its own,
        // as the fixture's project is.
        let mut init = fixture.remote.environment.command("/usr/bin/git");
        init.args(["-c", "init.defaultBranch=main", "-C"])
            .arg(&folder)
            .args(["init", "-q"]);
        fixture::successful(init)?;
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
        // The device asked its core; it never started a Hide of its own,
        // whatever folder its helper was installed in.
        let device_state = hide_kit::layout::default_state_dir(&fixture.remote.environment.home);
        ensure!(
            !device_state.join("hided.json").exists(),
            "the device started a Hide of its own at {}",
            device_state.display()
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
