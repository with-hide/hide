//! Moving the core to another machine (PRD core-host-node-move): candidate
//! hided on both machines, the pinned Herdr on each, and a real SSH
//! connection that only the source opens.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::support::core_move::{ALIAS, DEVICE_ID, Fixture, SOURCE_NODE, TARGET_NODE};
use crate::support::fake_tailscale::FakeTailscale;
use crate::support::remote_delivery::renderer::Renderer;
use crate::support::remote_delivery::{wait_for, wait_within};
use crate::support::screen::{first_snapshot, screen_socket, send, type_and_read};

/// B4: the move stops the source's core, places its brain state on the
/// target and starts the core there, and the source's window, at the same
/// address, becomes a node of that core and shows the same projects of the
/// same machines.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn the_core_moves_to_the_device_and_the_window_follows_it() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = fixture.projects()?;
        ensure!(
            before
                == vec![
                    (
                        SOURCE_NODE.to_owned(),
                        fixture.source.project().display().to_string()
                    ),
                    (
                        DEVICE_ID.to_owned(),
                        fixture.target.project().display().to_string()
                    ),
                ],
            "the projects before the move: {before:?}"
        );
        let window = fixture.window();
        let instance = fixture.health()?["instance"].as_u64().context("instance")?;
        // A window open on the core when the move starts.
        let mut open = Renderer::connect(window.0, &window.1)?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;

        // Its socket is closed as the core stops, and what answers at its
        // address by then is the move's screen (or the node after it), so
        // the window's reconnect hears the move rather than wait on a core
        // that is gone (W1).
        let (code, reason) = open.closed_within(std::time::Duration::from_secs(10))?;
        ensure!(
            (code, reason.as_str()) == (1012, "role_ended"),
            "the open window was closed as {code} {reason}"
        );
        let answering = fixture.health()?;
        ensure!(
            matches!(answering["role"].as_str(), Some("moving" | "node")),
            "the window's address answered {answering} once its socket closed"
        );
        let journal = fixture.journal_until("done")?;
        let intent = journal["intent"].as_str().context("intent")?.to_owned();

        // The window kept its address, and a node role answers there now.
        let state = fixture.source.daemon()?.context("source state")?;
        ensure!(
            (state["port"].as_u64(), state["token"].as_str())
                == (Some(u64::from(window.0)), Some(window.1.as_str())),
            "the window's address changed: {state}"
        );
        ensure!(
            fixture.health()?["instance"].as_u64() > Some(instance),
            "no new role is mounted"
        );
        let placement = fixture
            .source
            .record("core-placement.json")?
            .context("placement")?;
        ensure!(placement["node"] == TARGET_NODE, "{placement}");
        ensure!(placement.get("move_intent").is_none(), "{placement}");

        // The source's brain state is set aside, the target's core runs on
        // the copy and the move left no record there.
        let moved_out = fixture.source.state.join("moved-out").join(&intent);
        ensure!(
            moved_out.join("core-state.json").is_file(),
            "nothing set aside"
        );
        ensure!(!fixture.source.state.join("core-state.json").exists());
        ensure!(fixture.target_core()?.is_some(), "no core on the target");
        ensure!(fixture.target.record("core-handover.json")?.is_none());

        // The same projects of the same machines, each now named as the new
        // core names them.
        let after = wait_for("the projects through the new core", || {
            let projects = fixture.projects()?;
            Ok((projects.len() == 2).then_some(projects))
        })?;
        ensure!(
            after
                == vec![
                    (
                        SOURCE_NODE.to_owned(),
                        fixture.source.project().display().to_string()
                    ),
                    (
                        TARGET_NODE.to_owned(),
                        fixture.target.project().display().to_string()
                    ),
                ],
            "the projects after the move: {after:?}"
        );
        let registrations = fixture.snapshot()?;
        let devices: Vec<&Value> = registrations
            .pointer("/ui_state/device_registrations")
            .and_then(Value::as_array)
            .context("device registrations")?
            .iter()
            .collect();
        ensure!(
            devices.len() == 1 && devices[0]["id"] == SOURCE_NODE && devices[0]["inbound"] == true,
            "{devices:?}"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// A target whose state folder is reached through a link takes the core: the
/// target answers the folder by its resolved path, which the copy can reach,
/// where it once stopped the move at the check for a link it could not
/// follow.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_target_whose_state_folder_is_reached_through_a_link_takes_the_core() -> Result<()> {
    let fixture = Fixture::start_through_link()?;
    let journey = (|| {
        ensure!(
            fixture.target.state.starts_with(fixture.root.join("via")),
            "the target's state folder is not spelled through the link"
        );
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let journal = fixture.journal_until("done")?;
        let canonical = hide_platform::fs::identity::canonical(&fixture.target.state)?;
        ensure!(
            journal["peer"]["state_dir"]
                .as_str()
                .is_some_and(|sent| sent.trim_end_matches('/') == canonical.display().to_string()),
            "the copy was sent by {}, not by {}",
            journal["peer"]["state_dir"],
            canonical.display()
        );
        ensure!(fixture.target_core()?.is_some(), "no core on the target");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The choices made for a device's project (a guardrail, a repository, a
/// branch) are kept at a key made from the machine's id, which a move gives
/// another one: they follow the project through the move and the move back,
/// and neither stops on them nor loses them.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_device_project_s_settings_follow_it_through_the_move_and_the_move_back() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let theirs = fixture.target.project().display().to_string();
        let own = fixture.source.project().display().to_string();
        for (device, root, request) in [(DEVICE_ID, &theirs, "one"), (SOURCE_NODE, &own, "two")] {
            fixture.event(
                "project_settings_set",
                json!({"device_id": device, "root": root, "request_id": request, "patch": {"new_work": "worktree"}}),
            )?;
        }
        let kept = |device: &str, root: &str| -> Result<bool> {
            Ok(settings_at(&fixture, root)?.is_some_and(|(key, record)| {
                key == hide_project::project_id(device, std::path::Path::new(root))
                    && record["device_id"] == device
                    && record["new_work"] == "worktree"
            }))
        };
        wait_for("both choices kept under their machines", || {
            Ok((kept(DEVICE_ID, &theirs)? && kept(SOURCE_NODE, &own)?).then_some(()))
        })?;

        moved_forward(&fixture)?;
        // The device is the core's own machine now, the source a node that
        // links in by its node id.
        wait_for("both choices kept under their new machines", || {
            Ok((kept(TARGET_NODE, &theirs)? && kept(SOURCE_NODE, &own)?).then_some(()))
        })?;

        fixture.event("core_move", json!({"action": "back"}))?;
        back_journal_until(&fixture, "done")?;
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        wait_for("both choices kept under their first machines", || {
            Ok((kept(DEVICE_ID, &theirs)? && kept(SOURCE_NODE, &own)?).then_some(()))
        })?;
        Ok(())
    })();
    finish(fixture, journey)
}

/// The record of the project at `root` in the window's snapshot, with the key
/// it is kept at.
fn settings_at(fixture: &Fixture, root: &str) -> Result<Option<(String, Value)>> {
    let snapshot = fixture.snapshot()?;
    Ok(snapshot
        .pointer("/ui_state/project_settings")
        .and_then(Value::as_object)
        .and_then(|records| {
            records
                .iter()
                .find(|(_, record)| record["root"] == root)
                .map(|(key, record)| (key.clone(), record.clone()))
        }))
}

/// A window whose app is older than the core it moved to is told so with
/// both builds and the core's machine, and the core goes on as it was
/// (B11).
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_window_older_than_its_core_asks_for_its_app_to_be_updated() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        fixture.journal_until("done")?;
        let core = fixture.target_core()?.context("no core on the target")?;
        let core_release = hided::build_order::Release::of_this_build().shown();
        // The window's app is another build that presents an older release.
        let older = fixture.other_build("older")?;
        fixture.kill_source()?;
        fixture.start_source_with(
            &older,
            &[(hided::env::HIDE_BUILD_VERSION_OVERRIDE, "0.0.1")],
        )?;
        let answer = fixture.connect_with(&older)?;
        ensure!(
            answer["ok"] == false
                && answer["reason"] == "core_newer"
                && answer["app"] == "0.0.1"
                && answer["core"] == core_release.as_str()
                && answer["machine"]
                    .as_str()
                    .is_some_and(|machine| !machine.is_empty()),
            "hide connect answered {answer}"
        );
        ensure!(
            fixture.target_core()? == Some(core),
            "the core did not go on as it was"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// An app newer than the core its machine runs under the core's starter
/// updates that core through the same updater a node runs and attaches to
/// it, and never stops it to start its own (Q13, B10).
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_newer_app_on_the_core_machine_updates_its_core() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        fixture.journal_until("done")?;
        let before = fixture.target_core()?.context("no core on the target")?;
        let previous = fixture.target.health()?["starter"]["program"]
            .as_str()
            .context("the moved core names no starter")?
            .to_owned();
        let newer = fixture.other_build("newer")?;
        let newer_build =
            hided::build_id::of_file(&newer.join("hided")).map_err(anyhow::Error::msg)?;
        let app = fixture.bundle("newer", &newer)?;
        let answer = fixture.connect_on_target(
            &app,
            &[(hided::env::HIDE_BUILD_VERSION_OVERRIDE, "999.0.0")],
        )?;
        ensure!(answer["ok"] == true, "hide connect answered {answer}");
        let after = fixture.target_core()?.context("no core after the update")?;
        let health = fixture.target.health()?;
        ensure!(
            after != before && answer["pid"] == after && health["build"] == newer_build.as_str(),
            "the app attached to {answer} while the core runs {health}"
        );
        let program = health["starter"]["program"]
            .as_str()
            .context("the updated core names no starter")?;
        let (builds, current) = builds_beside(program)?;
        let mut expected = vec![version_of(&previous)?, version_of(program)?];
        expected.sort();
        ensure!(
            builds == expected && current == version_of(program)?,
            "the core machine keeps {builds:?} with current at {current}"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// An app older than the core its machine runs under the core's starter is
/// told to update with both builds, and the core goes on as it was (Q13,
/// B11).
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn an_older_app_on_the_core_machine_leaves_its_core_running() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        fixture.journal_until("done")?;
        let core = fixture.target_core()?.context("no core on the target")?;
        let older = fixture.other_build("older")?;
        let app = fixture.bundle("older", &older)?;
        let answer = fixture
            .connect_on_target(&app, &[(hided::env::HIDE_BUILD_VERSION_OVERRIDE, "0.0.1")])?;
        ensure!(
            answer["ok"] == false
                && answer["reason"] == "core_newer"
                && answer["app"] == "0.0.1"
                && answer["core"]
                    == hided::build_order::Release::of_this_build()
                        .shown()
                        .as_str(),
            "hide connect answered {answer}"
        );
        ensure!(
            fixture.target_core()? == Some(core),
            "the core did not go on as it was"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// A letter that had waited 59 minutes for an agent on the window's machine
/// when the core moved keeps that wait on the core's new machine, where it
/// counts only while the window's machine is reachable: it outlives its hour
/// while that machine is away and turns undelivered soon after it is back
/// (amendment 4, B13).
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_letter_held_across_the_move_keeps_its_wait() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        let unix_ms = || -> Result<u64> {
            Ok(std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis() as u64)
        };
        let sent = unix_ms()? - 59 * 60_000;
        fixture.kill_source()?;
        let path = fixture.source.state.join("delivery-ledger.json");
        let mut ledger: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
        let actor = |pane: &str| json!({"pane_id": pane, "name": "lead", "kind": "claude", "device_id": SOURCE_NODE, "session": "lead-session"});
        ledger["next_id"] = json!(100);
        ledger["letters"] = json!([{"id": "letter-1", "intent": "held-across-move", "sender": actor("w9:p8"), "recipient": actor("w9:p9"), "kind": "report", "body": "A letter held across the move", "state": "pending", "hook_confirmed": false, "waiting_answer": false, "reply_to": null, "created_at_unix_ms": sent, "finished_at_unix_ms": null, "bell_errors": 0, "bell_sent": false, "bell_attempts": 0, "human_notified": false}]);
        write_record(&path, &ledger)?;
        fixture.start_source()?;
        fixture.device_ready()?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        fixture.journal_until("done")?;
        let target_ledger = fixture.target.state.join("delivery-ledger.json");
        let held = || -> Result<(Value, Value)> {
            let ledger: Value = serde_json::from_slice(&std::fs::read(&target_ledger)?)?;
            let letter = ledger["letters"]
                .as_array()
                .and_then(|letters| letters.iter().find(|letter| letter["id"] == "letter-1"))
                .cloned()
                .context("the held letter")?;
            Ok((letter, ledger["node_clocks"][SOURCE_NODE].clone()))
        };
        // The window's machine goes away.
        fixture.kill_source()?;
        wait_for("the window's machine read as away", || {
            let (_, clock) = held()?;
            Ok((clock.is_object() && clock["since"].is_null()).then_some(()))
        })?;
        let hour_passed = sent + 60 * 60_000 + 5_000;
        wait_within(
            "an hour since the letter was sent",
            std::time::Duration::from_secs(120),
            || Ok((unix_ms()? >= hour_passed).then_some(())),
        )?;
        let (letter, clock) = held()?;
        ensure!(
            letter["state"] == "pending",
            "the letter did not wait while its machine was away: {letter} {clock}"
        );
        fixture.start_source()?;
        wait_within(
            "the held letter's deadline once its machine is back",
            std::time::Duration::from_secs(120),
            || {
                let (letter, _) = held()?;
                Ok((letter["state"] == "undelivered").then_some(()))
            },
        )?;
        Ok(())
    })();
    finish(fixture, journey)
}

/// The program the source's placement names on the target.
fn placed_program(fixture: &Fixture) -> Result<String> {
    Ok(fixture
        .source
        .record("core-placement.json")?
        .context("placement")?["program"]
        .as_str()
        .context("placement program")?
        .to_owned())
}

/// The build folders under the install root of `program` on the target,
/// and where its `current` leads.
fn builds_beside(program: &str) -> Result<(Vec<String>, String)> {
    let root = std::path::Path::new(program)
        .parent()
        .and_then(std::path::Path::parent)
        .context("build root")?;
    let mut builds: Vec<String> = std::fs::read_dir(root)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_>>()?;
    builds.retain(|name| name != "current");
    builds.sort();
    let current = std::fs::read_link(root.join("current"))?
        .to_string_lossy()
        .into_owned();
    Ok((builds, current))
}

