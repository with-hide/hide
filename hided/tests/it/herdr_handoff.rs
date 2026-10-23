//! A Herdr server an older Hide app started moves onto this app's Herdr when
//! `hide connect` looks at it (`hided::herdr_handoff`), and the process in
//! its pane goes on running; a server running a Herdr no Hide app holds is
//! left as it is.
//!
//! A device's node keeps its own rule (`hided::herdr_handoff::device_update`,
//! PRD device-herdr-update): the server on the device's registered socket,
//! started from the operator's own Herdr, moves to the pinned copy the build
//! brought only when it runs an older release, a server on another socket is
//! never touched, and a handoff that failed is not tried again for the same
//! server.
//!
//! This machine's server the operator started from their own older Herdr
//! is offered and moved by the core (`hided::herdr_handoff::LocalTakeover`,
//! PRD herdr-takeover): one on the protocol this build reads (0.9.1) and one
//! on an older protocol (0.8.2) are offered and moved with their panes, and
//! a failed move is remembered for that server and tried again on a press.
//!
//! Real processes against the pinned Herdr, so the tests are ignored by
//! default; run them with `--ignored`, the pinned binary in
//! `HIDE_E2E_HERDR_BIN`, an older release on the same protocol (Herdr 0.9.1,
//! which the remote mailbox lane fetches) in `HIDE_E2E_HERDR_OLDER_BIN` and
//! one on an older protocol (Herdr 0.8.2) in `HIDE_E2E_HERDR_OLDER_PROTOCOL_BIN`.
//! Herdr hands a server over on Unix only.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use herdr_core::herdr_takeover::{FailureCause, HerdrTakeover, Look, Moved, OfferKind};
use hide_kit::device_herdr::{BehindCause, HerdrUpdate};
use hide_platform::fs::identity::{canonical, file_id};
use hide_platform::process::{image_of, is_alive};
use hided::herdr_handoff::{LocalTakeover, Outcome, device_update, hand_off};
use serde_json::Value;

use crate::private_herdr::{PrivateHerdr, herdr_command};

#[test]
#[ignore = "needs the pinned Herdr: set HIDE_E2E_HERDR_BIN and run with --ignored"]
fn a_server_an_older_app_started_moves_onto_this_apps_herdr_with_its_panes() {
    let pinned = PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_BIN").expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
    );
    // Herdr makes its handoff socket inside its home, and macOS holds a
    // socket path to 104 bytes, which a folder under $TMPDIR outgrows.
    let root = tempfile::Builder::new()
        .prefix("hho")
        .tempdir_in("/tmp")
        .unwrap();
    let old = root.path().join("Old.app/Contents/Resources");
    let new = root.path().join("New.app/Contents/Resources");
    for resources in [&old, &new] {
        std::fs::create_dir_all(resources).unwrap();
        copy_program(&pinned, &resources.join("herdr"));
    }
    package_runtime(&new);
    let new_herdr = new.join("herdr");
    let mut herdr = PrivateHerdr::start(old.join("herdr"), &root.path().join("server"));

    let created = run(&herdr, &["workspace", "create", "--cwd"], &herdr.home);
    let pane = created["result"]["root_pane"]["pane_id"]
        .as_str()
        .expect("the new workspace names its pane")
        .to_owned();
    let pid_file = root.path().join("pane.pid");
    let command = format!("echo $$ > {}; exec sleep 600", pid_file.display());
    run(&herdr, &["pane", "run", &pane], Path::new(&command));
    let pane_pid = wait_for_pid(&pid_file);

    // A Herdr in a folder no Hide app made is the operator's own.
    assert_eq!(
        hand_off(&new_herdr, &herdr.socket),
        Outcome::NotHides {
            program: canonical(&old.join("herdr")).unwrap()
        }
    );

    // The old app was a Hide app, renamed aside as installing a newer one
    // does.
    package_runtime(&old);
    let aside = root.path().join("Old.app.bak");
    std::fs::rename(root.path().join("Old.app"), &aside).unwrap();
    let outcome = hand_off(&new_herdr, &herdr.socket);
    let Outcome::HandedOff {
        to_pid,
        from_program,
        ..
    } = outcome
    else {
        panic!("the server was not handed off: {outcome:?}");
    };
    // From here the server is this app's Herdr's to stop.
    herdr.bin = new_herdr.clone();
    assert_eq!(
        from_program,
        canonical(&aside.join("Contents/Resources/herdr")).unwrap()
    );
    assert_eq!(image_of(to_pid).unwrap().file, file_id(&new_herdr).unwrap());
    assert!(
        is_alive(pane_pid),
        "the pane's process ended with the old server"
    );
    let panes = run(&herdr, &["pane", "list"], Path::new(""));
    assert!(
        panes["result"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|listed| listed["pane_id"] == pane.as_str()),
        "the pane is gone from the new server: {panes}"
    );
    assert_eq!(hand_off(&new_herdr, &herdr.socket), Outcome::Ours);

    herdr.stop().unwrap();
    wait_until("the handed-off server ends", || !is_alive(to_pid));
}

