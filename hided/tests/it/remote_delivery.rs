//! Candidate CLI lane on the core's own engine. It crosses actual pane
//! ancestry, SSH, helper attestation over the node link and the durable
//! mailbox.
#![cfg(unix)]

use crate::support::remote_delivery as fixture;

use anyhow::{Context, Result, ensure};
use fixture::{Fixture, hook_diagnostics, quote, wait_for, wait_within};
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

/// A hook's confirmation of the letter `argv[2]`, sent as the hook sends it
/// (`hide_agent_hooks::hook_request`) to the daemon whose socket record is
/// `argv[1]`, from the pane it runs in; prints the daemon's answer line.
const CONFIRM_FROM_PANE: &str = r#"
import json, os, secrets, socket, sys
path = open(sys.argv[1]).read().strip()
stream = socket.socket(socket.AF_UNIX)
stream.connect(path)
request = {"pane_id": os.environ.get("HIDE_PANE_ID", ""), "nonce": secrets.token_hex(16),
    "one_shot": True, "install": os.environ.get("HIDE_INSTALL") or None,
    "hook": {"request_id": "confirm-1", "token": None,
        "command": {"kind": "letter_confirm", "ids": [sys.argv[2]]}}}
stream.sendall((json.dumps(request) + "\n").encode())
print(stream.makefile().readline().strip())
"#;

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
            "{} letter send remote-child --intent bridge-1 --body {}",
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
        // A letter's confirmation is a hook request on the daemon's own
        // socket, which the core takes only from the letter's recipient
        // (with-hide/hide-private#663): the sender's pane, proved by that
        // socket, is refused, and the letter stays pending.
        let record = hide_agent_hooks::hook_request::bootstrap_socket_record(&fixture.state);
        let refused = fixture.local.run_in_pane(&format!(
            "python3 -c {} {} {}",
            quote(CONFIRM_FROM_PANE),
            quote(&record),
            quote(&first_id)
        ))?;
        let refused: Value = serde_json::from_str(refused.stdout.trim()).with_context(|| {
            format!(
                "the sender's confirmation: {} {}",
                refused.stdout, refused.stderr
            )
        })?;
        ensure!(
            refused["ok"] == false && refused["reason"] == "recipient_required",
            "the sender's pane confirmed the letter it sent: {refused}"
        );
        ensure!(
            fixture
                .ledger()?
                .letters
                .iter()
                .any(|letter| letter.id == first_id && letter.state == State::Pending),
            "a refused confirmation changed delivery state"
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
            "remote hook did not receive the real letter: after {:?}, stderr {:?}, recorded {}",
            delivered.elapsed,
            delivered.stderr,
            hook_diagnostics(&fixture.remote.environment)
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
        let remote_state = fixture.remote.environment.home.join(".hide/state");
        ensure!(
            !hide_kit::layout::delivery_store(&remote_state).exists()
                && !hide_kit::layout::delivery_ledger(&remote_state).exists(),
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

        // The recipient ends its own registration from its own pane, which
        // stays open to answer the command.
        fixture
            .local
            .run_in_pane(&format!("{hide} agent end {recipient} --keep-pane"))?
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

/// `command` run in `agent`'s pane, printed with its answer line as the
/// run's transcript (`--no-capture`), and its `result`.
fn transcribed(agent: &fixture::Agent, side: &str, command: &str) -> Result<Value> {
    println!("{side}$ {command}");
    let ran = agent.run_in_pane(command)?;
    println!("{}", ran.stdout.trim_end());
    ran.json()
}

/// The letter commands answer the same under `hide letter` and under the
/// retired `hide request` they were (mission-113, `hide_shell::retired`):
/// a letter sent under either spelling is shown, acknowledged, answered and
/// cancelled under the other, through the real CLI in both machines' panes,
/// since session files, edited skills and calling tools keep the old one.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries (scripts/verify-cargo.sh cli)"]
fn the_letter_commands_answer_the_same_under_hide_letter_and_hide_request() -> Result<()> {
    Fixture::journey(|fixture| {
        let hide = quote(&fixture.hide);
        let remote_hide = quote(&fixture.remote.hide);
        let mut answers = Vec::new();
        for (spelled, other) in [("letter", "request"), ("request", "letter")] {
            let sent = transcribed(
                &fixture.local,
                "local",
                &format!(
                    "{hide} {spelled} send remote-child --intent spellings-{spelled} --body {}",
                    quote(format!("SENT_AS_{spelled}"))
                ),
            )?;
            let id = sent["id"].as_str().context("sent letter ID")?.to_owned();
            ensure!(
                sent["kind"] == "request" && sent["waiting_answer"] == true,
                "{spelled} send did not send a question: {sent}"
            );
            let shown = transcribed(
                &fixture.local,
                "local",
                &format!("{hide} {other} show {id}"),
            )?;
            ensure!(
                shown["id"] == id.as_str() && shown["body"] == format!("SENT_AS_{spelled}"),
                "{other} show did not show the letter {spelled} send sent: {shown}"
            );
            let acknowledged = transcribed(
                &fixture.remote,
                "device",
                &format!("{remote_hide} {spelled} ack {id}"),
            )?;
            ensure!(
                acknowledged["id"] == id.as_str() && acknowledged["state"] == "acknowledged",
                "{spelled} ack did not acknowledge the letter: {acknowledged}"
            );
            let replied = transcribed(
                &fixture.remote,
                "device",
                &format!(
                    "{remote_hide} {other} reply {id} --intent spellings-{spelled}-answer --body {}",
                    quote(format!("ANSWERED_AS_{other}"))
                ),
            )?;
            ensure!(
                replied["reply_to"] == id.as_str(),
                "{other} reply did not answer the letter: {replied}"
            );
            let answered = transcribed(
                &fixture.local,
                "local",
                &format!("{hide} {spelled} show {id}"),
            )?;
            ensure!(
                answered["waiting_answer"] == false,
                "the answer did not end the wait: {answered}"
            );
            let second = transcribed(
                &fixture.local,
                "local",
                &format!(
                    "{hide} {spelled} send remote-child --intent spellings-{spelled}-withdrawn --body {}",
                    quote("WITHDRAWN")
                ),
            )?;
            let second = second["id"]
                .as_str()
                .context("second letter ID")?
                .to_owned();
            let cancelled = transcribed(
                &fixture.local,
                "local",
                &format!("{hide} {other} cancel {second}"),
            )?;
            ensure!(
                cancelled["state"] == "cancelled",
                "{other} cancel did not withdraw the letter: {cancelled}"
            );
            answers.push(
                [&sent, &shown, &acknowledged, &replied, &cancelled].map(|answer| {
                    answer
                        .as_object()
                        .map(|fields| fields.keys().cloned().collect::<Vec<_>>())
                }),
            );
        }
        ensure!(
            answers[0] == answers[1],
            "the two spellings answer with other fields: {answers:?}"
        );
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

/// Runs `spawn`, a `hide agent spawn --machine` of the device, in the local
/// agent's pane, and reports `session` from the pane it starts on the device
/// as a real provider's hook does as soon as it starts: the fixture provider
/// reports no native session of its own, and the spawn finishes only once
/// one is reported. Answers the command's output and the device pane, if
/// one started.
fn spawn_on_device(
    fixture: &mut Fixture,
    spawn: &str,
    session: &str,
) -> Result<(fixture::PaneOutput, Option<String>)> {
    let before = fixture::ready_panes(&fixture.remote.environment.home)?;
    let finished = AtomicBool::new(false);
    let (local, remote) = (&mut fixture.local, &mut fixture.remote);
    std::thread::scope(|scope| {
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
                remote.report_session(&remote.core_pane(pane), session)?;
            }
            Ok(pane)
        });
        let output = local.run_in_pane(spawn);
        finished.store(true, Ordering::Relaxed);
        let reported = reporter
            .join()
            .map_err(|_| anyhow::anyhow!("reporter panicked"))?;
        let output = output?;
        Ok::<_, anyhow::Error>((output, reported?))
    })
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
        let (first, spawned) = spawn_on_device(fixture, &spawn, "fixture-spawned-session")?;
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

/// A spawn's brief rides the agent's start as its launch prompt, on a
/// device and on this machine by the one decision (`docs/delivery.md`, The
/// input ladder), and reaches the agent byte for byte from a file of the
/// pane's machine: the accounts run in the locale the test runner gives,
/// `C` under `scripts/verify-cargo.sh`, where a Linux line editor turned
/// each non-ASCII byte of a typed start line into `?` (with-hide/hide#2900).
/// The stand-in starts with the Korean brief as its last argument, nothing
/// is typed for it, no prompt file is left, and the agent's own record of
/// its first prompt confirms it.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries (scripts/verify-cargo.sh cli)"]
fn a_spawns_brief_rides_the_agents_start_on_a_device_and_here() -> Result<()> {
    Fixture::journey(|fixture| {
        fixture.agent_ready_on_device("claude-code", ".claude")?;
        // Korean over many lines, a few KB, as the issue's brief was.
        let brief = (1..=40)
            .map(|line| format!("{line}번째 줄: 시작하는 에이전트의 첫 지시입니다."))
            .collect::<Vec<_>>()
            .join("\n");
        briefed_start(fixture, &brief, true)?;
        briefed_start(fixture, &brief, false)
    })
}

/// Spawns an agent with `brief` from the local agent's pane, on the device
/// or on this machine, and checks what
/// [`a_spawns_brief_rides_the_agents_start_on_a_device_and_here`] says.
fn briefed_start(fixture: &mut Fixture, brief: &str, on_device: bool) -> Result<()> {
    let (name, session) = if on_device {
        ("remote-briefed", "fixture-briefed-remote")
    } else {
        ("local-briefed", "fixture-briefed-local")
    };
    let intent = format!("spawn-{name}");
    let project = fixture.commit_project(on_device)?;
    let machine = if on_device { "--machine remote " } else { "" };
    let spawn = format!(
        "{} agent spawn --parent here {machine}--name {name} --intent {intent} --kind claude --repo {} --branch {name} --brief {}",
        quote(&fixture.hide),
        quote(&project),
        quote(brief)
    );
    let home = if on_device {
        fixture.remote.environment.home.clone()
    } else {
        fixture.local.environment.home.clone()
    };
    let before = fixture::ready_panes(&home)?;
    // The spawn waits for its agent's session, which the stand-in reports
    // only when asked: the new pane reports it from its own account while
    // the caller's pane still runs the spawn.
    let agent = if on_device {
        &fixture.remote
    } else {
        &fixture.local
    };
    let finished = AtomicBool::new(false);
    let (answer, spawned) = std::thread::scope(|scope| {
        let reporter = scope.spawn(|| -> Result<Option<String>> {
            let pane = wait_for("the spawned agent's pane", || {
                if finished.load(Ordering::Relaxed) {
                    return Ok(Some(None));
                }
                Ok(fixture::ready_panes(&home)?
                    .into_iter()
                    .find(|pane| !before.contains(pane))
                    .map(Some))
            })?;
            if let Some(pane) = &pane {
                agent.report_session(&agent.core_pane(pane), session)?;
            }
            Ok(pane)
        });
        let answer = fixture.local.run_in_pane(&spawn);
        finished.store(true, Ordering::Relaxed);
        let reported = reporter
            .join()
            .map_err(|_| anyhow::anyhow!("reporter panicked"))?;
        Ok::<_, anyhow::Error>((answer?, reported?))
    })?;
    ensure!(
        answer.status == 0,
        "spawn refused: {} {}",
        answer.stdout,
        answer.stderr
    );
    let spawned = spawned.context("the spawned agent's pane")?;
    let screen_pane = agent.core_pane(&spawned);
    // The start carried the brief, its line breaks as U+2028, after `--`,
    // byte for byte from the file its machine kept, and the file is gone.
    let argv = fixture::started_with(&home, &spawned)?.context("the stand-in's arguments")?;
    let carried = brief.replace('\n', "\u{2028}");
    ensure!(
        argv.ends_with(&["--".to_owned(), carried]),
        "the start did not carry the brief whole: {argv:?}"
    );
    let account = if on_device {
        &fixture.remote.environment
    } else {
        &fixture.local.environment
    };
    let left = fixture::prompt_files(account)?;
    ensure!(left.is_empty(), "the start left its prompt file: {left:?}");
    let record = |ledger: &hide_core::delivery::ledger::Ledger| -> Result<Value> {
        Ok(serde_json::to_value(
            ledger
                .spawns
                .iter()
                .find(|record| record.intent == intent)
                .context("spawn receipt")?,
        )?)
    };
    let saved = record(&fixture.ledger()?)?;
    ensure!(
        saved["brief_input"]["route"] == "launch" && saved["brief_input"]["attempts"] == 1,
        "the brief was not sent as the start's launch prompt: {}",
        saved["brief_input"]
    );
    // The agent's first turn records the brief as Claude Code does, with its
    // line breaks, which confirms it.
    let agent = if on_device {
        &fixture.remote
    } else {
        &fixture.local
    };
    agent.run_in(&screen_pane, &hook(&agent.hooks, session, brief))?;
    wait_for("the launched brief confirmed by its record", || {
        let saved = record(&fixture.ledger()?)?;
        Ok(saved["brief_input"]["confirmed_at_unix_ms"]
            .is_u64()
            .then_some(()))
    })?;
    Ok(())
}

/// How many lines of the daemon's diagnostic log at `state` are of `kind`.
fn logged(state: &std::path::Path, kind: &str) -> Result<usize> {
    let path = state.join("Logs/core.jsonl");
    if !path.exists() {
        return Ok(0);
    }
    Ok(String::from_utf8(fixture::read(&path)?)?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["kind"] == kind)
        .count())
}

/// The last line of the daemon's diagnostic log at `state` that is of
/// `kind`, for a failure to name.
fn last_logged(state: &std::path::Path, kind: &str) -> String {
    let text = fixture::read(&state.join("Logs/core.jsonl")).unwrap_or_default();
    String::from_utf8_lossy(&text)
        .lines()
        .rfind(|line| line.contains(&format!("\"kind\":\"{kind}\"")))
        .unwrap_or("none")
        .to_owned()
}

/// with-hide/hide#2882: a device link died without its close reaching the
/// device (the Mac roamed off its Wi-Fi, and the server kept the connection
/// for fifteen minutes), Hide connected again, and the new link's node
/// waited out its two minutes for the pane ownership the old link's node
/// still held: the device's panes never came back and every spawn there
/// failed. Every node now ends a link whose core sent nothing for
/// `HEARTBEAT_LIMIT`, so the old one lets go well inside that wait. The
/// fixture's relay makes the connection half-open, as the network did: no
/// byte crosses and no close reaches the server.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries (scripts/verify-cargo.sh cli)"]
fn a_device_link_cut_without_a_close_gives_its_panes_to_the_next_link() -> Result<()> {
    use hide_node_link::liveness::HEARTBEAT_LIMIT;
    /// How long the next link's node waits for the pane ownership before
    /// its restore fails (`hide_core::engine::own::restore`, `RECOVERY_WAIT`).
    const RECOVERY_WAIT: Duration = Duration::from_secs(120);
    Fixture::journey(|fixture| {
        // Sleep off: the stand-ins' conversations are dated long ago, so the
        // idle rule ends the local caller while the cut link waits out its
        // limit, and its next command never runs; the journey's subject is
        // the link, not sleep.
        fixture.event("agent_sleep_set", serde_json::json!({"after_hours": null}))?;
        let project = fixture.commit_remote_project()?;
        fixture.agent_ready_on_device("claude-code", ".claude")?;
        let state = fixture.state.clone();
        let closed = logged(&state, "host.closed")?;
        let ready = logged(&state, "host.ready")?;
        let restored = logged(&state, "engine.restored")?;
        let unfinished = logged(&state, "engine.restore_unfinished")?;

        fixture.network.cut();
        // The core hears nothing from its node either, and ends its link.
        wait_within(
            "the core to end the cut link",
            HEARTBEAT_LIMIT + Duration::from_secs(30),
            || Ok((logged(&state, "host.closed")? > closed).then_some(())),
        )?;
        fixture.reconnect_device()?;
        wait_within(
            "the device's panes restored on the next link",
            RECOVERY_WAIT + Duration::from_secs(60),
            || {
                ensure!(
                    logged(&state, "engine.restore_unfinished")? == unfinished,
                    "the next link's restore did not finish: {}",
                    last_logged(&state, "engine.restore_unfinished")
                );
                Ok((logged(&state, "engine.restored")? > restored).then_some(()))
            },
        )?;
        ensure!(
            logged(&state, "host.ready")? > ready,
            "the panes were restored with no new link"
        );
        let restore: Value = serde_json::from_str(&last_logged(&state, "engine.restored"))?;
        ensure!(
            restore["adopted"]
                .as_u64()
                .is_some_and(|adopted| adopted >= 1),
            "the next link did not take the device's running pane back: {restore}"
        );

        // The device's pane answers through the core again.
        let read = fixture.local.run_in_pane(&format!(
            "{} pane read {}",
            quote(&fixture.hide),
            quote(&fixture.remote.pane)
        ))?;
        ensure!(
            read.status == 0,
            "the device's pane did not answer after the restore: {} {}",
            read.stdout,
            read.stderr
        );

        // A spawn on the device starts its agent, and the same command
        // again returns that agent.
        let spawn = format!(
            "{} agent spawn --parent here --machine remote --name after-cut --intent spawn-after-cut --kind claude --repo {} --branch after-cut",
            quote(&fixture.hide),
            quote(&project)
        );
        let (first, spawned) = spawn_on_device(fixture, &spawn, "fixture-after-cut-session")?;
        ensure!(
            first.status == 0,
            "the spawn after the cut was refused: {} {}",
            first.stdout,
            first.stderr
        );
        spawned.context("the spawned agent's pane on the device")?;
        let child = |ledger: &hide_core::delivery::ledger::Ledger| {
            let records: Vec<_> = ledger
                .spawns
                .iter()
                .filter(|record| record.intent == "spawn-after-cut")
                .collect();
            match records.as_slice() {
                [record] => record.child.clone(),
                _ => None,
            }
        };
        let started = child(&fixture.ledger()?).context("the spawn's one finished record")?;
        let again = fixture.local.run_in_pane(&spawn)?;
        ensure!(
            again.status == 0 && child(&fixture.ledger()?) == Some(started),
            "the same spawn again did not return the same agent: {} {}",
            again.stdout,
            again.stderr
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

/// How long the journey gives one switch's answer, and one look of the
/// daemon's own: each is an SSH login that runs the device's `hide` once,
/// and macOS scans a new copy of a program at its first start (under load
/// the first start of a 400 MB debug binary took 5 to 6 s, #2846). The
/// CLI's own bound is minutes (`device_cli::SWITCH_BOUND`).
const SWITCH_WITHIN: Duration = Duration::from_secs(60);

/// `hide device switch` for the fixture's device, named by its id or the
/// alias it was added by, at a terminal of the core's machine: whether it
/// succeeded, and the line it answered.
fn switch_device(fixture: &Fixture, device: &str, words: &[&str]) -> Result<(bool, Value)> {
    let mut command = fixture.local.environment.command(&fixture.hide);
    command
        .args(["device", "switch", device, "--json"])
        .args(words);
    let ran = fixture::capture_within(command, SWITCH_WITHIN)?;
    let text = String::from_utf8_lossy(&ran.stdout).into_owned();
    let answer =
        serde_json::from_str(text.lines().last().unwrap_or_default()).with_context(|| {
            format!(
                "the switch answered no JSON line: {text} {}",
                String::from_utf8_lossy(&ran.stderr)
            )
        })?;
    Ok((ran.status.success(), answer))
}

/// The fixture's device as the core's snapshot lists it.
fn device_row(snapshot: &Value) -> Result<Value> {
    snapshot
        .pointer("/navigator/devices")
        .and_then(Value::as_array)
        .and_then(|rows| rows.iter().find(|row| row["id"] == fixture::DEVICE))
        .cloned()
        .context("the device left the core's list")
}

/// The operator's decision of 2026-11-24: a device is switched to its
/// machine's own Hide only when the operator asks, and only when that Hide
/// would be back after a restart with nobody there. The fixture's account
/// has no login item of the system's (its HOME is not the account's own),
/// so `hide device switch` answers that the device is left: it pairs
/// nothing and starts nothing there, the device's terminal runs on, and
/// the device is one row across another start of the core. A device that
/// is off is not reached by the switch, which answers a refusal. Nothing
/// switches a device nobody asked about: at each point the journey reads
/// it, after a start and after a connection, it is still the helper's.
/// A switch that runs is held against stand-ins: `pairing_wsl`'s
/// `a_distribution_is_switched_through_wsl` and
/// `a_distribution_is_read_without_change_and_asked_to_linger_only_by_a_switch`, and
/// `hide-core`'s `device_switch_folder` tests.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries (scripts/verify-cargo.sh cli)"]
fn a_switch_leaves_on_its_helper_a_device_whose_hide_would_not_be_back_alone() -> Result<()> {
    Fixture::journey(|fixture| {
        let stand_in = |fixture: &mut Fixture| -> Result<String> {
            let ran = fixture.remote.run_in_pane("echo $PPID")?;
            ensure!(ran.status == 0, "the pane's command failed: {}", ran.stderr);
            Ok(ran.stdout.trim().to_owned())
        };
        let before = stand_in(fixture)?;
        let account = fixture.remote.environment.clone();
        let state = std::path::PathBuf::from(
            account
                .var("HIDE_STATE_DIR")
                .context("the device account's state folder")?,
        );
        // What the daemon itself said it did, in its diagnostic log.
        let said = |fixture: &Fixture, kind: &str| -> Result<Vec<Value>> {
            let log = fixture.state.join("Logs/core.jsonl");
            Ok(String::from_utf8(fixture::read(&log)?)?
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|row| row["kind"] == kind)
                .collect())
        };
        // The device is the helper's, and no Hide was started on it.
        let left = |fixture: &Fixture| -> Result<()> {
            let row = device_row(&fixture.snapshot()?)?;
            ensure!(row["peer_key"].is_null(), "the device was switched: {row}");
            ensure!(
                !state.join("hided.json").exists(),
                "a Hide was started on a device that was left"
            );
            Ok(())
        };
        // The device's link is up, as each switch below needs it and as
        // its terminal's carrying on is read over it.
        let connected = |fixture: &Fixture| -> Result<()> {
            fixture::wait_within(
                "the device to connect",
                SWITCH_WITHIN + fixture::READY_BOUND,
                || {
                    let snapshot = fixture.snapshot()?;
                    Ok(snapshot
                        .pointer("/status/remote")
                        .and_then(Value::as_array)
                        .is_some_and(|rows| {
                            rows.iter().any(|row| {
                                row["target_id"] == fixture::DEVICE && row["state"] == "connected"
                            })
                        })
                        .then_some(()))
                },
            )
        };
        // Connected, with nothing asked: as it was added.
        left(fixture)?;

        // The device is off when the core next starts: a switch asked for
        // at a terminal does not reach it, answers a refusal and changes
        // nothing.
        let retries = said(fixture, "host.retry_scheduled")?.len();
        fixture.ssh.online(false)?;
        fixture.restart_core(|_| Ok(()))?;
        wait_for("a connection attempt that did not reach the device", || {
            Ok((said(fixture, "host.retry_scheduled")?.len() > retries).then_some(()))
        })?;
        let (ok, answer) = switch_device(fixture, fixture::DEVICE, &["--install"])?;
        ensure!(
            !ok && answer["value"]["switch"].is_null(),
            "a switch to a device that is off: {answer}"
        );
        left(fixture)?;
        // It is back, and connected again with nothing asked.
        fixture.ssh.online(true)?;
        fixture.reconnect_device()?;
        connected(fixture)?;
        left(fixture)?;

        // One row for the machine: one device the core dials over SSH and
        // no paired machine.
        let one_row = |fixture: &Fixture| -> Result<()> {
            let snapshot = fixture.snapshot()?;
            let remote: Vec<&Value> = snapshot
                .pointer("/navigator/devices")
                .and_then(Value::as_array)
                .map(|rows| rows.iter().filter(|row| row["kind"] == "remote").collect())
                .unwrap_or_default();
            ensure!(
                matches!(remote.as_slice(), [row] if row["id"] == fixture::DEVICE
                    && row["label"] == "Remote fixture" && row["peer_key"].is_null()),
                "the devices: {remote:?}"
            );
            let peers = snapshot
                .pointer("/navigator/peers")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            ensure!(
                peers.is_empty(),
                "a device that was left was paired: {peers:?}"
            );
            Ok(())
        };

        // The switch reads the machine once, with or without the install
        // allowed and named by the alias a terminal knows the device by:
        // its Hide would not be back alone, so the device is left.
        for words in [&[][..], &["--install"][..]] {
            let (ok, answer) = switch_device(fixture, "delivery-fixture", words)?;
            ensure!(
                ok && answer["value"]["switch"] == "left"
                    && answer["value"]["device_id"] == fixture::DEVICE,
                "the switch of a device whose Hide would not be back alone: {answer}"
            );
            left(fixture)?;
        }
        one_row(fixture)?;
        ensure!(
            stand_in(fixture)? == before,
            "the terminal of a device that was left did not carry on"
        );

        // The next start finds the same one device, still the helper's.
        fixture.restart_core(|_| Ok(()))?;
        connected(fixture)?;
        left(fixture)?;
        one_row(fixture)?;
        ensure!(
            stand_in(fixture)? == before,
            "the terminal did not carry on across the core's next start"
        );
        Ok(())
    })
}