fn version_of(program: &str) -> Result<String> {
    Ok(std::path::Path::new(program)
        .parent()
        .and_then(std::path::Path::file_name)
        .context("version folder")?
        .to_string_lossy()
        .into_owned())
}

/// A window whose app is newer than the core it moved to updates the core
/// to its own build through the core machine's starter and links to it,
/// and the core machine keeps that build and the one before (B10).
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_window_newer_than_its_core_updates_it_and_links() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        fixture.journal_until("done")?;
        let before = fixture.target_core()?.context("no core on the target")?;
        let previous = placed_program(&fixture)?;
        let newer = fixture.other_build("newer")?;
        let newer_build =
            hided::build_id::of_file(&newer.join("hided")).map_err(anyhow::Error::msg)?;
        fixture.kill_source()?;
        fixture.start_source_with(
            &newer,
            &[(hided::env::HIDE_BUILD_VERSION_OVERRIDE, "999.0.0")],
        )?;
        let done = fixture.logged(&fixture.source, "update.done")?;
        wait_for("the window's link to the updated core", || {
            Ok((fixture.health()?["core_link"] == "live").then_some(()))
        })?;
        let after = fixture.target_core()?.context("no core after the update")?;
        ensure!(after != before, "the core was not replaced");
        let health = fixture.target.health()?;
        ensure!(
            health["build"] == newer_build.as_str(),
            "the core runs {health}"
        );
        let program = placed_program(&fixture)?;
        ensure!(
            done["program"] == program.as_str() && program != previous,
            "the placement names {program} after {done}"
        );
        let (builds, current) = builds_beside(&program)?;
        let mut expected = vec![version_of(&previous)?, version_of(&program)?];
        expected.sort();
        ensure!(
            builds == expected && current == version_of(&program)?,
            "the core machine keeps {builds:?} with current at {current}"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// An update to a build whose core does not start goes back to the build
/// the core ran, which takes links again; the window does not try again on
/// this connection, its page names both builds, and its Retry is the next
/// connection's one attempt (B10, B20).
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_core_update_that_does_not_start_goes_back_to_the_previous_build() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        fixture.journal_until("done")?;
        let previous = placed_program(&fixture)?;
        let previous_build = fixture.target.health()?["build"].clone();
        let newer = fixture.other_build("newer")?;
        let newer_build =
            hided::build_id::of_file(&newer.join("hided")).map_err(anyhow::Error::msg)?;
        std::fs::write(
            fixture
                .target
                .home()
                .join(hided::env::FIXTURE_FAIL_BUILD_FILE),
            &newer_build,
        )?;
        fixture.kill_source()?;
        fixture.start_source_with(
            &newer,
            &[(hided::env::HIDE_BUILD_VERSION_OVERRIDE, "999.0.0")],
        )?;
        let failed = fixture.logged(&fixture.source, "update.failed")?;
        ensure!(
            failed["reason"]
                .as_str()
                .is_some_and(|reason| reason.starts_with("rolled_back")),
            "{failed}"
        );
        let health = fixture.target.health()?;
        ensure!(health["build"] == previous_build, "the core runs {health}");
        ensure!(placed_program(&fixture)? == previous, "the placement moved");
        let (builds, current) = builds_beside(&previous)?;
        ensure!(
            builds.len() == 2 && current == version_of(&previous)?,
            "the core machine keeps {builds:?} with current at {current}"
        );
        // Dials go on after the failure, and none updates again.
        wait_for("a dial after the failed update", || {
            let ended = records(&fixture, "link.ended")?;
            let failed = ended
                .iter()
                .filter(|record| record["reason"] == "update_failed");
            Ok((failed.count() >= 2).then_some(()))
        })?;
        ensure!(
            records(&fixture, "update.started")?.len() == 1,
            "the update ran again"
        );
        // What the window's update-failed page names: the core's machine,
        // the build it runs again and this app's.
        let health = fixture.health()?;
        let builds = &health["builds"];
        ensure!(
            health["core_link_reason"] == "update_failed"
                && builds["machine"]
                    .as_str()
                    .is_some_and(|machine| !machine.is_empty())
                && builds["core"]
                    == hided::build_order::Release::of_this_build()
                        .shown()
                        .as_str()
                && builds["app"] == "999.0.0",
            "the node's health: {health}"
        );
        // Retry is the next connection's one attempt, which succeeds once
        // the build can start.
        std::fs::remove_file(
            fixture
                .target
                .home()
                .join(hided::env::FIXTURE_FAIL_BUILD_FILE),
        )?;
        let answer = fixture.connect_with(&newer)?;
        ensure!(answer["ok"] == true, "hide connect answered {answer}");
        fixture.logged(&fixture.source, "update.done")?;
        wait_for("the window's link to the updated core", || {
            Ok((fixture.health()?["core_link"] == "live").then_some(()))
        })?;
        ensure!(records(&fixture, "update.started")?.len() == 2);
        ensure!(fixture.target.health()?["build"] == newer_build.as_str());
        Ok(())
    })();
    finish(fixture, journey)
}

/// An update is refused while a move holds the core's machine, and the
/// core goes on as it was (B10, D-20).
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_core_update_waits_for_a_move_on_the_core_machine() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        fixture.journal_until("done")?;
        let core = fixture.target_core()?.context("no core on the target")?;
        write_record(
            &fixture.target.state.join("core-handover.json"),
            &json!({"version": 1, "intent": "move-0123456789abcdef", "source": SOURCE_NODE, "target": TARGET_NODE, "state": {"state": "pending", "lease_until_unix_ms": lease_ahead()}}),
        )?;
        let newer = fixture.other_build("newer")?;
        fixture.kill_source()?;
        fixture.start_source_with(
            &newer,
            &[(hided::env::HIDE_BUILD_VERSION_OVERRIDE, "999.0.0")],
        )?;
        let failed = fixture.logged(&fixture.source, "update.failed")?;
        ensure!(failed["reason"] == "move_running", "{failed}");
        ensure!(fixture.target_core()? == Some(core), "the core was touched");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The source's window state the journeys compare: its projects and the
/// devices it registers.
fn visible(fixture: &Fixture) -> Result<Value> {
    let snapshot = fixture.snapshot()?;
    Ok(json!({
        "projects": fixture.projects()?,
        "devices": snapshot.pointer("/ui_state/device_registrations").cloned(),
    }))
}

/// A move whose check fails changes nothing on either machine: the source
/// keeps its core and its window, and the target keeps its folder as it was.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_failing_check_changes_nothing() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let planted = fixture.target.state.join("core-state.json");
        std::fs::write(&planted, b"{\"schema_version\":0}")?;
        // Settings of the target's own, never merged with the core's.
        let theirs = fixture.target.ai_settings();
        std::fs::create_dir_all(theirs.parent().context("settings folder")?)?;
        std::fs::write(&theirs, r#"{"provider":"codex"}"#)?;
        let target_before = listing(&fixture.target.state)?;
        let before = visible(&fixture)?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let failed = fixture.logged(&fixture.source, "checks.failed")?;
        ensure!(failed["checks"] == json!(["target_state"]), "{failed}");
        // Every check ran, so the window counts the twelve that passed.
        ensure!(failed["checked"] == 13, "{failed}");
        let detail = failed["failed"][0]["detail"].as_str().unwrap_or_default();
        ensure!(
            detail.contains("core-state.json") && detail.contains("Hide AI settings"),
            "{failed}"
        );
        ensure!(fixture.role()? == "core");
        ensure!(visible(&fixture)? == before, "the window changed");
        for name in ["core-move.json", "core-placement.json", "move-staging"] {
            ensure!(
                !fixture.source.state.join(name).exists(),
                "the source has {name}"
            );
        }
        ensure!(
            listing(&fixture.target.state)? == target_before,
            "the target changed"
        );
        ensure!(std::fs::read(&planted)? == b"{\"schema_version\":0}");
        ensure!(std::fs::read_to_string(&theirs)? == r#"{"provider":"codex"}"#);
        Ok(())
    })();
    finish(fixture, journey)
}

/// B3: each step missing on the target is named with what its check found,
/// `gh` signed out, no desktop session, sleep on power and Hide AI's agent
/// signed out, and the move starts nothing on either machine.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn each_step_missing_on_the_target_is_named_and_nothing_changes() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        fixture.fail_checks()?;
        let target_before = listing(&fixture.target.state)?;
        let before = visible(&fixture)?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let failed = fixture.logged(&fixture.source, "checks.failed")?;
        ensure!(
            failed["checks"] == json!(["gui_session", "sleep", "gh", "ai"]),
            "{failed}"
        );
        // With no desktop session its logins may not have run: no count.
        ensure!(failed["checked"].is_null(), "{failed}");
        ensure!(fixture.role()? == "core");
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(!fixture.source.state.join("core-move.json").exists());
        ensure!(
            listing(&fixture.target.state)? == target_before,
            "the target changed"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// The target's core cannot start after the source's core stopped: the
/// source waits while a core may still be starting there, then starts its
/// own core again unchanged; the retry keeps the move's intent and sends
/// nothing the target already holds.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_target_core_that_cannot_start_is_rolled_back_and_the_retry_reuses_the_copy() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        // Another process holds the target folder's instance lock, so the
        // core the move starts there exits at once.
        let lock = hided::state_file::acquire_lock(&fixture.target.state)?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let failed = fixture.logged(&fixture.source, "abort.failed")?;
        let intent = failed["intent"].as_str().context("intent")?.to_owned();
        // Never two cores: while the target's folder may still start one,
        // the source runs none.
        ensure!(fixture.role()? == "moving");
        ensure!(fixture.target_core()?.is_none());
        drop(lock);
        let journal = fixture.journal_until("rolled_back")?;
        ensure!(journal["phase"]["failed"] == "start_target", "{journal}");
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(!fixture.source.state.join("core-placement.json").exists());
        ensure!(fixture.source.state.join("core-state.json").is_file());
        // The target holds no brain state, only the copy for a retry.
        ensure!(!fixture.target.state.join("core-state.json").exists());
        ensure!(fixture.target.record("core-handover.json")?.is_none());
        let incoming = fixture.target.state.join("move-incoming").join(&intent);
        ensure!(incoming.join("core-state.json").is_file(), "no copy kept");

        fixture.device_ready()?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        wait_for("the retry started", || {
            Ok((records(&fixture, "move.started")?.len() == 2).then_some(()))
        })?;
        let journal = fixture.journal_until("done")?;
        ensure!(
            journal["intent"] == intent.as_str(),
            "the retry took a new intent"
        );
        let sent: Vec<Value> = records(&fixture, "copy.sent")?;
        ensure!(sent.len() == 2, "{sent:?}");
        // The source's core ran between the two tries, so a store it wrote
        // since is sent again; the rest the target already held.
        let files = sent[0]["uploaded"].as_array().context("first try")?.len();
        let held = sent[1]["held"].as_u64().context("held")?;
        let uploaded = sent[1]["uploaded"].as_array().context("uploaded")?;
        ensure!(
            held >= 5 && held as usize + uploaded.len() == files,
            "the retry sent the copy again: {sent:?}"
        );
        ensure!(fixture.target_core()?.is_some(), "no core on the target");
        ensure!(!incoming.exists(), "the copy stayed after the move");
        Ok(())
    })();
    finish(fixture, journey)
}

/// A source killed while its target's core starts finds the move in its
/// journal at its next start and undoes it before it runs anything.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_source_killed_mid_move_rolls_back_on_its_next_start() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        let lock = hided::state_file::acquire_lock(&fixture.target.state)?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        fixture.logged(&fixture.source, "abort.failed")?;
        fixture.kill_source()?;
        drop(lock);
        fixture.start_source()?;
        let journal = fixture.journal_until("rolled_back")?;
        ensure!(journal["phase"]["cause"]["kind"] == "local", "{journal}");
        ensure!(journal["phase"]["failed"] == "start_target", "{journal}");
        fixture.logged(&fixture.source, "move.resumed")?;
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(!fixture.target.state.join("core-state.json").exists());
        ensure!(fixture.target_core()?.is_none());
        Ok(())
    })();
    finish(fixture, journey)
}