/// A device home: the operator's Herdr in a folder no Hide app made, and the
/// pinned copy a build brought under the helper root, as the upload places
/// it.
struct DeviceHome {
    root: tempfile::TempDir,
    pinned: PathBuf,
    record: PathBuf,
}

impl DeviceHome {
    fn new(pinned_bin: &Path) -> Self {
        // Herdr makes its handoff socket inside its home, and macOS holds a
        // socket path to 104 bytes, which a folder under $TMPDIR outgrows.
        let root = tempfile::Builder::new()
            .prefix("hhd")
            .tempdir_in("/tmp")
            .unwrap();
        let copy = root.path().join("helper/herdr/0123456789abcdef");
        std::fs::create_dir_all(&copy).unwrap();
        let pinned = copy.join("herdr");
        copy_program(pinned_bin, &pinned);
        let record = root.path().join("helper/herdr/handoff.json");
        Self {
            root,
            pinned,
            record,
        }
    }

    /// The operator's own `herdr`, a copy of `bin` in their command folder.
    fn operator_herdr(&self, name: &str, bin: &Path) -> PathBuf {
        let folder = self.root.path().join(name).join(".local/bin");
        std::fs::create_dir_all(&folder).unwrap();
        let herdr = folder.join("herdr");
        copy_program(bin, &herdr);
        herdr
    }

    fn update(&self, socket: &Path) -> HerdrUpdate {
        device_update(&self.pinned, socket, &self.record)
    }
}

fn pinned_bin() -> PathBuf {
    PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_BIN").expect("HIDE_E2E_HERDR_BIN names the pinned herdr"),
    )
}

fn older_bin() -> PathBuf {
    PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_OLDER_BIN")
            .expect("HIDE_E2E_HERDR_OLDER_BIN names an older stable Herdr"),
    )
}

/// The pid of the server answering on `socket`.
fn server_pid(socket: &Path) -> u32 {
    hide_platform::ipc::LocalStream::connect(socket)
        .unwrap()
        .peer_pid()
        .unwrap()
}

/// A pane on `herdr` running a process that writes its pid and sleeps.
fn pane_process(herdr: &PrivateHerdr, root: &Path, label: &str) -> (String, u32) {
    let created = run(herdr, &["workspace", "create", "--cwd"], &herdr.home);
    let pane = created["result"]["root_pane"]["pane_id"]
        .as_str()
        .expect("the new workspace names its pane")
        .to_owned();
    let pid_file = root.join(format!("{label}.pid"));
    let command = format!("echo $$ > {}; exec sleep 600", pid_file.display());
    run(herdr, &["pane", "run", &pane], Path::new(&command));
    (pane, wait_for_pid(&pid_file))
}

/// B5, B4, B13: a device server already on the pinned release stays as it
/// is, whatever file it runs; no server is no server; and nothing touches
/// the operator's own `herdr` or a server on another socket.
#[test]
#[ignore = "needs the pinned Herdr: set HIDE_E2E_HERDR_BIN and run with --ignored"]
fn a_device_server_on_the_pinned_release_is_left_running_its_own_herdr() {
    let pinned = pinned_bin();
    let device = DeviceHome::new(&pinned);
    let operator = device.operator_herdr("operator", &pinned);
    let before = std::fs::read(&operator).unwrap();
    let herdr = PrivateHerdr::start(operator.clone(), &device.root.path().join("a"));
    let pid = server_pid(&herdr.socket);

    assert_eq!(
        device.update(&herdr.socket),
        HerdrUpdate::Current {
            version: crate::private_herdr::pinned_version()
        }
    );
    assert_eq!(server_pid(&herdr.socket), pid);
    assert_eq!(image_of(pid).unwrap().file, file_id(&operator).unwrap());
    assert_eq!(std::fs::read(&operator).unwrap(), before);
    assert_eq!(
        device.update(&device.root.path().join("none.sock")),
        HerdrUpdate::NoServer {
            version: crate::private_herdr::pinned_version()
        }
    );
}