/// A source killed while its core stopped for a move, before anything was
/// sent, finds the move in its journal at its next start and starts its
/// core again on its untouched folder; the target was never touched.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_source_killed_while_its_core_stops_starts_it_again() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        let target_before = listing(&fixture.target.state)?;
        // A move's journal, from a move that stopped at its staging.
        let planted = fixture.source.state.join("move-staging");
        std::fs::write(&planted, b"")?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let mut journal = fixture.journal_until("rolled_back")?;
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        std::fs::remove_file(&planted)?;
        fixture.kill_source()?;
        // What a kill while the core stops leaves: the journal at stopping,
        // written before the stop, and nothing staged or sent.
        journal["phase"] = json!({"direction": "forward", "phase": "stopping"});
        write_record(&fixture.source.state.join("core-move.json"), &journal)?;
        fixture.start_source()?;
        let journal = fixture.journal_until("rolled_back")?;
        ensure!(journal["phase"]["failed"] == "stop_core", "{journal}");
        fixture.logged(&fixture.source, "move.resumed")?;
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        fixture.device_ready()?;
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(
            listing(&fixture.target.state)? == target_before,
            "the target changed"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// A stop asked while a move's check waits on a peer that stalls ends the
/// process within `hide stop`'s grace, after its core stopped as any stop
/// stops it, and the next start runs the core unchanged.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_stop_during_a_stalled_check_ends_the_process_with_its_core_stopped() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        fixture.device_ready()?;
        let program = fixture.helper_program()?;
        let asked = fixture.root.join("inspect-asked");
        fixture.stand_in_peer(
            &program,
            &format!(
                r#"if [ "$1 $2" = "core-move inspect" ]; then : > '{asked}'; exec sleep 20; fi"#,
                asked = asked.display()
            ),
        )?;
        fixture.event("core_move", json!({"action": "check", "device": DEVICE_ID}))?;
        wait_for("the check on the peer", || Ok(asked.exists().then_some(())))?;
        fixture.stop_source()?;
        let ended = fixture.source_ended_within(std::time::Duration::from_secs(5))?;
        ensure!(ended.success(), "the source's hided: {ended}");
        fixture.logged(&fixture.source, "process.stopped")?;
        ensure!(
            records(&fixture, "core.stop_unconfirmed")?.is_empty(),
            "the core's stop was not confirmed"
        );
        ensure!(
            fixture.source.record("hided.json")?.is_none(),
            "the stopped process left its record"
        );
        fixture.start_source()?;
        ensure!(fixture.role()? == "core");
        fixture.device_ready()?;
        ensure!(visible(&fixture)? == before, "the window changed");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The source's core does not stop for the move (a fixture makes its stop
/// hang): past the stop's bound the process ends unsuccessfully, which a
/// login item restarts, and its next start undoes the move and runs the core
/// on its folder; the target was never touched.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_core_that_will_not_stop_is_restarted_unchanged() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        let target_before = listing(&fixture.target.state)?;
        fixture.kill_source()?;
        fixture
            .source
            .herdr
            .environment
            .set(hided::env::HIDE_FIXTURE_CORE_STOP, "hang");
        fixture.start_source()?;
        fixture.device_ready()?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        // Past the stop's 20 s bound, and before a stop with none would
        // ever end.
        let ended = fixture.source_ended_within(std::time::Duration::from_secs(40))?;
        ensure!(ended.code() == Some(3), "the source's hided: {ended}");
        let unconfirmed = fixture.logged(&fixture.source, "core.stop_unconfirmed")?;
        ensure!(unconfirmed["cause"] == "timed_out", "{unconfirmed}");
        let journal = fixture
            .source
            .record("core-move.json")?
            .context("journal")?;
        ensure!(journal["phase"]["phase"] == "stopping", "{journal}");

        fixture
            .source
            .herdr
            .environment
            .unset(hided::env::HIDE_FIXTURE_CORE_STOP);
        fixture.start_source()?;
        let journal = fixture.journal_until("rolled_back")?;
        ensure!(journal["phase"]["failed"] == "stop_core", "{journal}");
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        fixture.device_ready()?;
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(
            listing(&fixture.target.state)? == target_before,
            "the target changed"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// The new core took the link but the source never heard it: at its next
/// start the source reads the target's handover and goes forward.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_lost_link_answer_after_the_target_took_the_move_goes_forward() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let mut journal = fixture.journal_until("done")?;
        let after = wait_for("the projects through the new core", || {
            let projects = fixture.projects()?;
            Ok((projects.len() == 2).then_some(projects))
        })?;
        fixture.kill_source()?;
        // The state a lost answer leaves: the source recorded the link as
        // sent, the target recorded the move as active.
        let intent = journal["intent"].as_str().context("intent")?.to_owned();
        journal["phase"] = json!({"direction": "forward", "phase": "attach_sent"});
        write_record(&fixture.source.state.join("core-move.json"), &journal)?;
        let mut placement = fixture
            .source
            .record("core-placement.json")?
            .context("placement")?;
        placement["move_intent"] = json!(intent);
        write_record(
            &fixture.source.state.join("core-placement.json"),
            &placement,
        )?;
        write_record(
            &fixture.target.state.join("core-handover.json"),
            &json!({"version": 1, "intent": intent, "source": SOURCE_NODE, "target": TARGET_NODE, "state": {"state": "active"}}),
        )?;
        fixture.start_source()?;
        fixture.journal_until("done")?;
        ensure!(fixture.role()? == "node");
        ensure!(fixture.target.record("core-handover.json")?.is_none());
        let placement = fixture
            .source
            .record("core-placement.json")?
            .context("placement")?;
        ensure!(placement.get("move_intent").is_none(), "{placement}");
        let projects = wait_for("the projects through the new core", || {
            let projects = fixture.projects()?;
            Ok((projects.len() == 2).then_some(projects))
        })?;
        ensure!(projects == after, "{projects:?}");
        Ok(())
    })();
    finish(fixture, journey)
}

/// A move resumed while the alias it reaches the peer by cannot be read
/// (the SSH config is gone) still starts the daemon: this machine
/// runs its node from the placement and waits on the peer, and once the
/// alias resolves again the peer's handover takes the move forward.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_resumed_move_whose_alias_cannot_be_read_waits_for_its_peer() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let mut journal = fixture.journal_until("done")?;
        fixture.kill_source()?;
        let intent = journal["intent"].as_str().context("intent")?.to_owned();
        journal["phase"] = json!({"direction": "forward", "phase": "attach_sent"});
        write_record(&fixture.source.state.join("core-move.json"), &journal)?;
        let mut placement = fixture
            .source
            .record("core-placement.json")?
            .context("placement")?;
        placement["move_intent"] = json!(intent);
        write_record(
            &fixture.source.state.join("core-placement.json"),
            &placement,
        )?;
        write_record(
            &fixture.target.state.join("core-handover.json"),
            &json!({"version": 1, "intent": intent, "source": SOURCE_NODE, "target": TARGET_NODE, "state": {"state": "active"}}),
        )?;
        let config = fixture.source.home().join(".ssh/config");
        let away = config.with_file_name("config.away");
        std::fs::rename(&config, &away)?;
        fixture.start_source()?;
        let failed = fixture.logged_after_link_wait(&fixture.source, "status.failed")?;
        ensure!(
            failed["failure"]["kind"] == "unreachable"
                && failed["failure"]["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.starts_with("ssh_alias")),
            "{failed}"
        );
        ensure!(fixture.role()? == "node");
        std::fs::rename(&away, &config)?;
        fixture.journal_until("done")?;
        ensure!(fixture.target.record("core-handover.json")?.is_none());
        Ok(())
    })();
    finish(fixture, journey)
}

/// The new core refuses this machine's first link (here it cannot record
/// the commit while another change holds its record): this machine runs no
/// core until that machine's pending core is stopped and its copy taken
/// back, then starts its own unchanged.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_refused_first_link_rolls_back() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        wait_for("the copy placed on the target", || {
            Ok(fixture
                .target
                .record("core-handover.json")?
                .filter(|record| record["state"]["state"] == "pending"))
        })?;
        let held = fixture.hold_target_handover()?;
        let refused = fixture.logged(&fixture.target, "attach.refused")?;
        ensure!(
            refused["node"] == SOURCE_NODE && refused["reason"] == "move_unavailable",
            "{refused}"
        );
        fixture.logged(&fixture.source, "abort.failed")?;
        // Never two cores: the target's core may still take a link, so
        // this machine runs none.
        ensure!(fixture.role()? == "moving");
        let pending = fixture
            .target_core()?
            .context("the target's pending core")?;
        drop(held);
        let journal = fixture.journal_until("rolled_back")?;
        ensure!(journal["phase"]["failed"] == "reattach", "{journal}");
        ensure!(
            journal["phase"]["cause"]["kind"] == "link_refused",
            "{journal}"
        );
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(!hide_platform::process::is_alive(pending));
        ensure!(fixture.target_core()?.is_none());
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(!fixture.source.state.join("core-placement.json").exists());
        ensure!(!fixture.target.state.join("core-state.json").exists());
        ensure!(fixture.target.record("core-handover.json")?.is_none());
        Ok(())
    })();
    finish(fixture, journey)
}

/// The abandon a move's wait offers, sent from a window on whatever role
/// the address mounts; answers why it was refused, none when it was taken.
fn abandon(window: &mut Renderer) -> Result<Option<String>> {
    window.ask_ending("core_move", json!({"action": "abandon"}))
}

/// Waits on `window` for the move's wait to offer an abandon.
fn abandon_offered(window: &mut Renderer) -> Result<Value> {
    window.core_move_until(std::time::Duration::from_secs(60), |view| {
        view["state"] == "waiting" && view["abandon"] == true
    })
}

/// Amendment 4: the peer took the move's link but its answer was lost, and
/// the peer is then cut off, so this machine cannot know whether the move
/// committed. The operator gives the wait up: this machine's core starts
/// at once on its own folder as the move found it, while the cut-off
/// peer's core keeps running. Once the peer is reached again, running
/// another build by then, this build is installed there and stops that
/// core, its state set aside, so one core runs. An abandon asked outside
/// such a wait is refused; a second one is the control's unit test, since
/// here the window it would come from closes with the move.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_move_given_up_while_its_peer_is_cut_off_ends_the_peer_s_core_once_it_is_back() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        let before = fixture.projects()?;
        ensure!(
            abandon(&mut fixture.held_window()?)?.as_deref() == Some("move_not_waiting"),
            "an abandon with no move waiting was taken"
        );
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let mut journal = fixture.journal_until("done")?;
        let stray = fixture.target_core()?.context("the target's core")?;
        fixture.kill_source()?;
        let intent = journal["intent"].as_str().context("intent")?.to_owned();
        // The answer was lost before this machine committed, so its folder
        // is as the move found it: what the commit set aside is put back.
        let aside = fixture.source.state.join("moved-out").join(&intent);
        for entry in std::fs::read_dir(&aside)? {
            let entry = entry?;
            std::fs::rename(entry.path(), fixture.source.state.join(entry.file_name()))?;
        }
        std::fs::remove_dir(&aside)?;
        // By the time it is reached again the peer runs an older build,
        // which has no abandon step: it answers its usage as its own build's
        // line and does nothing.
        let helper = std::path::PathBuf::from(
            journal["peer"]["program"]
                .as_str()
                .context("the peer's program")?,
        );
        let folder = helper
            .parent()
            .and_then(std::path::Path::parent)
            .context("the peer's helper root")?
            .join("0000000000000000");
        std::fs::create_dir_all(&folder)?;
        let older = folder.join("hided");
        std::fs::write(
            &older,
            "#!/bin/sh\necho '{\"build\":\"older\",\"answer\":{\"kind\":\"refused\",\"reason\":\"usage\"}}'\nexit 2\n",
        )?;
        std::fs::set_permissions(&older, std::fs::Permissions::from_mode(0o755))?;
        journal["peer"]["program"] = json!(folder.join("hided"));
        // The state a lost link answer leaves.
        journal["phase"] = json!({"direction": "forward", "phase": "attach_sent"});
        write_record(&fixture.source.state.join("core-move.json"), &journal)?;
        let mut placement = fixture
            .source
            .record("core-placement.json")?
            .context("placement")?;
        placement["move_intent"] = json!(intent);
        write_record(
            &fixture.source.state.join("core-placement.json"),
            &placement,
        )?;
        write_record(
            &fixture.target.state.join("core-handover.json"),
            &json!({"version": 1, "intent": intent, "source": SOURCE_NODE, "target": TARGET_NODE, "state": {"state": "active"}}),
        )?;
        fixture.ssh.online(false)?;
        fixture.start_source()?;
        fixture.logged_after_link_wait(&fixture.source, "status.failed")?;
        ensure!(fixture.role()? == "node");
        let mut window = fixture.held_window()?;
        let offered = abandon_offered(&mut window)?;
        // A window held before it drew the core names the peer by this.
        ensure!(offered["alias"] == ALIAS, "{offered}");
        let refused = abandon(&mut window)?;
        if refused.is_some() {
            let now = fixture
                .held_window()?
                .core_move_until(std::time::Duration::from_secs(5), |_| true)?;
            bail!("the offered abandon was refused: {refused:?}; offered {offered}; now {now}");
        }
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(fixture.source.record("core-placement.json")?.is_none());
        let journal = fixture
            .source
            .record("core-move.json")?
            .context("journal")?;
        ensure!(journal["phase"]["phase"] == "abandoned", "{journal}");
        let kept = fixture
            .source
            .record("core-move-abandoned.json")?
            .context("the moves given up")?;
        ensure!(kept["moves"][0]["intent"] == intent.as_str(), "{kept}");
        wait_for("this machine's projects", || {
            Ok((fixture.projects()? == before).then_some(()))
        })?;
        ensure!(
            hide_platform::process::is_alive(stray),
            "the cut-off peer's core ended before it was reached"
        );
        fixture.ssh.online(true)?;
        wait_for("the peer's core stopped, leaving one core", || {
            Ok((!hide_platform::process::is_alive(stray)).then_some(()))
        })?;
        ensure!(fixture.target_core()?.is_none(), "a core runs on the peer");
        let installed = fixture.logged(&fixture.source, "abandon.installed")?;
        ensure!(
            installed["intent"] == intent.as_str() && installed["was"] == json!(older),
            "{installed}"
        );
        fixture.logged(&fixture.source, "abandon.ended")?;
        ensure!(fixture.target.record("core-handover.json")?.is_none());
        ensure!(
            fixture
                .target
                .state
                .join("moved-out")
                .join(&intent)
                .join("core-state.json")
                .is_file(),
            "the peer's core state was not set aside"
        );
        ensure!(fixture.source.record("core-move-abandoned.json")?.is_none());
        ensure!(fixture.role()? == "core");
        Ok(())
    })();
    finish(fixture, journey)
}

/// Amendment 4: the pending core cannot record the commit, so it refuses
/// the link, and the rollback's abort waits on it while it is busy. The
/// operator gives the wait up: this machine's core starts, and once the
/// peer can act its pending core is stopped and its state set aside,
/// leaving nothing of the move on either machine.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_move_given_up_while_its_pending_peer_is_busy_leaves_nothing_of_it() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = fixture.projects()?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        wait_for("the copy placed on the target", || {
            Ok(fixture
                .target
                .record("core-handover.json")?
                .filter(|record| record["state"]["state"] == "pending"))
        })?;
        let held = fixture.hold_target_handover()?;
        fixture.logged(&fixture.target, "attach.refused")?;
        fixture.logged(&fixture.source, "abort.failed")?;
        ensure!(fixture.role()? == "moving");
        let pending = fixture
            .target_core()?
            .context("the target's pending core")?;
        let intent = fixture
            .source
            .record("core-move.json")?
            .context("journal")?["intent"]
            .as_str()
            .context("intent")?
            .to_owned();
        let mut window = fixture.held_window()?;
        abandon_offered(&mut window)?;
        ensure!(
            abandon(&mut window)?.is_none(),
            "the offered abandon was refused"
        );
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        wait_for("this machine's projects", || {
            Ok((fixture.projects()? == before).then_some(()))
        })?;
        // The peer cannot act while its record is held: its core waits.
        fixture.logged(&fixture.source, "abandon.failed")?;
        ensure!(hide_platform::process::is_alive(pending));
        drop(held);
        wait_for("the pending core stopped", || {
            Ok((!hide_platform::process::is_alive(pending)).then_some(()))
        })?;
        ensure!(fixture.target_core()?.is_none());
        fixture.logged(&fixture.source, "abandon.ended")?;
        ensure!(fixture.target.record("core-handover.json")?.is_none());
        ensure!(!fixture.target.state.join("core-state.json").exists());
        ensure!(
            !fixture
                .target
                .state
                .join("move-incoming")
                .join(&intent)
                .exists(),
            "the move's copy is left on the peer"
        );
        ensure!(fixture.source.record("core-move-abandoned.json")?.is_none());
        let journal = fixture
            .source
            .record("core-move.json")?
            .context("journal")?;
        ensure!(journal["phase"]["phase"] == "abandoned", "{journal}");
        Ok(())
    })();
    finish(fixture, journey)
}

/// Amendment 4: a move back whose core machine does not answer its
/// retirement offers to take the core here. Taken, the move back commits
/// on the copy placed here and this machine runs the core, while the core's
/// machine stays held for the move: a start there, as its login item would
/// make, starts no core. Once that machine answers, it retires as it would
/// have, its state set aside, and nothing of the move is left to ask.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_move_back_taken_here_retires_the_held_core_once_its_machine_answers() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = fixture.projects()?;
        moved_forward(&fixture)?;
        let program = std::path::PathBuf::from(placed_program(&fixture)?);
        let asked = fixture.root.join("retire-asked");
        let go = fixture.root.join("retire-go");
        // The first retirement waits until the test holds the core
        // machine's handover, which every retirement then waits on.
        fixture.stand_in_peer(
            &program,
            &format!(
                r#"if [ "$1 $2" = "core-move retire" ] && [ ! -e '{go}' ]; then
  : > '{asked}'
  while [ ! -e '{go}' ]; do sleep 0.1; done
fi"#,
                asked = asked.display(),
                go = go.display()
            ),
        )?;
        fixture.event("core_move", json!({"action": "back"}))?;
        wait_for("the retirement asked", || Ok(asked.exists().then_some(())))?;
        let held = fixture.hold_target_handover()?;
        std::fs::write(&go, b"")?;
        let mut window = fixture.held_window()?;
        let offered = abandon_offered(&mut window)?;
        ensure!(offered["direction"] == "back", "{offered}");
        ensure!(offered["alias"] == ALIAS, "{offered}");
        let refused = abandon(&mut window)?;
        ensure!(
            refused.is_none(),
            "taking the core here was refused: {refused:?}"
        );
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        back_journal_until(&fixture, "done")?;
        ensure!(!fixture.source.state.join("core-placement.json").exists());
        let kept = fixture
            .source
            .record("core-move-abandoned.json")?
            .context("the retirement owed")?;
        ensure!(kept["moves"][0]["step"] == "retire", "{kept}");
        let after = fixture.projects()?;
        ensure!(after == before, "before: {before:?}\nafter: {after:?}");
        // The core's machine is held for the move meanwhile.
        let handover = fixture
            .target
            .record("core-handover.json")?
            .context("the core machine's handover")?;
        ensure!(handover["state"]["state"] == "stopped_for", "{handover}");
        let ended = fixture.start_target_hided()?;
        ensure!(ended.success(), "the held machine's hided: {ended}");
        ensure!(fixture.target_core()?.is_none(), "a held core started");
        drop(held);
        wait_for("the held core retired", || {
            Ok(fixture
                .target
                .record("core-handover.json")?
                .filter(|handover| handover["state"]["state"] == "retired"))
        })?;
        // The retirement answers once its state is set aside, after the
        // record says retired.
        let ended = fixture.logged(&fixture.source, "abandon.ended")?;
        ensure!(ended["answer"] == "retired", "{ended}");
        ensure!(!fixture.target.state.join("core-state.json").exists());
        ensure!(
            records(&fixture, "retire.leftover")?.is_empty(),
            "the retirement left part of the folder"
        );
        wait_for("nothing of the move left to ask", || {
            Ok(fixture
                .source
                .record("core-move-abandoned.json")?
                .is_none()
                .then_some(()))
        })?;
        ensure!(fixture.target_core()?.is_none());
        ensure!(fixture.role()? == "core");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The target stops its pending core for the rollback but cannot take the
/// copy back out of its folder: it says so, and this machine, knowing no
/// core of the move runs or starts there, starts its own core rather than
/// asking again forever with no core on either machine.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_copy_the_target_cannot_take_back_still_restarts_the_old_core() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let placed = wait_for("the copy placed on the target", || {
            Ok(fixture
                .target
                .record("core-handover.json")?
                .filter(|record| record["state"]["state"] == "pending"))
        })?;
        // Held, the record refuses the link, so the move is undone.
        let held = fixture.hold_target_handover()?;
        fixture.logged(&fixture.source, "abort.failed")?;
        let intent = placed["intent"].as_str().context("intent")?;
        // The labels go back to a name a folder holds that cannot be
        // emptied.
        let blocked = fixture
            .target
            .state
            .join("move-incoming")
            .join(intent)
            .join("labels.json");
        std::fs::create_dir_all(blocked.join("kept"))?;
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o500))?;
        drop(held);
        let journal = fixture.journal_until("rolled_back");
        std::fs::set_permissions(&blocked, std::fs::Permissions::from_mode(0o700))?;
        let journal = journal?;
        ensure!(
            journal["phase"]["cause"]["kind"] == "link_refused",
            "{journal}"
        );
        let unclean = fixture.logged(&fixture.source, "rollback.unclean")?;
        ensure!(
            unclean["peer"]
                .as_str()
                .is_some_and(|peer| peer.contains("labels.json")),
            "{unclean}"
        );
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(
            fixture.target_core()?.is_none(),
            "a core runs on the target"
        );
        ensure!(visible(&fixture)? == before, "the window changed");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The answer to the place is lost and the target cannot be asked to take
/// the copy back: no core was started on it, so this machine starts its
/// own core at once, and the retry's checks take the stranded copy back
/// before the move goes on with the same intent.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_copy_stranded_by_a_lost_place_answer_is_taken_back_by_the_retry() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let program = fixture.helper_program()?;
        let allowed = std::path::PathBuf::from(format!("{}.abort-allowed", program.display()));
        let lost = std::path::PathBuf::from(format!("{}.place-lost", program.display()));
        // The first place runs and its answer is lost; the abort fails until
        // it is allowed.
        fixture.stand_in_peer(
            &program,
            &format!(
                r#"case "$1 $2" in
  "core-move place") if [ ! -e '{lost}' ]; then : > '{lost}'; "$0.real" "$@" > /dev/null; exit 0; fi ;;
  "core-move abort") [ -e '{allowed}' ] || exit 1 ;;
esac"#,
                lost = lost.display(),
                allowed = allowed.display()
            ),
        )?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let first = fixture.journal_until("rolled_back")?;
        ensure!(
            first["phase"]["failed"] == "copy" && first["phase"]["cause"]["step"] == "place",
            "{first}"
        );
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        let stranded = fixture
            .target
            .record("core-handover.json")?
            .context("the stranded copy's record")?;
        ensure!(stranded["state"]["state"] == "pending", "{stranded}");
        ensure!(fixture.target.state.join("node.json").is_file());
        std::fs::write(&allowed, b"")?;
        fixture.device_ready()?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        wait_for("the retry started", || {
            Ok((records(&fixture, "move.started")?.len() == 2).then_some(()))
        })?;
        let journal = fixture.journal_until("done")?;
        ensure!(journal["intent"] == first["intent"], "{journal}");
        ensure!(fixture.target_core()?.is_some(), "no core on the target");
        fixture.logged(&fixture.source, "retry.aborted_stranded")?;
        Ok(())
    })();
    finish(fixture, journey)
}

/// This machine cannot stage its copy after its core stopped: nothing
/// reached the target, and the core starts again on its untouched folder.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_staging_failure_restarts_the_old_core() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        let target_before = listing(&fixture.target.state)?;
        // This machine cannot make its staging folder.
        std::fs::write(fixture.source.state.join("move-staging"), b"")?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let journal = fixture.journal_until("rolled_back")?;
        ensure!(journal["phase"]["failed"] == "copy", "{journal}");
        ensure!(journal["phase"]["cause"]["kind"] == "staging", "{journal}");
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(!fixture.source.state.join("core-placement.json").exists());
        ensure!(
            listing(&fixture.target.state)? == target_before,
            "the target changed"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// The connection drops partway through the upload: the core starts again
/// unchanged, and the retry keeps the move's intent and sends again only
/// what the target does not already hold whole.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn an_upload_cut_midway_resumes_by_digest() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        // Armed once the core is stopping, so only the move's own upload
        // counts: the manifest, then the copy's files in name order. The
        // cut comes as the fourth of them opens, three already whole.
        fixture.journal_until("stopping")?;
        fixture.ssh.cut_sftp_at_open(5);
        // The connection's end ends the upload, well before an SFTP
        // request's 30 s limit would.
        let journal = wait_within(
            "the cut move rolled back",
            std::time::Duration::from_secs(15),
            || {
                Ok(fixture
                    .source
                    .record("core-move.json")?
                    .filter(|journal| journal["phase"]["phase"] == "rolled_back"))
            },
        )?;
        ensure!(journal["phase"]["failed"] == "copy", "{journal}");
        let intent = journal["intent"].as_str().context("intent")?.to_owned();
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(!fixture.target.state.join("core-state.json").exists());
        ensure!(fixture.target.record("core-handover.json")?.is_none());
        let incoming = fixture.target.state.join("move-incoming").join(&intent);
        ensure!(incoming.is_dir(), "nothing of the copy reached the target");
        let whole: Vec<String> = listing(&incoming)?
            .into_iter()
            .filter(|name| !name.ends_with(".part"))
            .collect();
        ensure!(whole.len() == 3, "the cut came elsewhere: {whole:?}");
        let manifest: Value = serde_json::from_slice(&std::fs::read(
            fixture
                .target
                .state
                .join("move-incoming")
                .join(format!("{intent}.manifest.json")),
        )?)?;
        let files = manifest["files"].as_object().context("manifest")?.len();

        fixture.device_ready()?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        wait_for("the retry started", || {
            Ok((records(&fixture, "move.started")?.len() == 2).then_some(()))
        })?;
        let journal = fixture.journal_until("done")?;
        ensure!(
            journal["intent"] == intent.as_str(),
            "the retry took a new intent"
        );
        let sent = records(&fixture, "copy.sent")?;
        ensure!(
            sent.len() == 1,
            "the cut try reported a whole copy: {sent:?}"
        );
        // The source's core ran between the tries, so its state store is
        // sent again; the two small files before the cut are held.
        let held = sent[0]["held"].as_u64().context("held")?;
        let uploaded = sent[0]["uploaded"].as_array().context("uploaded")?;
        ensure!(
            held >= 1 && held as usize + uploaded.len() == files,
            "the retry sent the copy again: {sent:?}"
        );
        ensure!(fixture.target_core()?.is_some(), "no core on the target");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The target refuses to place the copy, here because its folder gained
/// brain state of its own after the checks: that state stays its own and
/// in place, the copy waits in `move-incoming` for a retry, and this
/// machine's core starts again unchanged.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_place_failure_leaves_no_brain_state_on_the_target() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        // The place waits for the record's lock, so the folder changes
        // after the checks and before the place reads it.
        let held = fixture.hold_target_handover()?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let journal = fixture.journal_until("placed")?;
        let intent = journal["intent"].as_str().context("intent")?.to_owned();
        let own = fixture.target.state.join("labels.json");
        std::fs::write(&own, b"{\"own\":true}")?;
        drop(held);
        let journal = fixture.journal_until("rolled_back")?;
        ensure!(journal["phase"]["failed"] == "copy", "{journal}");
        ensure!(
            journal["phase"]["cause"]["kind"] == "refused"
                && journal["phase"]["cause"]["step"] == "place",
            "{journal}"
        );
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(
            std::fs::read(&own)? == b"{\"own\":true}",
            "the target's own state changed"
        );
        ensure!(!fixture.target.state.join("core-state.json").exists());
        ensure!(!fixture.target.state.join("node.json").exists());
        ensure!(fixture.target.record("core-handover.json")?.is_none());
        ensure!(fixture.target_core()?.is_none());
        let incoming = fixture.target.state.join("move-incoming").join(&intent);
        ensure!(
            incoming.join("node.json").is_file(),
            "the copy was not kept"
        );
        ensure!(
            std::fs::read(incoming.join("labels.json"))? != b"{\"own\":true}",
            "the target's own state went into the copy"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// The target's answer names a file the copy does not hold, here one of
/// this machine's own by its absolute path: the move refuses the answer and
/// rolls back, and no file of this machine's beyond the copy reaches the
/// target.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_target_that_asks_for_a_file_outside_the_copy_is_refused() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        let own = fixture.source.home().join("outside-the-copy");
        std::fs::write(&own, b"this machine's own")?;
        let own_path = own.to_str().context("path")?;
        let program = fixture.helper_program()?;
        // Answered as the target's own build would, so only its content is
        // hostile.
        let build = hided::build_id::of_file(&program).map_err(anyhow::Error::msg)?;
        let answer = serde_json::to_string(&json!({
            "build": build,
            "answer": {"kind": "differs", "differs": [own_path], "extra": []},
        }))?;
        fixture.stand_in_peer(
            &program,
            &format!(
                r#"if [ "$1 $2" = "core-move verify" ]; then
  printf '%s\n' '{answer}'
  exit 0
fi"#
            ),
        )?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let journal = fixture.journal_until("rolled_back")?;
        let cause = &journal["phase"]["cause"];
        ensure!(
            journal["phase"]["failed"] == "copy"
                && cause["kind"] == "refused"
                && cause["step"] == "verify"
                && cause["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains(own_path)),
            "{journal}"
        );
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(visible(&fixture)? == before, "the window changed");
        let reached = files_named(&fixture.root.join("c"), "outside-the-copy")?;
        ensure!(reached.is_empty(), "it reached the target: {reached:?}");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The target's core cannot be confirmed stopped when the move is undone
/// (here its record cannot be read; a login item that cannot be removed
/// fails the same step): this machine waits and runs no core, and rolls
/// back once that machine answers that its core is gone.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_target_core_that_cannot_be_stopped_is_waited_for() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        wait_for("the copy placed on the target", || {
            Ok(fixture
                .target
                .record("core-handover.json")?
                .filter(|record| record["state"]["state"] == "pending"))
        })?;
        // The first link is refused, as in `a_refused_first_link_rolls_back`,
        // so the move is undone with the target's core running.
        let held = fixture.hold_target_handover()?;
        fixture.logged(&fixture.source, "abort.failed")?;
        let pending = fixture
            .target_core()?
            .context("the target's pending core")?;
        let record = fixture.target.state.join("hided.json");
        let readable = std::fs::metadata(&record)?.permissions();
        std::fs::set_permissions(&record, std::fs::Permissions::from_mode(0o000))?;
        drop(held);
        wait_for("the stop refused", || {
            Ok(records(&fixture, "abort.failed")?
                .into_iter()
                .find(|record| {
                    record["failure"]["reason"]
                        .as_str()
                        .is_some_and(|reason| reason.contains("the core's state could not be read"))
                }))
        })?;
        ensure!(fixture.role()? == "moving", "this machine started a core");
        ensure!(hide_platform::process::is_alive(pending));
        std::fs::set_permissions(&record, readable)?;
        let journal = fixture.journal_until("rolled_back")?;
        ensure!(journal["phase"]["failed"] == "reattach", "{journal}");
        wait_for("the source's core again", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(!hide_platform::process::is_alive(pending));
        ensure!(visible(&fixture)? == before, "the window changed");
        ensure!(!fixture.target.state.join("core-state.json").exists());
        Ok(())
    })();
    finish(fixture, journey)
}