/// B2, B4, B11, B13: a server the operator started from an older Herdr on
/// the device's registered socket moves to the pinned copy with its pane's
/// process alive and listed; a server on another socket keeps its Herdr and
/// its pane; the operator's `herdr` is not changed.
#[test]
#[ignore = "needs the pinned and an older Herdr: set HIDE_E2E_HERDR_BIN and HIDE_E2E_HERDR_OLDER_BIN and run with --ignored"]
fn an_older_device_server_moves_to_the_pinned_copy_with_its_pane() {
    let pinned = pinned_bin();
    let older = older_bin();
    let device = DeviceHome::new(&pinned);
    let operator = device.operator_herdr("operator", &older);
    let before = std::fs::read(&operator).unwrap();
    let mut registered = PrivateHerdr::start_any(operator.clone(), &device.root.path().join("a"));
    let other_herdr = device.operator_herdr("other", &older);
    let other = PrivateHerdr::start_any(other_herdr.clone(), &device.root.path().join("b"));
    let (pane, pane_pid) = pane_process(&registered, device.root.path(), "registered");
    let (_, other_pane_pid) = pane_process(&other, device.root.path(), "other");
    let other_pid = server_pid(&other.socket);
    let older_version = String::from_utf8(
        std::process::Command::new(&older)
            .arg("--version")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .split_whitespace()
    .last()
    .unwrap()
    .to_owned();

    let outcome = device.update(&registered.socket);
    let HerdrUpdate::HandedOff {
        server_version,
        version,
        to_pid,
        ..
    } = outcome
    else {
        panic!("the device server was not handed over: {outcome:?}");
    };
    assert_eq!(server_version, older_version);
    assert_eq!(version, crate::private_herdr::pinned_version());
    // From here the server is the pinned copy's to stop.
    registered.bin = device.pinned.clone();
    assert_eq!(
        image_of(to_pid).unwrap().file,
        file_id(&device.pinned).unwrap()
    );
    assert!(
        is_alive(pane_pid),
        "the pane's process ended with the old server"
    );
    let panes = run(&registered, &["pane", "list"], Path::new(""));
    assert!(
        panes["result"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|listed| listed["pane_id"] == pane.as_str()),
        "the pane is gone from the new server: {panes}"
    );
    assert!(matches!(
        device.update(&registered.socket),
        HerdrUpdate::Current { .. }
    ));

    assert_eq!(server_pid(&other.socket), other_pid);
    assert_eq!(
        image_of(other_pid).unwrap().file,
        file_id(&other_herdr).unwrap()
    );
    assert!(is_alive(other_pane_pid));
    assert_eq!(std::fs::read(&operator).unwrap(), before);

    registered.stop().unwrap();
    wait_until("the handed-off server ends", || !is_alive(to_pid));
}

/// B8, B10: a handoff the replacement refuses leaves the old server and its
/// pane as they were, answers why, and is not tried again for that server.
#[test]
#[ignore = "needs the pinned and an older Herdr: set HIDE_E2E_HERDR_BIN and HIDE_E2E_HERDR_OLDER_BIN and run with --ignored"]
fn a_refused_device_handoff_leaves_the_server_and_is_not_repeated() {
    let pinned = pinned_bin();
    let device = DeviceHome::new(&pinned);
    // A pinned copy that names the pinned release but cannot take a server.
    std::fs::remove_file(&device.pinned).unwrap();
    std::fs::write(
        &device.pinned,
        format!(
            "#!/bin/sh\nif [ \"$1\" = status ]; then exec '{}' \"$@\"; fi\nexit 1\n",
            pinned.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(
        &device.pinned,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
    )
    .unwrap();
    let operator = device.operator_herdr("operator", &older_bin());
    let herdr = PrivateHerdr::start_any(operator.clone(), &device.root.path().join("a"));
    let (_, pane_pid) = pane_process(&herdr, device.root.path(), "refused");
    let pid = server_pid(&herdr.socket);

    let first = device.update(&herdr.socket);
    assert!(
        matches!(
            &first,
            HerdrUpdate::Behind {
                cause: BehindCause::HandoffFailed,
                ..
            }
        ),
        "{first:?}"
    );
    assert_eq!(server_pid(&herdr.socket), pid);
    assert_eq!(image_of(pid).unwrap().file, file_id(&operator).unwrap());
    assert!(is_alive(pane_pid));

    let again = device.update(&herdr.socket);
    let HerdrUpdate::Behind { cause, reason, .. } = &again else {
        panic!("the second pass did not answer the earlier failure: {again:?}");
    };
    assert_eq!(*cause, BehindCause::HandoffFailed);
    assert!(reason.contains("earlier handoff"), "{reason}");
    assert_eq!(server_pid(&herdr.socket), pid);
}

fn older_protocol_bin() -> PathBuf {
    PathBuf::from(
        std::env::var_os("HIDE_E2E_HERDR_OLDER_PROTOCOL_BIN")
            .expect("HIDE_E2E_HERDR_OLDER_PROTOCOL_BIN names a Herdr on an older protocol"),
    )
}

fn version_of(herdr: &Path) -> String {
    String::from_utf8(
        std::process::Command::new(herdr)
            .arg("--version")
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap()
    .split_whitespace()
    .last()
    .unwrap()
    .to_owned()
}

/// B1, B4, B5, B7, B21: a server the operator started from their own older
/// Herdr, on this build's protocol or an older one, is offered for a move,
/// moves to the app's Herdr with its pane's process alive and listed when
/// asked, and is then nothing to move; the operator's `herdr` is never
/// written, and when it is behind the server the move says which release.
fn operator_server_moves_when_asked(older: PathBuf, same_protocol: bool) {
    let pinned = pinned_bin();
    let device = DeviceHome::new(&pinned);
    let operator = device.operator_herdr("operator", &older);
    let before = std::fs::read(&operator).unwrap();
    let mut herdr = PrivateHerdr::start_any(operator.clone(), &device.root.path().join("a"));
    let (pane, pane_pid) = pane_process(&herdr, device.root.path(), "operator");
    let from_pid = server_pid(&herdr.socket);
    let takeover = LocalTakeover::new(
        device.pinned.clone(),
        herdr.socket.clone(),
        device.root.path().join("state/herdr-handoff.json"),
    )
    .with_operator_herdr(operator.clone());

    let Look::Offer(offer) = takeover.look() else {
        panic!("the operator's older server is not offered");
    };
    assert_eq!(offer.kind, OfferKind::Move);
    assert_eq!(offer.server_version, version_of(&older));
    assert_eq!(offer.version, crate::private_herdr::pinned_version());
    assert_eq!(offer.server_pid, Some(from_pid));
    assert_eq!(
        offer.server_protocol == hide_herdr_client::HERDR_PROTOCOL_REVISION,
        same_protocol
    );
    assert_eq!(offer.failed_before, None);
    assert_eq!(server_pid(&herdr.socket), from_pid, "a look moves nothing");

    let moved = takeover.hand_over();
    let Moved::Done {
        to_pid,
        path_herdr_behind,
        ..
    } = moved
    else {
        panic!("the server was not moved: {moved:?}");
    };
    herdr.bin = device.pinned.clone();
    assert_eq!(
        image_of(to_pid).unwrap().file,
        file_id(&device.pinned).unwrap()
    );
    assert!(is_alive(pane_pid), "the pane's process ended with the move");
    let panes = run(&herdr, &["pane", "list"], Path::new(""));
    assert!(
        panes["result"]["panes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|listed| listed["pane_id"] == pane.as_str()),
        "the pane is gone from the moved server: {panes}"
    );
    // The operator's own herdr speaks the old protocol after a move off an
    // older one, and keeps up after a move off 0.9.x (B5).
    assert_eq!(
        path_herdr_behind,
        (!same_protocol).then(|| version_of(&older))
    );
    assert_eq!(takeover.look(), Look::Nothing);
    assert_eq!(std::fs::read(&operator).unwrap(), before);

    herdr.stop().unwrap();
    wait_until("the moved server ends", || !is_alive(to_pid));
}

#[test]
#[ignore = "needs the pinned and an older Herdr: set HIDE_E2E_HERDR_BIN and HIDE_E2E_HERDR_OLDER_BIN and run with --ignored"]
fn the_operators_server_on_this_protocol_moves_when_asked() {
    operator_server_moves_when_asked(older_bin(), true);
}

#[test]
#[ignore = "needs the pinned Herdr and one on an older protocol: set HIDE_E2E_HERDR_BIN and HIDE_E2E_HERDR_OLDER_PROTOCOL_BIN and run with --ignored"]
fn the_operators_server_on_an_older_protocol_moves_when_asked() {
    operator_server_moves_when_asked(older_protocol_bin(), false);
}

/// B8, D-23: a move the app's Herdr refuses leaves the server and its pane,
/// names the refusal, is remembered for that server by an app started again,
/// and is still tried on the next press.
#[test]
#[ignore = "needs the pinned and an older Herdr: set HIDE_E2E_HERDR_BIN and HIDE_E2E_HERDR_OLDER_BIN and run with --ignored"]
fn a_refused_move_is_remembered_and_tried_again_on_a_press() {
    let pinned = pinned_bin();
    let device = DeviceHome::new(&pinned);
    // The app's Herdr names the pinned release but cannot take a server.
    std::fs::remove_file(&device.pinned).unwrap();
    std::fs::write(
        &device.pinned,
        format!(
            "#!/bin/sh\nif [ \"$1\" = status ]; then exec '{}' \"$@\"; fi\nexit 1\n",
            pinned.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(
        &device.pinned,
        <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
    )
    .unwrap();
    let operator = device.operator_herdr("operator", &older_bin());
    let herdr = PrivateHerdr::start_any(operator.clone(), &device.root.path().join("a"));
    let (_, pane_pid) = pane_process(&herdr, device.root.path(), "refused");
    let pid = server_pid(&herdr.socket);
    let record = device.root.path().join("state/herdr-handoff.json");
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    let app = || LocalTakeover::new(device.pinned.clone(), herdr.socket.clone(), record.clone());

    let first = app().hand_over();
    assert!(
        matches!(
            &first,
            Moved::Failed {
                cause: FailureCause::Refused,
                ..
            }
        ),
        "{first:?}"
    );
    assert_eq!(server_pid(&herdr.socket), pid);
    assert_eq!(image_of(pid).unwrap().file, file_id(&operator).unwrap());
    assert!(is_alive(pane_pid));

    let Look::Offer(offer) = app().look() else {
        panic!("the server is no longer offered after its failed move");
    };
    assert_eq!(offer.failed_before, Some(FailureCause::Refused));
    let again = app().hand_over();
    assert!(
        matches!(
            &again,
            Moved::Failed {
                cause: FailureCause::Refused,
                ..
            }
        ),
        "a press tries again: {again:?}"
    );
    assert_eq!(server_pid(&herdr.socket), pid);
}

/// Runs `herdr <args> [last]` against the private server and reads its JSON
/// answer; an empty `last` adds nothing.
fn run(herdr: &PrivateHerdr, args: &[&str], last: &Path) -> Value {
    let mut command = herdr_command(&herdr.bin, &herdr.home, &herdr.socket, &herdr.config);
    command.args(args);
    if !last.as_os_str().is_empty() {
        command.arg(last);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "herdr {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or(Value::Null)
}

/// What makes a folder a Hide app's runtime: its daemon beside the shell
/// archive (`hide_kit::holds_package_runtime`).
fn package_runtime(resources: &Path) {
    std::fs::write(resources.join("app.asar"), "").unwrap();
    std::fs::write(resources.join("hided"), "").unwrap();
}

/// A copy of `from` at `to` that runs as a program of its own, its bytes
/// written rather than cloned (`docs/TESTING.md`, Writing a Rust test, 8).
fn copy_program(from: &Path, to: &Path) {
    let mut source = std::fs::File::open(from).unwrap();
    let mut target = std::fs::File::create(to).unwrap();
    std::io::copy(&mut source, &mut target).unwrap();
    target
        .set_permissions(source.metadata().unwrap().permissions())
        .unwrap();
}

fn wait_for_pid(file: &Path) -> u32 {
    let mut pid = None;
    wait_until("the pane's shell writes its pid", || {
        pid = std::fs::read_to_string(file)
            .ok()
            .and_then(|text| text.trim().parse().ok());
        pid.is_some()
    });
    pid.unwrap()
}

/// Polls `done` until it holds; the minute only ends a wait that never
/// would.
#[allow(clippy::disallowed_methods)] // a polling helper: it sleeps between observations of a state, bounded by a deadline
fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}