/// B6: from the node's window the core comes back to its machine; the
/// window keeps its address and shows what it showed before the core left,
/// and the other machine is a device again with no core of its own.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn the_core_moves_back_and_the_window_shows_what_it_showed_before() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        let settings = std::fs::read_to_string(fixture.source.ai_settings())?;
        let window = fixture.window();
        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        let forward = fixture.journal_until("done")?;
        wait_for("the projects through the new core", || {
            Ok((fixture.projects()?.len() == 2).then_some(()))
        })?;
        ensure!(fixture.role()? == "node");
        ensure!(
            std::fs::read_to_string(fixture.target.ai_settings())? == settings,
            "the target holds other settings"
        );
        ensure!(!fixture.source.ai_settings().exists());
        ensure!(
            fixture
                .source
                .state
                .join("moved-out")
                .join(forward["intent"].as_str().context("intent")?)
                .join("ai.json")
                .is_file()
        );

        // A window open on the node when the move back starts is closed as
        // the node stops, and its reconnect hears the move (W1).
        let mut open = Renderer::connect(window.0, &window.1)?;
        fixture.event("core_move", json!({"action": "back"}))?;
        let (code, reason) = open.closed_within(std::time::Duration::from_secs(10))?;
        ensure!(
            code == 1012,
            "the open window was closed as {code} {reason}"
        );
        let answering = fixture.health()?;
        ensure!(
            matches!(answering["role"].as_str(), Some("moving" | "core")),
            "the window's address answered {answering} once its socket closed"
        );
        let journal = wait_for("the move back done", || {
            let journal = fixture.source.record("core-move.json")?;
            if let Some(journal) = &journal
                && journal["phase"]["direction"] == "back"
                && journal["phase"]["phase"] == "rolled_back"
            {
                bail!("the move back rolled back: {journal}");
            }
            Ok(journal.filter(|journal| {
                journal["phase"]["direction"] == "back" && journal["phase"]["phase"] == "done"
            }))
        })?;
        ensure!(journal["intent"] != forward["intent"], "{journal}");
        let intent = journal["intent"].as_str().context("intent")?;
        // Hide AI's settings went with the core and came back with it.
        ensure!(
            std::fs::read_to_string(fixture.source.ai_settings())? == settings,
            "the settings did not come back"
        );
        ensure!(!fixture.target.ai_settings().exists());
        ensure!(
            fixture
                .target
                .state
                .join("moved-out")
                .join(intent)
                .join("ai.json")
                .is_file()
        );
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        let state = fixture.source.daemon()?.context("source state")?;
        ensure!(
            (state["port"].as_u64(), state["token"].as_str())
                == (Some(u64::from(window.0)), Some(window.1.as_str())),
            "the window's address changed: {state}"
        );
        ensure!(!fixture.source.state.join("core-placement.json").exists());
        ensure!(fixture.source.state.join("core-state.json").is_file());

        // The other machine runs no core and keeps its last state aside.
        ensure!(
            fixture.target_core()?.is_none(),
            "the target still runs a core"
        );
        ensure!(!fixture.target.state.join("core-state.json").exists());
        let moved_out = fixture.target.state.join("moved-out").join(intent);
        ensure!(
            moved_out.join("core-state.json").is_file(),
            "nothing set aside there"
        );
        let handover = fixture
            .target
            .record("core-handover.json")?
            .context("handover")?;
        ensure!(handover["state"]["state"] == "retired", "{handover}");

        fixture.device_ready()?;
        let after = wait_for("the projects of both machines again", || {
            let now = visible(&fixture)?;
            Ok((now["projects"].as_array().map(Vec::len) == Some(2)).then_some(now))
        })?;
        ensure!(after == before, "before: {before}\nafter: {after}");
        Ok(())
    })();
    finish(fixture, journey)
}

/// A key typed through the window into a pane of the window's machine
/// reaches it after the core leaves that machine and after it comes back.
/// The process keeps running across each role swap, and the role it leaves
/// lets go of every pane it held; a session left holding one would make the
/// next role's attach meet an attached client (`owner_conflict`), fall back
/// to observing, and refuse every key as read-only while the pane is still
/// listed, linked and drawn.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_pane_of_the_window_s_machine_takes_keys_after_the_move_and_the_move_back() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        let pane = fixture.source_pane()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        // The core's own session holds the pane before the core leaves.
        typed_through_window(&fixture, &runtime, &pane, "before")?;

        moved_forward(&fixture)?;
        ensure!(fixture.role()? == "node");
        // The node's own session for the pane, which the moved core names
        // as its device's.
        typed_through_window(
            &fixture,
            &runtime,
            &format!("remote:{SOURCE_NODE}:pane:{pane}"),
            "moved",
        )?;
        one_session_in_control(&fixture, &pane)?;

        fixture.event("core_move", json!({"action": "back"}))?;
        back_journal_until(&fixture, "done")?;
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        typed_through_window(&fixture, &runtime, &pane, "back")?;
        one_session_in_control(&fixture, &pane)?;
        Ok(())
    })();
    finish(fixture, journey)
}

/// Types `echo <word>-"ok"` into `pane` through a window at the source's
/// address once it shows the source's checkout, as the operator's window
/// does, and waits for `<word>-ok` to come back; a refusal names what the
/// source logged about held panes.
fn typed_through_window(
    fixture: &Fixture,
    runtime: &tokio::runtime::Runtime,
    pane: &str,
    word: &str,
) -> Result<()> {
    let (port, token) = fixture.window();
    let (workspace, checkout) = source_checkout(fixture)?;
    runtime
        .block_on(async {
            let mut socket = screen_socket(port, &token).await?;
            // A window draws a pane only once its first state has come, and
            // only then does its view of the pane have output to draw.
            first_snapshot(&mut socket, std::time::Duration::from_secs(30)).await?;
            send(
                &mut socket,
                "focus_device",
                json!({"device_id": SOURCE_NODE}),
            )
            .await?;
            send(
                &mut socket,
                "focus_checkout",
                json!({"workspace_id": workspace, "checkout_id": checkout, "focus_device": true}),
            )
            .await?;
            type_and_read(
                &mut socket,
                pane,
                &format!("echo {word}-\"ok\""),
                &format!("{word}-ok"),
            )
            .await
        })
        .map(|_| ())
        .with_context(|| {
            format!(
                "the keys typed after {word} never reached {pane}; held panes logged: {:?}",
                held_pane_records(fixture)
            )
        })
}

/// The Project and checkout ids of the source's project, from whichever
/// core the window reaches: its own navigator's, or its device's session.
fn source_checkout(fixture: &Fixture) -> Result<(String, String)> {
    let path = fixture.source.project().display().to_string();
    wait_for("the source's checkout listed", || {
        let snapshot = fixture.snapshot()?;
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
}

/// The source runs one `herdr terminal session` for `pane`, in control: a
/// session the role before still held would be a second, and the attach of
/// the role now an observer.
fn one_session_in_control(fixture: &Fixture, pane: &str) -> Result<()> {
    let sessions = fixture.source_sessions(pane)?;
    ensure!(
        sessions.len() == 1 && sessions[0].contains("terminal session control "),
        "the sessions the source runs for {pane}: {sessions:?}"
    );
    Ok(())
}

/// What the source logged about a pane held by another client or refusing
/// keys.
fn held_pane_records(fixture: &Fixture) -> Vec<String> {
    let Ok(logs) = std::fs::read_dir(fixture.source.state.join("Logs")) else {
        return Vec::new();
    };
    logs.flatten()
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .flat_map(|text| {
            text.lines()
                .filter(|line| line.contains("owner_conflict") || line.contains("read_only"))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Moves the core to the target and waits until the window shows both
/// machines' projects through it.
fn moved_forward(fixture: &Fixture) -> Result<Value> {
    fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
    let journal = fixture.journal_until("done")?;
    wait_for("the projects through the new core", || {
        Ok((fixture.projects()?.len() == 2).then_some(()))
    })?;
    Ok(journal)
}

/// The source's move-back journal once it reaches `phase`.
fn back_journal_until(fixture: &Fixture, phase: &str) -> Result<Value> {
    wait_for(&format!("the move back at {phase}"), || {
        Ok(fixture.source.record("core-move.json")?.filter(|journal| {
            journal["phase"]["direction"] == "back" && journal["phase"]["phase"] == phase
        }))
    })
}

/// A move back whose commit cannot remove this machine's placement (a
/// folder is in its way) stays at Retiring and shows the failure, rather
/// than recording a move it cannot finish; once the way is clear the move
/// ends and this machine runs the core.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_move_back_whose_placement_cannot_be_removed_waits_at_retiring() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        moved_forward(&fixture)?;
        let program = std::path::PathBuf::from(placed_program(&fixture)?);
        let placement = fixture.source.state.join("core-placement.json");
        let blocked = fixture.root.join("placement-blocked");
        // The retirement runs, and then the placement cannot be removed.
        fixture.stand_in_peer(
            &program,
            &format!(
                r#"if [ "$1 $2" = "core-move retire" ] && [ ! -e '{blocked}' ]; then
  "$0.real" "$@" || exit
  : > '{blocked}'; rm -f '{placement}'; mkdir -p '{placement}/in-the-way'
  exit 0
fi"#,
                blocked = blocked.display(),
                placement = placement.display()
            ),
        )?;
        fixture.event("core_move", json!({"action": "back"}))?;
        let unfinished = fixture.logged(&fixture.source, "commit.unfinished")?;
        ensure!(
            unfinished["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("core-placement.json")),
            "{unfinished}"
        );
        let journal = fixture
            .source
            .record("core-move.json")?
            .context("journal")?;
        ensure!(journal["phase"]["phase"] == "retiring", "{journal}");
        std::fs::remove_dir_all(&placement)?;
        back_journal_until(&fixture, "done")?;
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        Ok(())
    })();
    finish(fixture, journey)
}

/// A retirement that commits and then cannot set the core machine's folder
/// aside (a file is where it goes) still commits the move back: the
/// leftover is logged and this machine runs the core, with no rollback that
/// would resume a core already retired.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_retirement_that_cannot_set_its_folder_aside_still_commits_the_move_back() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        moved_forward(&fixture)?;
        let moved_out = fixture.target.state.join("moved-out");
        ensure!(
            !moved_out.exists(),
            "the target set something aside already"
        );
        std::fs::write(&moved_out, b"")?;
        fixture.event("core_move", json!({"action": "back"}))?;
        back_journal_until(&fixture, "done")?;
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        let leftover = fixture.logged(&fixture.source, "retire.leftover")?;
        ensure!(
            leftover["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("moved-out")),
            "{leftover}"
        );
        ensure!(
            records(&fixture, "move.rolling_back")?.is_empty(),
            "the move back rolled back"
        );
        let handover = fixture
            .target
            .record("core-handover.json")?
            .context("the target's record")?;
        ensure!(handover["state"]["state"] == "retired", "{handover}");
        ensure!(
            fixture.target_core()?.is_none(),
            "a core runs on the retired machine"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// A move back the core's machine refuses to resume after (here both its
/// retirement and its resume are asked of another move) waits with the copy
/// it placed, rather than taking it out and leaving neither machine a core;
/// once the resume is answered that machine runs its core again and this
/// one is its node.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_move_back_whose_core_machine_refuses_to_resume_keeps_its_copy_and_waits() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        moved_forward(&fixture)?;
        let program = std::path::PathBuf::from(placed_program(&fixture)?);
        let refuse = fixture.root.join("refuse-resume");
        let asked = fixture.root.join("resumes-refused");
        std::fs::write(&refuse, b"")?;
        fixture.stand_in_peer(
            &program,
            &format!(
                r#"if [ "$1 $2" = "core-move resume" ] && [ -e '{refuse}' ]; then
  echo refused >> '{asked}'
fi
if [ "$1 $2" = "core-move retire" ] || {{ [ "$1 $2" = "core-move resume" ] && [ -e '{refuse}' ]; }}; then
  n=$#; i=0
  while [ $i -lt $n ]; do
    arg=$1; shift; i=$((i + 1))
    if [ "$arg" = --intent ]; then
      shift; i=$((i + 1)); set -- "$@" --intent another-move
    else
      set -- "$@" "$arg"
    fi
  done
  exec "$0.real" "$@"
fi"#,
                refuse = refuse.display(),
                asked = asked.display()
            ),
        )?;
        fixture.event("core_move", json!({"action": "back"}))?;
        let failed = fixture.logged(&fixture.source, "resume.failed")?;
        ensure!(
            failed["failure"]["reason"]
                .as_str()
                .is_some_and(|reason| reason.contains("another move")),
            "{failed}"
        );
        // Asked again, with the copy still in place and no core anywhere.
        wait_for("the refused resume asked again", || {
            let refused = std::fs::read_to_string(&asked).unwrap_or_default();
            Ok((refused.lines().count() >= 2).then_some(()))
        })?;
        ensure!(fixture.role()? == "moving");
        let journal = fixture
            .source
            .record("core-move.json")?
            .context("journal")?;
        ensure!(journal["phase"]["phase"] == "retiring", "{journal}");
        ensure!(
            fixture.source.state.join("core-state.json").is_file(),
            "the placed copy was taken out"
        );
        ensure!(
            fixture.target_core()?.is_none(),
            "a core runs on the target"
        );
        std::fs::remove_file(&refuse)?;
        back_journal_until(&fixture, "rolled_back")?;
        wait_for("the window a node again", || {
            Ok((fixture.role()? == "node").then_some(()))
        })?;
        ensure!(fixture.target_core()?.is_some(), "no core on the target");
        ensure!(
            !fixture.source.state.join("core-state.json").exists(),
            "the copy is left here"
        );
        wait_for("the projects through the resumed core", || {
            Ok((fixture.projects()?.len() == 2).then_some(()))
        })?;
        Ok(())
    })();
    finish(fixture, journey)
}

/// The core's machine answers a move back's release as already retired
/// for it (here its record is made to say so) before any copy reached this
/// machine: the move back starts no core on an empty folder, whose commit
/// would lose everything the core held, and its process ends naming why.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_move_back_told_it_retired_before_any_copy_came_starts_no_core() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        moved_forward(&fixture)?;
        let program = std::path::PathBuf::from(placed_program(&fixture)?);
        let record = fixture.target.state.join("core-handover.json");
        fixture.stand_in_peer(
            &program,
            &format!(
                r#"if [ "$1 $2" = "core-move release" ]; then
  previous=
  for arg in "$@"; do
    [ "$previous" = --intent ] && intent=$arg
    previous=$arg
  done
  # Private, as the record's own writer leaves it.
  (umask 077; printf '{{"version":1,"intent":"%s","source":"{source}","target":"{target}","state":{{"state":"retired"}}}}' "$intent" > '{record}.planted')
  mv '{record}.planted' '{record}'
fi"#,
                source = TARGET_NODE,
                target = SOURCE_NODE,
                record = record.display()
            ),
        )?;
        fixture.event("core_move", json!({"action": "back"}))?;
        let ended = fixture.source_ended_within(std::time::Duration::from_secs(60))?;
        ensure!(
            !ended.success(),
            "the source ended as if the move back were done"
        );
        let output = std::fs::read_to_string(fixture.root.join("source-hided.log"))?;
        ensure!(
            output.contains("committed on the core's machine with no copy placed here"),
            "{output}"
        );
        ensure!(
            !records(&fixture, "move.committed")?
                .iter()
                .any(|record| record["direction"] == "back"),
            "the move back committed"
        );
        ensure!(
            !fixture.source.state.join("core-state.json").exists(),
            "a core started on an empty folder"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// A move back whose check fails changes nothing: the window stays a node
/// of the core, which keeps running.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_failing_move_back_check_changes_nothing() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        moved_forward(&fixture)?;
        let core = fixture.target_core()?.context("the target's core")?;
        // This machine holds brain state of its own again, and the agent
        // the core's Hide AI asks is signed out here.
        std::fs::write(fixture.source.state.join("labels.json"), b"{}")?;
        fixture.sign_out_here()?;
        let target_before = listing(&fixture.target.state)?;
        fixture.event("core_move", json!({"action": "back"}))?;
        let failed = wait_for("the move back's checks", || {
            Ok(records(&fixture, "checks.failed")?
                .into_iter()
                .find(|record| record["direction"] == "back"))
        })?;
        ensure!(failed["checks"] == json!(["own_state", "ai"]), "{failed}");
        ensure!(failed["checked"] == 8, "{failed}");
        ensure!(fixture.role()? == "node");
        ensure!(
            fixture.target_core()? == Some(core),
            "the target's core changed"
        );
        let target_after = listing(&fixture.target.state)?;
        ensure!(
            target_after == target_before,
            "the target changed from {target_before:?} to {target_after:?}"
        );
        let journal = fixture
            .source
            .record("core-move.json")?
            .context("journal")?;
        ensure!(journal["phase"]["direction"] == "forward", "{journal}");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The copy cannot be pulled after the core's machine stopped its core:
/// that core starts again on its untouched folder and the window is its
/// node again.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_failed_pull_restarts_the_source_unchanged() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        moved_forward(&fixture)?;
        let before = fixture.projects()?;
        let core = fixture.target_core()?.context("the target's core")?;
        let state_before = std::fs::read(fixture.target.state.join("core-state.json"))?;
        // This machine cannot make its staging folder.
        let staging = fixture.source.state.join("move-staging");
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        std::fs::write(&staging, b"")?;
        fixture.event("core_move", json!({"action": "back"}))?;
        let journal = back_journal_until(&fixture, "rolled_back")?;
        ensure!(journal["phase"]["failed"] == "copy", "{journal}");
        // The journal says rolled back before the node role starts (#962).
        wait_for("the window a node again", || {
            Ok((fixture.role()? == "node").then_some(()))
        })?;
        // Nothing of the move is left to remove: the staging folder it
        // could not make holds nothing.
        ensure!(
            records(&fixture, "rollback.unclean")?.is_empty(),
            "{}",
            fixture.log_tail(&fixture.source)
        );
        let restarted = fixture.target_core()?.context("no core on the target")?;
        ensure!(restarted != core, "the target's core never stopped");
        ensure!(fixture.target.record("core-handover.json")?.is_none());
        ensure!(
            !fixture
                .target
                .state
                .join("move-staging")
                .join(journal["intent"].as_str().context("intent")?)
                .exists()
        );
        ensure!(fixture.source.state.join("core-placement.json").is_file());
        // The core started on the folder it stopped with.
        let state_after = std::fs::read(fixture.target.state.join("core-state.json"))?;
        let parsed = |bytes: &[u8]| -> Result<Value> {
            let mut value: Value = serde_json::from_slice(bytes)?;
            value["pane_terminal_sizes"] = Value::Null;
            Ok(value["workspace_registrations"].take())
        };
        ensure!(
            parsed(&state_after)? == parsed(&state_before)?,
            "the target's projects changed"
        );
        let after = wait_for("the projects through the restarted core", || {
            let projects = fixture.projects()?;
            Ok((projects.len() == 2).then_some(projects))
        })?;
        ensure!(after == before, "{after:?}");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The core's machine releases a copy whose manifest names a file outside
/// it, here one in this machine's home by its absolute path: the move back
/// refuses the manifest before it pulls a file, that core starts again,
/// and nothing is written outside this machine's staging folder.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_copy_whose_manifest_names_a_file_outside_it_is_refused() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        moved_forward(&fixture)?;
        let before = fixture.projects()?;
        let planted = fixture.source.home().join("planted-by-the-core");
        let planted_path = planted.to_str().context("path")?;
        let hostile = fixture.root.join("hostile.manifest.json");
        std::fs::write(
            &hostile,
            serde_json::to_vec(&json!({"files": {planted_path: {"size": 7, "sha256": "00"}}}))?,
        )?;
        // The manifest the release wrote is replaced, and the file it names
        // is there to be pulled.
        fixture.stand_in_peer(
            std::path::Path::new(&placed_program(&fixture)?),
            &format!(
                r#"if [ "$1 $2" = "core-move release" ]; then
  "$0.real" "$@" > "$0.out"
  status=$?
  for manifest in '{staging}'/*.manifest.json; do
    copy="${{manifest%.manifest.json}}"
    cp '{hostile}' "$manifest"
    mkdir -p "$copy{parent}"
    printf planted > "$copy{planted_path}"
  done
  cat "$0.out"
  exit $status
fi"#,
                staging = fixture.target.state.join("move-staging").display(),
                hostile = hostile.display(),
                parent = planted.parent().context("parent")?.display(),
            ),
        )?;
        fixture.event("core_move", json!({"action": "back"}))?;
        let journal = back_journal_until(&fixture, "rolled_back")?;
        let cause = &journal["phase"]["cause"];
        ensure!(
            journal["phase"]["failed"] == "copy"
                && cause["kind"] == "refused"
                && cause["step"] == "release"
                && cause["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains(planted_path)),
            "{journal}"
        );
        ensure!(
            !planted.exists(),
            "the core's machine wrote outside the copy"
        );
        ensure!(fixture.role()? == "node");
        let after = wait_for("the projects through the restarted core", || {
            let projects = fixture.projects()?;
            Ok((projects.len() == 2).then_some(projects))
        })?;
        ensure!(after == before, "{after:?}");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The core's machine cannot be reached after it stopped its core: this
/// machine starts no core of its own, waits, and when that machine answers
/// again its core starts on its folder and the window is its node.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_source_unreachable_after_its_stop_is_waited_for() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        moved_forward(&fixture)?;
        let released = fixture.target_core()?.context("the target's core")?;
        fixture.event("core_move", json!({"action": "back"}))?;
        wait_for("the target's core released", || {
            let bytes =
                std::fs::read(fixture.target.state.join("Logs/core.jsonl")).unwrap_or_default();
            Ok(String::from_utf8_lossy(&bytes)
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .any(|record| record["kind"] == "release.checked" && record["refused"].is_null())
                .then_some(()))
        })?;
        fixture.ssh.online(false)?;
        fixture.kill_source()?;
        let phase = fixture
            .source
            .record("core-move.json")?
            .context("journal")?["phase"]["phase"]
            .clone();
        ensure!(
            matches!(
                phase.as_str(),
                Some("releasing" | "released" | "placed_here")
            ),
            "the move back got past its commit before the cut: {phase}"
        );
        // The released core ends on its own, whatever reaches its machine.
        wait_for("the released core ended", || {
            Ok((!hide_platform::process::is_alive(released)).then_some(()))
        })?;
        fixture.start_source()?;
        fixture.logged(&fixture.source, "resume.failed")?;
        ensure!(fixture.role()? == "moving");
        ensure!(
            fixture.target_core()?.is_none(),
            "the target started a core while cut off"
        );
        ensure!(!fixture.source.state.join("core-state.json").exists());
        fixture.ssh.online(true)?;
        back_journal_until(&fixture, "rolled_back")?;
        wait_for("the window a node again", || {
            Ok((fixture.role()? == "node").then_some(()))
        })?;
        ensure!(fixture.target_core()?.is_some(), "no core on the target");
        wait_for("the projects through the restarted core", || {
            Ok((fixture.projects()?.len() == 2).then_some(()))
        })?;
        Ok(())
    })();
    finish(fixture, journey)
}

/// This machine was killed after the core's machine retired its core,
/// before it recorded the move back done: at its next start it finishes
/// the move and runs the core. The retired machine's hided, started again
/// as its login item would, ends at once and successfully, so keep-alive
/// does not restart it, and starts no core.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_driver_killed_after_the_retirement_finishes_the_move_back() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        let before = visible(&fixture)?;
        moved_forward(&fixture)?;
        let placement = fixture
            .source
            .record("core-placement.json")?
            .context("placement")?;
        fixture.event("core_move", json!({"action": "back"}))?;
        let mut journal = back_journal_until(&fixture, "done")?;
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        fixture.kill_source()?;
        // What a kill between the retirement and its record leaves: the
        // journal at retiring and the placement still naming the core's
        // machine.
        journal["phase"] = json!({"direction": "back", "phase": "retiring"});
        write_record(&fixture.source.state.join("core-move.json"), &journal)?;
        write_record(
            &fixture.source.state.join("core-placement.json"),
            &placement,
        )?;
        fixture.start_source()?;
        back_journal_until(&fixture, "done")?;
        fixture.logged(&fixture.source, "move.resumed")?;
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        ensure!(!fixture.source.state.join("core-placement.json").exists());
        ensure!(fixture.target_core()?.is_none());

        let ended = fixture.start_target_hided()?;
        ensure!(ended.success(), "the retired machine's hided: {ended}");
        fixture.logged(&fixture.target, "start.held")?;
        ensure!(fixture.target_core()?.is_none(), "a retired core started");
        ensure!(!fixture.target.state.join("core-state.json").exists());

        fixture.device_ready()?;
        let after = wait_for("the projects of both machines again", || {
            let now = visible(&fixture)?;
            Ok((now["projects"].as_array().map(Vec::len) == Some(2)).then_some(now))
        })?;
        ensure!(after == before, "before: {before}\nafter: {after}");
        Ok(())
    })();
    finish(fixture, journey)
}

/// The names in `dir`, without its log folder or a running core's write in
/// flight (the `.<file>.hide-<pid>-<n>` an atomic replacement renames over
/// its file).
fn listing(dir: &std::path::Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .map(|entry| Ok(entry?.file_name().to_string_lossy().into_owned()))
        .collect::<Result<_>>()?;
    // What the machine keeps of its own work, written by its node's ordinary
    // reads and never moved with a core: its log and its worktree facts.
    let worktree_facts = hide_kit::layout::worktree_facts(dir);
    let worktree_facts = worktree_facts
        .file_name()
        .map(|name| name.to_string_lossy().into_owned());
    names.retain(|name| {
        name != "Logs"
            && Some(name) != worktree_facts.as_ref()
            && !(name.starts_with('.') && name.contains(".hide-"))
    });
    names.sort();
    Ok(names)
}

/// Every file named `name` below `dir`, links not followed.
fn files_named(dir: &std::path::Path, name: &str) -> Result<Vec<std::path::PathBuf>> {
    let mut found = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                folders.push(entry.path());
            } else if entry.file_name() == name {
                found.push(entry.path());
            }
        }
    }
    Ok(found)
}

/// A pending lease's deadline that no journey outlasts.
fn lease_ahead() -> u64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the clock is past 1970");
    (now + std::time::Duration::from_secs(3600)).as_millis() as u64
}

fn records(fixture: &Fixture, kind: &str) -> Result<Vec<Value>> {
    let bytes = std::fs::read(fixture.source.state.join("Logs/core.jsonl"))?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| record["kind"] == kind)
        .collect())
}

/// Amendment 3: a core started on a move's copy acts on nothing outside its
/// machine until the move's own link commits it, and then does at once.
/// This watches what such a core reaches from this process: the device its
/// copy registers (a listener its SSH config names) and Mobile's
/// `tailscale`. The bell, watch warnings, human notices, provider calls,
/// GitHub reads and the Factory are held inside the core, each tested there.
#[test]
fn a_pending_core_does_nothing_outside_until_its_link_commits() -> Result<()> {
    const INTENT: &str = "move-fixture";
    let dir = tempfile::tempdir()?;
    let home = hide_platform::fs::identity::canonical(dir.path())?;
    let state = home.join("state");
    hide_platform::fs::private::create_dir_all(&state)?;
    write_record(
        &state.join("core-handover.json"),
        &json!({"version": 1, "intent": INTENT, "source": SOURCE_NODE, "target": TARGET_NODE, "state": {"state": "pending", "lease_until_unix_ms": lease_ahead()}}),
    )?;
    write_record(&state.join("mobile.json"), &json!({"enabled": true}))?;
    let tailscale = FakeTailscale::new(&home);
    tailscale.ready();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let running = runtime
        .block_on(hided::start_daemon(hided::env::Env {
            home: home.clone(),
            herdr_socket_path: None,
            herdr_bin_path: None,
            login_shell: None,
            state_dir: state.clone(),
            legacy_state_dir: None,
            account_state_folder: false,
            keep_alive: true,
            vite_origin: None,
            bind: "127.0.0.1:0".parse()?,
            idle_secs: 600,
            build: None,
            starter_program: None,
            open_command: None,
            host_helper_root: None,
            host_cli_dir: None,
            pane_id: None,
            tailscale_bin: Some(tailscale.bin.clone()),
            search_path: None,
        }))
        .map_err(anyhow::Error::msg)?;
    // Bound after the daemon, so it closes first: a redial waiting in its
    // backlog for an SSH greeting would hold the core's stop until the
    // dial's own timeout.
    let device = std::net::TcpListener::bind("127.0.0.1:0")?;
    device.set_nonblocking(true)?;
    std::fs::create_dir_all(home.join(".ssh"))?;
    std::fs::write(
        home.join(".ssh/config"),
        format!(
            "Host probe\n  HostName 127.0.0.1\n  Port {}\n  User fixture\n  IdentityAgent none\n",
            device.local_addr()?.port()
        ),
    )?;
    let snapshot = || -> Result<Value> {
        Ok(Renderer::connect(running.port, &running.token)?
            .snapshot()
            .clone())
    };
    let attempted = |snapshot: &Value| {
        snapshot
            .pointer("/status/remote")
            .and_then(Value::as_array)
            .is_some_and(|rows| rows.iter().any(|row| row["target_id"] == "probe"))
    };

    Renderer::connect(running.port, &running.token)?.event(
        "register_device",
        json!({"id": "probe", "label": "Probe", "ssh_alias": "probe"}),
    )?;
    let pending = snapshot()?;
    ensure!(
        pending["ui_state"]["device_registrations"][0]["id"] == "probe",
        "the registration is kept: {}",
        pending["ui_state"]["device_registrations"]
    );
    ensure!(!attempted(&pending), "a pending core dialed the device");
    // A move of its own, or its checks, would dial the device: the window
    // is refused before anything is asked.
    for action in ["check", "start"] {
        let refused = Renderer::connect(running.port, &running.token)?
            .event("core_move", json!({"action": action, "device": "probe"}))
            .expect_err("a pending core took a move");
        ensure!(refused.to_string().contains("core_pending"), "{refused}");
    }
    runtime.block_on(running.mobile.reconcile());
    ensure!(
        tailscale.calls().is_empty(),
        "a pending core ran tailscale: {}",
        tailscale.calls()
    );

    running
        .move_gate
        .admit(SOURCE_NODE, Some(INTENT))
        .map_err(anyhow::Error::msg)?;
    wait_for("the device dialed after the commit", || {
        match device.accept() {
            Ok(_) => Ok(Some(())),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
            Err(error) => Err(error.into()),
        }
    })?;
    ensure!(
        attempted(&snapshot()?),
        "no connection row after the commit"
    );
    wait_for("Mobile's first check after the commit", || {
        Ok(tailscale.calls().contains("status").then_some(()))
    })?;
    running.stop();
    Ok(())
}

/// Writes a record as the product does: owner-only, replaced whole.
fn write_record(path: &std::path::Path, value: &Value) -> Result<()> {
    hide_platform::fs::atomic::write_file(
        path,
        &serde_json::to_vec_pretty(value)?,
        hide_platform::fs::Access::Private,
    )?;
    Ok(())
}

/// D-29: a project of the machine the core leaves keeps its Local issues and
/// its checkout's link through the move and back. The new core shows the
/// same tasks by key with the same link, an issue made there continues the
/// project's numbers, and the core that comes back shows that issue too.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn a_moved_machine_s_project_keeps_its_issues_through_the_move_and_back() -> Result<()> {
    let fixture = Fixture::start()?;
    let journey = (|| {
        // A window open on the core, as the operator's is: a core reads a
        // project's GitHub only while a window is attached to it.
        let window = fixture.window();
        let _open = Renderer::window(window.0, &window.1)?;
        // A Git project on the source whose checkout's branch names L-1.
        let app = fixture.source.home().join("app");
        std::fs::create_dir(&app)?;
        for args in [
            &["init", "-q", "-b", "1-first"][..],
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "init",
            ],
        ] {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&app)
                .args(args)
                .env("HOME", fixture.source.home())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()?;
            ensure!(status.success(), "git {args:?} failed: {status}");
        }
        let app = app.display().to_string();
        fixture.event(
            "create_workspace",
            json!({"path": app, "label": "App", "initialize_git": false}),
        )?;
        let app_id = wait_for("the app project with a Local source", || {
            let snapshot = fixture.snapshot()?;
            Ok(app_project(&snapshot, None, &app)
                .filter(|project| project.pointer("/tasks/source/kind") == Some(&json!("local")))
                .and_then(|project| project["id"].as_str().map(str::to_owned)))
        })
        .with_context(|| {
            let shown = fixture.snapshot().map(|snapshot| {
                app_project(&snapshot, None, &app)
                    .map(|project| json!({"is_git": project["is_git"], "tasks": project["tasks"], "github": project.pointer("/checkouts/0/github")}))
            });
            format!("app: {shown:?}")
        })?;
        fixture.event(
            "backlog_add",
            json!({"intent": "before-move", "workspace_id": app_id, "title": "Made before the move"}),
        )?;
        let before = wait_for("the issue and the checkout it links", || {
            let snapshot = fixture.snapshot()?;
            Ok(app_project(&snapshot, None, &app)
                .map(|project| issues_of(&project, &app))
                .filter(|issues| {
                    issues["tasks"].as_array().map(Vec::len) == Some(1)
                        && issues["linked"].is_string()
                }))
        })?;
        ensure!(before["tasks"][0][1] == "L-1", "{before}");
        ensure!(before["linked"] == before["tasks"][0][0], "{before}");

        fixture.event("core_move", json!({"action": "start", "device": DEVICE_ID}))?;
        fixture.journal_until("done")?;
        let node_app_id = wait_for("the same issues and link through the new core", || {
            let snapshot = fixture.snapshot()?;
            Ok(app_project(&snapshot, Some(SOURCE_NODE), &app)
                .filter(|project| issues_of(project, &app) == before)
                .and_then(|project| project["id"].as_str().map(str::to_owned)))
        })
        .with_context(|| {
            let shown = fixture
                .snapshot()
                .map(|snapshot| app_project(&snapshot, Some(SOURCE_NODE), &app));
            format!("before: {before}\nafter: {shown:?}")
        })?;
        fixture.event(
            "backlog_add",
            json!({"intent": "after-move", "workspace_id": node_app_id, "title": "Made on the new core"}),
        )?;
        let moved = wait_for("the new issue numbered after the old one", || {
            let snapshot = fixture.snapshot()?;
            Ok(app_project(&snapshot, Some(SOURCE_NODE), &app)
                .map(|project| issues_of(&project, &app))
                .filter(|issues| issues["tasks"].as_array().map(Vec::len) == Some(2)))
        })?;
        ensure!(
            moved["tasks"][0][1] == "L-2" && moved["tasks"][0][2] == "Made on the new core",
            "{moved}"
        );

        fixture.event("core_move", json!({"action": "back"}))?;
        back_journal_until(&fixture, "done")?;
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        fixture.device_ready()?;
        let window = fixture.window();
        let _back = Renderer::window(window.0, &window.1)?;
        wait_for("both issues back on this machine's core", || {
            let snapshot = fixture.snapshot()?;
            Ok(app_project(&snapshot, None, &app)
                .filter(|project| issues_of(project, &app) == moved)
                .map(|_| ()))
        })
        .with_context(|| {
            let shown = fixture
                .snapshot()
                .map(|snapshot| app_project(&snapshot, None, &app));
            format!("moved: {moved}\nreturned: {shown:?}")
        })?;
        Ok(())
    })();
    finish(fixture, journey)
}

/// The project at `path` as a window shows it: one of the core's own
/// (`node` none), or one in the session of node `node` that dials the core.
fn app_project(snapshot: &Value, node: Option<&str>, path: &str) -> Option<Value> {
    let projects = match node {
        None => snapshot.pointer("/navigator/workspaces")?,
        Some(node) => snapshot
            .pointer("/status/remote")?
            .as_array()?
            .iter()
            .find(|status| status["target_id"] == node)?
            .pointer("/session/workspaces")?,
    };
    projects
        .as_array()?
        .iter()
        .find(|project| project["path"] == path)
        .cloned()
}

/// A project's tasks as (key, id, title, open) and the task its checkout at
/// `path` links, which compare the same on either core: a Local task's key
/// names the project's path and the issue's number.
fn issues_of(project: &Value, path: &str) -> Value {
    let tasks: Vec<Value> = project
        .pointer("/tasks/tasks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|task| json!([task["key"], task["id"], task["title"], task["open"]]))
        .collect();
    let linked = project["checkouts"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|checkout| checkout["path"] == path)
        .map(|checkout| checkout["task_key"].clone())
        .unwrap_or(Value::Null);
    json!({"tasks": tasks, "linked": linked})
}

/// Ends `fixture` by how `journey` went (`support::run::finish`), with each
/// machine's core log on a failure.
fn finish(mut fixture: Fixture, journey: Result<()>) -> Result<()> {
    let journey = journey.map_err(|error| {
        let source = fixture.log_tail(&fixture.source);
        let target = fixture.log_tail(&fixture.target);
        error.context(format!("source log:\n{source}\ntarget log:\n{target}"))
    });
    crate::support::run::finish(&mut fixture, journey)
}

/// D-15(3) and D-29: what the operator arranged on each machine is the same
/// before the move, once it commits and after the move back, compared by the
/// machine and path each id names, since the two cores name the machines
/// differently. The state is written by the product's own events where one
/// exists; labels and a letter held for the target are placed at rest,
/// because their writers need an AI provider and a registered agent. The
/// target's core keeps the labels it was given in memory, so its folder has
/// none to compare.
#[test]
#[ignore = "external lane requires this worktree's CLI binaries and HIDE_E2E_HERDR_BIN"]
fn what_each_machine_shows_survives_the_move_and_the_move_back() -> Result<()> {
    let mut fixture = Fixture::start()?;
    let journey = (|| {
        seed_at_rest(&mut fixture)?;
        seed_through_events(&fixture)?;
        let shown_before = settled_window(&fixture, SOURCE_NODE)?;

        let forward = moved_forward(&fixture)?;
        let kept_before = kept_per_machine(
            &fixture
                .source
                .state
                .join("moved-out")
                .join(forward["intent"].as_str().context("intent")?),
            SOURCE_NODE,
        )?;
        ensure!(
            kept_before["source"]["workspace_views"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|view| view["layout"]["root"]["area"]["displays"]
                    .as_array()
                    .is_some_and(|displays| !displays.is_empty())
                    && view["bookmarks"]
                        .as_object()
                        .is_some_and(|bookmarks| !bookmarks.is_empty())),
            "the move fixture must keep one View and a tab bookmark to it: {kept_before:#}"
        );
        let shown_after = wait_for("the window as it was, through the new core", || {
            let shown = window_shows(&fixture, TARGET_NODE)?;
            Ok(shown.filter(|shown| *shown == shown_before))
        })
        .with_context(|| {
            format!(
                "before: {shown_before}\nafter: {:?}",
                window_shows(&fixture, TARGET_NODE)
            )
        })?;

        fixture.event("core_move", json!({"action": "back"}))?;
        let back = back_journal_until(&fixture, "done")?;
        wait_for("this machine's core", || {
            Ok((fixture.role()? == "core").then_some(()))
        })?;
        fixture.device_ready()?;
        let kept_after = kept_per_machine(
            &fixture
                .target
                .state
                .join("moved-out")
                .join(back["intent"].as_str().context("intent")?),
            TARGET_NODE,
        )?;
        wait_for("the window as it was, back on this machine", || {
            Ok((window_shows(&fixture, SOURCE_NODE)?.as_ref() == Some(&shown_after)).then_some(()))
        })
        .with_context(|| {
            format!(
                "before: {shown_after}\nreturned: {:?}",
                window_shows(&fixture, SOURCE_NODE)
            )
        })?;
        let kept_returned = kept_per_machine(&fixture.source.state, SOURCE_NODE)?;

        ensure!(
            kept_returned == kept_before,
            "before: {kept_before:#}\nreturned: {kept_returned:#}"
        );
        let mut without_labels = kept_before.clone();
        without_labels["source"]["labels"] = json!({});
        ensure!(
            kept_after == without_labels,
            "before: {kept_before:#}\nafter: {kept_after:#}"
        );
        ensure!(
            kept_before["source"]["labels"]["w1:p1"]["goal"] == "Seeded goal",
            "{kept_before:#}"
        );
        ensure!(
            kept_before["target"]["letters"].as_array().map(Vec::len) == Some(1),
            "{kept_before:#}"
        );
        Ok(())
    })();
    finish(fixture, journey)
}

/// A label on the source's pane and a letter held for an agent on the
/// target, in the stores' own shapes, before the source's core starts.
fn seed_at_rest(fixture: &mut Fixture) -> Result<()> {
    fixture.kill_source()?;
    write_record(
        &fixture.source.state.join("labels.json"),
        &json!({"version": 1, "targets": {SOURCE_NODE: {"w1:p1": {"goal": "Seeded goal", "line": "Seeded line"}}}}),
    )?;
    let actor = |pane: &str, device: &str| json!({"pane_id": pane, "name": "lead", "kind": "claude", "device_id": device, "session": "lead-session"});
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;
    let path = fixture.source.state.join("delivery-ledger.json");
    let mut ledger: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    ledger["next_id"] = json!(100);
    ledger["letters"] = json!([{"id": "letter-1", "intent": "held-for-target", "sender": actor("w1:p1", SOURCE_NODE), "recipient": actor(&format!("remote:{DEVICE_ID}:pane:w9:p9"), DEVICE_ID), "kind": "report", "body": "A letter held for the target", "state": "undelivered", "waiting_answer": false, "reply_to": null, "created_at_unix_ms": now, "finished_at_unix_ms": now, "bell_errors": 0, "bell_sent": false, "human_notified": false}]);
    write_record(&path, &ledger)?;
    fixture.start_source()?;
    fixture.device_ready()
}

/// Folds, path settings, Views, a second tab and an Agent split on the
/// source's checkout, each through the event the window sends.
fn seed_through_events(fixture: &Fixture) -> Result<()> {
    let source = fixture.source.project();
    let target = fixture.target.project();
    std::fs::create_dir_all(source.join("src"))?;
    std::fs::write(source.join("src/a.txt"), "a")?;
    std::fs::create_dir_all(target.join("lib"))?;
    let snapshot = fixture.snapshot()?;
    let registrations = snapshot
        .pointer("/ui_state/workspace_registrations")
        .and_then(Value::as_array)
        .context("registrations")?
        .clone();
    let project = |device: &str| {
        registrations
            .iter()
            .find(|row| row["device_id"] == device)
            .map(|row| row["id"].clone())
            .context("project")
    };
    let (source_project, target_project) = (project(SOURCE_NODE)?, project(DEVICE_ID)?);
    let checkout = snapshot
        .pointer("/navigator/workspaces/0/checkouts/0/id")
        .cloned()
        .context("checkout")?;
    fixture.event(
        "create_tab",
        json!({"workspace_id": source_project, "checkout_id": checkout, "label": "second"}),
    )?;
    let second = wait_for("the source's second tab", || {
        let snapshot = fixture.snapshot()?;
        let tabs = snapshot
            .pointer("/navigator/workspaces/0/checkouts/0/tabs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok((tabs.len() == 2).then(|| tabs[1]["id"].clone()))
    })?;
    let bindings = snapshot
        .pointer("/ui_state/shortcut_bindings")
        .cloned()
        .unwrap_or(json!({}));
    for (kind, payload) in [
        (
            "agent_layout",
            json!({"workspace": {"device_id": SOURCE_NODE, "path": source}, "action": "split", "tab_id": second, "area_id": "a1", "edge": "right", "request_id": "seed-split"}),
        ),
        ("session_fold_toggle", json!({"key": source_project})),
        ("session_fold_toggle", json!({"key": target_project})),
        ("checkout_agents_toggle", json!({"checkout_id": checkout})),
        (
            "issue_source_set",
            json!({"project_path": source, "source": "local"}),
        ),
        (
            "issue_source_set",
            json!({"project_path": target, "source": "github"}),
        ),
        (
            "project_checkouts_fold",
            json!({"workspace_id": target_project, "expanded": false}),
        ),
        ("workspace_view", json!({"tool": "files", "pinned": true})),
        (
            "browser_open",
            json!({"url": "https://example.test/seed", "workspace": {"device_id": SOURCE_NODE, "path": source}}),
        ),
        (
            "ui_state_update",
            json!({"expanded_paths": [source.join("src")], "device_expanded_paths": {DEVICE_ID: [target.join("lib")]}, "selected_path": source.join("src/a.txt"), "shortcut_bindings": bindings}),
        ),
    ] {
        fixture.event(kind, payload)?;
    }
    Ok(())
}

/// What the window shows once the seeded split has reached it and the
/// saves behind it had time to land.
fn settled_window(fixture: &Fixture, own: &str) -> Result<Value> {
    let shown = wait_for("the split drawn", || {
        Ok(window_shows(fixture, own)?.filter(|shown| {
            shown
                .pointer("/agent_areas/layout/root/split")
                .is_some_and(|split| !split.is_null())
        }))
    })?;
    std::thread::park_timeout(std::time::Duration::from_secs(2));
    Ok(shown)
}

/// The machine a device id or node id names in either core.
fn machine_of(device: &str, own: &str) -> Result<&'static str> {
    let device = if device == "local" { own } else { device };
    Ok(match device {
        SOURCE_NODE => "source",
        TARGET_NODE | DEVICE_ID => "target",
        other => bail!("an id names no machine of the fixture: {other}"),
    })
}

/// `value` with every `remote:<id>:tab:` or `remote:<id>:pane:` prefix that
/// names `machine` taken off, as that machine's own Herdr spells its ids.
fn unqualify(value: &Value, machine: &str, own: &str) -> Value {
    match value {
        Value::String(text) => {
            for kind in [":tab:", ":pane:"] {
                if let Some((owner, id)) = text.split_once(kind)
                    && let Some(device) = owner.strip_prefix("remote:")
                    && machine_of(device, own).ok() == Some(machine)
                {
                    return json!(id);
                }
            }
            value.clone()
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| unqualify(item, machine, own))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| {
                    let key = match unqualify(&json!(key), machine, own) {
                        Value::String(key) => key,
                        _ => key.clone(),
                    };
                    (key, unqualify(item, machine, own))
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

/// What the window shows, read from the core whose node is `own`: the
/// machine in front with the checkout, tab and pane it shows, every
/// machine's checkouts with their tabs, and the front Workspace's Agent
/// areas, each id as its machine's Herdr spells it; `None` while a core
/// that just started names no machine in front yet.
fn window_shows(fixture: &Fixture, own: &str) -> Result<Option<Value>> {
    let snapshot = fixture.snapshot()?;
    let Some(front) = snapshot
        .pointer("/navigator/focused_device_id")
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let front = machine_of(front, own)?;
    let tabs_of = |checkouts: &[Value]| -> serde_json::Map<String, Value> {
        checkouts
            .iter()
            .filter_map(|checkout| {
                let tabs: Vec<Value> = checkout["tabs"]
                    .as_array()?
                    .iter()
                    .map(|tab| json!({"id": tab["id"], "panes": tab["panes"].as_array().map(|panes| panes.iter().map(|pane| pane["id"].clone()).collect::<Vec<_>>())}))
                    .collect();
                Some((checkout["path"].as_str()?.to_owned(), json!(tabs)))
            })
            .collect()
    };
    let checkouts_in = |workspaces: Option<&Value>| -> Vec<Value> {
        workspaces
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|workspace| {
                workspace["checkouts"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .collect()
    };
    let mut machines = serde_json::Map::new();
    let mut shown = Value::Null;
    let own_checkouts = checkouts_in(snapshot.pointer("/navigator/workspaces"));
    let own_machine = machine_of(own, own)?;
    machines.insert(own_machine.to_owned(), json!(tabs_of(&own_checkouts)));
    if front == own_machine {
        let focused = snapshot.pointer("/navigator/focused_checkout_id");
        let checkout = own_checkouts.iter().find(|row| Some(&row["id"]) == focused);
        shown = json!({
            "checkout": checkout.map(|row| row["path"].clone()),
            "tab": checkout.map(|row| row["active_tab_id"].clone()),
            "pane": snapshot.pointer("/ui_state/selected_pane_id"),
        });
    }
    for status in snapshot
        .pointer("/status/remote")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let machine = machine_of(status["target_id"].as_str().context("target")?, own)?;
        let session = &status["session"];
        let checkouts = checkouts_in(session.get("workspaces"));
        machines.insert(
            machine.to_owned(),
            unqualify(&json!(tabs_of(&checkouts)), machine, own),
        );
        if front == machine {
            let focused = session.get("focused_checkout_id");
            shown = unqualify(
                &json!({
                    "checkout": checkouts.iter().find(|row| Some(&row["id"]) == focused).map(|row| row["path"].clone()),
                    "tab": session["focused_tab_id"],
                    "pane": session["focused_pane_id"],
                }),
                machine,
                own,
            );
        }
    }
    let view = snapshot
        .pointer("/workspace_view")
        .cloned()
        .unwrap_or(Value::Null);
    let agent_areas = match view["device_id"].as_str() {
        Some(device) => {
            let machine = machine_of(device, own)?;
            unqualify(
                &json!({"machine": machine, "path": view["path"], "layout": {"root": view["agent_layout"]["root"], "active_area": view["agent_layout"]["active_area"], "canvases": view["agent_layout"]["canvases"]}}),
                machine,
                own,
            )
        }
        None => Value::Null,
    };
    Ok(Some(
        json!({"front": front, "shown": shown, "machines": machines, "agent_areas": agent_areas}),
    ))
}

/// What the core whose stores are in `dir`, running on node `own`, keeps
/// for each machine: every id replaced by the machine and path it names, so
/// two cores that name the machines differently compare equal when they
/// keep the same things for each. Which checkout and pane a machine shows
/// is its own Herdr's focus and is compared on the window instead.
fn kept_per_machine(dir: &std::path::Path, own: &str) -> Result<Value> {
    let read = |name: &str| -> Result<Value> {
        Ok(serde_json::from_slice(
            &std::fs::read(dir.join(name))
                .with_context(|| format!("{name} in {}", dir.display()))?,
        )?)
    };
    let core = read("core-state.json")?;
    let views = read("workspace-views.json")?;
    let labels = read("labels.json")?;
    let ledger = read("delivery-ledger.json")?;
    let registrations = core["workspace_registrations"]
        .as_array()
        .context("registrations")?;
    let place_project = |id: &str| -> Result<(&'static str, String)> {
        let registration = registrations
            .iter()
            .find(|row| row["id"] == id)
            .with_context(|| format!("{id} names no registered project"))?;
        Ok((
            machine_of(registration["device_id"].as_str().context("device")?, own)?,
            registration["path"].as_str().context("path")?.to_owned(),
        ))
    };
    // A project id as (machine, path); a checkout id as (machine, the
    // checkout's own key), which a remote form spells after `:checkout:`.
    let place = |id: &str| -> Result<(&'static str, String)> {
        if let Some((owner, checkout)) = id.split_once(":checkout:") {
            if let Some(device) = owner.strip_prefix("remote:") {
                return Ok((machine_of(device, own)?, format!("checkout {checkout}")));
            }
            let (machine, _) = place_project(owner)?;
            return Ok((machine, format!("checkout {checkout}")));
        }
        place_project(id)
    };
    let device_of = |machine: &str| -> Result<Option<String>> {
        if machine_of(own, own)? == machine {
            return Ok(None);
        }
        Ok(registrations
            .iter()
            .filter_map(|row| row["device_id"].as_str())
            .find(|device| machine_of(device, own).ok() == Some(machine))
            .map(str::to_owned))
    };
    let mut kept = serde_json::Map::new();
    for machine in ["source", "target"] {
        let device = device_of(machine)?;
        let ids = |field: &str| -> Result<Vec<String>> {
            let mut paths = Vec::new();
            for id in core[field].as_array().into_iter().flatten() {
                let (at, path) = place(id.as_str().context("id")?)?;
                if at == machine {
                    paths.push(path);
                }
            }
            paths.sort();
            Ok(paths)
        };
        let projects: Vec<String> = registrations
            .iter()
            .filter(|row| {
                row["device_id"]
                    .as_str()
                    .and_then(|device| machine_of(device, own).ok())
                    == Some(machine)
            })
            .filter_map(|row| row["path"].as_str().map(str::to_owned))
            .collect();
        let by_path = |field: &str| -> Value {
            let mut found = serde_json::Map::new();
            for path in &projects {
                if let Some(value) = core[field].get(path) {
                    found.insert(path.clone(), value.clone());
                }
            }
            Value::Object(found)
        };
        let own_or_device = |field: &str| -> Value {
            match &device {
                None => core[field].clone(),
                Some(device) => core[format!("device_{field}")][device].clone(),
            }
        };
        // A tab or pane id as this machine's Herdr names it, whichever core
        // qualifies it.
        let unqualified = |value: &Value| -> Value { unqualify(value, machine, own) };
        let workspace_views: Vec<Value> = views["workspaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|row| row["device_id"].as_str().and_then(|device| machine_of(device, own).ok()) == Some(machine))
            .map(|row| unqualified(&json!({"path": row["path"], "tool": row["tool"], "pinned": row["pinned"], "tree": row["tree"], "layout": row["view_layout"], "agent_layout": row["agent_layout"], "bookmarks": row["tab_views"]})))
            .collect();
        let label_records: serde_json::Map<String, Value> = labels["targets"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(node, _)| machine_of(node, own).ok() == Some(machine))
            .flat_map(|(_, panes)| panes.as_object().cloned().unwrap_or_default())
            .map(|(pane, record)| {
                (
                    pane,
                    json!({"goal": record["goal"], "line": record["line"]}),
                )
            })
            .collect();
        let letters: Vec<Value> = ledger["letters"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|letter| letter["recipient"]["device_id"].as_str().and_then(|device| machine_of(device, own).ok()) == Some(machine))
            .map(|letter| json!({"intent": letter["intent"], "pane": unqualified(&letter["recipient"]["pane_id"]), "body": letter["body"]}))
            .collect();
        kept.insert(
            machine.to_owned(),
            json!({
                "session_folds": ids("session_open_folds")?,
                "collapsed_projects": ids("collapsed_workspace_ids")?,
                "collapsed_sessions": ids("session_collapsed_checkout_ids")?,
                "issue_sources": by_path("project_issue_sources"),
                "base_branches": by_path("project_base_branches"),
                "expanded_paths": own_or_device("expanded_paths"),
                "inactive_open": projects.iter().filter(|path| core["expanded_inactive_checkout_project_paths"].as_array().is_some_and(|open| open.contains(&json!(path)))).collect::<Vec<_>>(),
                "workspace_views": workspace_views,
                "labels": label_records,
                "letters": letters,
            }),
        );
    }
    kept.insert("selected_path".to_owned(), core["selected_path"].clone());
    Ok(Value::Object(kept))
}
